use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Instant, UNIX_EPOCH};

use crate::errors::{AppError, AppResult};
use crate::models::{
    CurrentCard, CurrentQuotaResponse, ProfileCard, ProfileIndexEntry, ProfilesIndex,
    ProfilesSnapshotResponse,
};

use super::fs_ops::atomic_write_file;
use super::metadata::{derive_refresh_token_status_from_auth_path, load_profile_metadata};
use super::paths::{
    get_backup_root, get_codex_home, get_profiles_index_path, get_switch_lock_path,
    list_profile_dirs, utc_timestamp, DEFAULT_PAGE_SIZE,
};
use super::process_lock::acquire_process_lock;
use super::profiles::{
    build_current_display_title, build_display_title, compare_profile_names,
    compare_profile_names_for_count, compute_subscription_days_left,
    detect_unmanaged_live_account, profile_activated_at_ms, resolve_backup_target,
    resolve_current_profile,
};
use super::session_usage::{load_latest_local_quota_snapshot_since, normalize_quota_summary};

const PROFILES_INDEX_SCHEMA_VERSION: u32 = 7;

/// How long a freshly-rebuilt index lives in the in-process cache
/// before the next `load_profiles_index` call re-reads the disk.
/// Tuned to dedupe the back-to-back IPC pair issued by the front-end
/// `refreshAllData` call (`get_profiles_snapshot` + `get_current_live_quota`
/// fire concurrently via `Promise.all` and each used to do its own
/// reconcile + write of `profiles.json`) without holding stale data
/// across user-visible state changes.
const PROFILES_INDEX_CACHE_TTL_MS: u128 = 250;

#[cfg(test)]
thread_local! {
    static TEST_PROFILE_INDEX_CACHE_ENABLED: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

struct CachedProfilesIndex {
    fetched_at: Instant,
    index: ProfilesIndex,
}

fn profile_index_cache_enabled() -> bool {
    #[cfg(test)]
    {
        TEST_PROFILE_INDEX_CACHE_ENABLED.with(|enabled| enabled.get())
    }
    #[cfg(not(test))]
    {
        true
    }
}

fn cache_slot() -> &'static std::sync::Mutex<std::collections::HashMap<PathBuf, CachedProfilesIndex>>
{
    static SLOT: OnceLock<
        std::sync::Mutex<std::collections::HashMap<PathBuf, CachedProfilesIndex>>,
    > = OnceLock::new();
    SLOT.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn try_load_cached_index(codex_home: &Path) -> Option<ProfilesIndex> {
    // Tests bypass the cache so per-test fs setup is observable on
    // the next `load_profiles_index` call without explicit
    // invalidation. Production has a single `codex_home` and benefits
    // from the dedup; tests run with throw-away temp dirs and do
    // their own state setup between calls.
    if !profile_index_cache_enabled() {
        return None;
    }
    let slot = cache_slot().lock().ok()?;
    let entry = slot.get(codex_home)?;
    if entry.fetched_at.elapsed().as_millis() >= PROFILES_INDEX_CACHE_TTL_MS {
        return None;
    }
    Some(entry.index.clone())
}

/// Invariant for callers mutating per-profile metadata (`auth.json`,
/// `profile.json`, the `.current_profile` marker, or anything that
/// shows up in `ProfileIndexEntry`): the mutation site MUST follow up
/// with a `load_profiles_index` call so the cache repopulates from
/// the post-write disk state. Today every action handler in
/// `shared/commands/actions.rs`, both refresh paths in
/// `mac/win/runtime/refresh_runtime.rs`, the bulk refresh in
/// `dashboard.rs`, and `login_runtime` / `switch_core` all comply —
/// breaking this invariant would silently leak stale data through
/// the 250 ms cache window for the next concurrent IPC pair.
fn store_cached_index(codex_home: &Path, index: &ProfilesIndex) {
    if !profile_index_cache_enabled() {
        return;
    }
    let Ok(mut slot) = cache_slot().lock() else {
        return;
    };
    slot.insert(
        codex_home.to_path_buf(),
        CachedProfilesIndex {
            fetched_at: Instant::now(),
            index: index.clone(),
        },
    );
}

pub fn invalidate_profiles_index_cache(codex_home: Option<&Path>) {
    if !profile_index_cache_enabled() {
        return;
    }
    let codex_home = codex_home.map(PathBuf::from).unwrap_or_else(get_codex_home);
    let Ok(mut slot) = cache_slot().lock() else {
        return;
    };
    slot.remove(&codex_home);
}

pub fn reload_profiles_index_after_mutation(codex_home: Option<&Path>) -> AppResult<ProfilesIndex> {
    let codex_home = codex_home.map(PathBuf::from).unwrap_or_else(get_codex_home);
    invalidate_profiles_index_cache(Some(&codex_home));
    load_profiles_index(Some(&codex_home))
}

/// Compact all card directories into spreadsheet-column order. The operation
/// is transactional so a failed target rename or metadata write leaves both
/// the directories and `.current_profile` untouched.
pub fn compact_profile_folders(codex_home: &Path) -> AppResult<bool> {
    let backup_root = get_backup_root(Some(codex_home));
    let lock_path = get_switch_lock_path(Some(codex_home));
    if !backup_root.is_dir() {
        return Ok(false);
    }
    if lock_path.is_file() {
        let owner = fs::read_to_string(&lock_path).unwrap_or_default();
        match owner.trim() {
            "SWITCH_IN_PROGRESS" | "LOGIN_BUSY" | "REFRESH_BUSY" | "RESTART_BUSY" => {
                return Ok(false);
            }
            "PROFILE_REPACK_BUSY" => {
                return Err(AppError::new(
                    "PROFILE_REPACK_BUSY",
                    "Profile folder compaction is already in progress.",
                ));
            }
            _ => {
                return Err(AppError::new(
                    "PROFILE_LOCK_UNKNOWN",
                    "A profile lifecycle lock has no recognized owner; refusing to read profile state.",
                ));
            }
        }
    }
    let _lock = match acquire_process_lock(
        Some(codex_home),
        "PROFILE_REPACK_BUSY",
        "Profile folder compaction is already in progress.",
    ) {
        Ok(lock) => lock,
        Err(error) => {
            // Any acquisition failure, including an unexpected lock path
            // shape or filesystem error, must fail closed. Returning false
            // here would let the caller read a potentially half-renamed tree.
            return Err(error);
        }
    };

    let mut dirs = list_profile_dirs(&backup_root);
    let card_count = dirs.len();
    dirs.sort_by(|left, right| {
        let left = left
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let right = right
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        compare_profile_names_for_count(left, right, card_count)
    });
    let names = dirs
        .iter()
        .filter_map(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string)
        })
        .collect::<Vec<_>>();
    if names.is_empty() {
        return Ok(false);
    }
    let targets = (0..names.len())
        .map(super::profiles::sequence_name)
        .collect::<Vec<_>>();
    if names == targets {
        return Ok(false);
    }
    for target in &targets {
        let path = backup_root.join(target);
        if path.exists() && !names.iter().any(|name| name == target) {
            return Err(AppError::new(
                "PROFILE_COMPACT_TARGET_OCCUPIED",
                format!("Profile compaction target is occupied: {}", path.display()),
            ));
        }
    }

    let marker_path = backup_root.join(super::paths::CURRENT_PROFILE_FILENAME);
    let marker_before = fs::read(&marker_path).ok();
    let metadata_before = dirs
        .iter()
        .map(|dir| {
            (
                dir.join(super::paths::PROFILE_METADATA_FILENAME),
                fs::read(dir.join(super::paths::PROFILE_METADATA_FILENAME)).ok(),
            )
        })
        .collect::<Vec<_>>();
    let mut staged: Vec<(PathBuf, PathBuf, String)> = Vec::new();
    for (index, dir) in dirs.iter().enumerate() {
        let temporary = backup_root.join(format!(".compact-{}-{}", std::process::id(), index));
        if let Err(error) = fs::rename(dir, &temporary) {
            for (old, staged_path, _) in staged.iter().rev() {
                if staged_path.exists() {
                    let _ = fs::rename(staged_path, old);
                }
            }
            return Err(AppError::new(
                "PROFILE_COMPACT_FAILED",
                format!("Failed to stage {}: {error}", dir.display()),
            ));
        }
        staged.push((dir.clone(), temporary, targets[index].clone()));
    }
    let result = (|| {
        for (old, temporary, target) in &staged {
            let destination = backup_root.join(target);
            fs::rename(temporary, &destination).map_err(|error| {
                AppError::new(
                    "PROFILE_COMPACT_FAILED",
                    format!(
                        "Failed to move {} -> {}: {error}",
                        old.display(),
                        destination.display()
                    ),
                )
            })?;
            let metadata_path = destination.join(super::paths::PROFILE_METADATA_FILENAME);
            if metadata_path.is_file() {
                let raw = fs::read_to_string(&metadata_path).map_err(|error| {
                    AppError::new(
                        "PROFILE_COMPACT_FAILED",
                        format!("Failed to read {}: {error}", metadata_path.display()),
                    )
                })?;
                let mut value: serde_json::Value = serde_json::from_str(&raw).map_err(|error| {
                    AppError::new(
                        "PROFILE_COMPACT_FAILED",
                        format!(
                            "Invalid profile metadata {}: {error}",
                            metadata_path.display()
                        ),
                    )
                })?;
                let Some(object) = value.as_object_mut() else {
                    return Err(AppError::new(
                        "PROFILE_COMPACT_FAILED",
                        format!(
                            "Profile metadata must be a JSON object: {}",
                            metadata_path.display()
                        ),
                    ));
                };
                object.insert(
                    "folder_name".to_string(),
                    serde_json::Value::String(target.clone()),
                );
                let serialized = serde_json::to_string_pretty(&value)
                    .map_err(|error| AppError::new("PROFILE_COMPACT_FAILED", error.to_string()))?;
                atomic_write_file(&metadata_path, format!("{serialized}\n"))?;
            }
        }
        if let Some(raw) = marker_before.as_ref() {
            let current = String::from_utf8_lossy(raw).trim().to_string();
            if let Some(index) = names.iter().position(|name| name == &current) {
                atomic_write_file(&marker_path, format!("{}\n", targets[index]))?;
            }
        }
        Ok::<(), AppError>(())
    })();
    if let Err(error) = result {
        for (_, temporary, target) in &staged {
            let destination = backup_root.join(target);
            if destination.exists() {
                let _ = fs::rename(&destination, temporary);
            }
        }
        for (old, temporary, _) in &staged {
            if temporary.exists() {
                let _ = fs::rename(temporary, old);
            }
        }
        for (path, raw) in metadata_before {
            if let Some(raw) = raw {
                let _ = fs::write(path, raw);
            }
        }
        if let Some(raw) = marker_before {
            let _ = fs::write(marker_path, raw);
        }
        return Err(error);
    }
    Ok(true)
}

fn content_fingerprint(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn file_signature(path: &Path) -> (Option<u64>, Option<u64>, Option<u64>) {
    let metadata = match fs::metadata(path) {
        Ok(value) => value,
        Err(_) => return (None, None, None),
    };

    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .and_then(|value| u64::try_from(value.as_millis()).ok());
    let fingerprint = fs::read(path)
        .ok()
        .map(|contents| content_fingerprint(&contents));

    (modified, Some(metadata.len()), fingerprint)
}

fn build_profile_index_entry(profile_name: &str, codex_home: &Path) -> ProfileIndexEntry {
    let profile_dir = get_backup_root(Some(codex_home)).join(profile_name);
    let metadata = load_profile_metadata(profile_name, Some(codex_home));
    let account_label = metadata
        .account_label
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let has_account_identity = account_label.is_some();
    let auth_path = profile_dir.join("auth.json");
    let metadata_path = profile_dir.join("profile.json");
    let (auth_mtime_ms, auth_size, auth_fingerprint) = file_signature(&auth_path);
    let (profile_mtime_ms, profile_size, profile_fingerprint) = file_signature(&metadata_path);

    ProfileIndexEntry {
        folder_name: profile_name.to_string(),
        account_label,
        email: metadata.email,
        account_name: metadata.account_name,
        nickname: metadata.nickname_override,
        has_account_identity,
        plan_name: metadata.plan_name,
        subscription_expires_at: metadata.subscription_expires_at,
        openai_base_url: metadata.openai_base_url,
        last_authorization_method: metadata.last_authorization_method,
        auth_present: auth_path.is_file(),
        stored_quota: metadata.quota,
        stored_quota_updated_at_ms: metadata.quota_updated_at_ms,
        last_plan_check_ms: metadata.last_plan_check_ms,
        refresh_token_status: derive_refresh_token_status_from_auth_path(&auth_path),
        auth_mtime_ms,
        auth_size,
        auth_fingerprint,
        profile_mtime_ms,
        profile_size,
        profile_fingerprint,
        updated_at: utc_timestamp(),
    }
}

fn save_profiles_index(index: &ProfilesIndex, codex_home: &Path) -> AppResult<()> {
    let index_path = get_profiles_index_path(Some(codex_home));
    let serialized = serde_json::to_string_pretty(index).map_err(|error| {
        AppError::new(
            "PROFILES_INDEX_INVALID",
            format!("Failed to serialize profiles index: {error}"),
        )
    })?;

    atomic_write_file(&index_path, format!("{serialized}\n")).map_err(|error| {
        AppError::new(
            "PROFILES_INDEX_WRITE_FAILED",
            format!(
                "Failed to write profiles index {}: {}",
                index_path.display(),
                error.message
            ),
        )
    })
}

fn rebuild_profiles_index(codex_home: &Path) -> ProfilesIndex {
    let backup_root = get_backup_root(Some(codex_home));
    let current_profile = resolve_current_profile(&backup_root);
    let mut profile_dirs = list_profile_dirs(&backup_root);
    profile_dirs.sort_by(|left, right| {
        let left = left
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let right = right
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        compare_profile_names(left, right)
    });
    let profiles = profile_dirs
        .iter()
        .filter_map(|profile_dir| profile_dir.file_name().and_then(|name| name.to_str()))
        .map(|profile_name| build_profile_index_entry(profile_name, codex_home))
        .collect::<Vec<_>>();

    ProfilesIndex {
        schema_version: PROFILES_INDEX_SCHEMA_VERSION,
        updated_at: utc_timestamp(),
        current_profile,
        profiles,
    }
}

fn load_profiles_index_file(codex_home: &Path) -> Option<ProfilesIndex> {
    let raw = fs::read_to_string(get_profiles_index_path(Some(codex_home))).ok()?;
    let index = serde_json::from_str::<ProfilesIndex>(&raw).ok()?;
    (index.schema_version == PROFILES_INDEX_SCHEMA_VERSION).then_some(index)
}

fn index_entry_matches_disk(entry: &ProfileIndexEntry, profile_dir: &Path) -> bool {
    let auth_path = profile_dir.join("auth.json");
    let metadata_path = profile_dir.join("profile.json");
    let (auth_mtime_ms, auth_size, auth_fingerprint) = file_signature(&auth_path);
    let (profile_mtime_ms, profile_size, profile_fingerprint) = file_signature(&metadata_path);

    entry.auth_present == auth_path.is_file()
        && entry.auth_mtime_ms == auth_mtime_ms
        && entry.auth_size == auth_size
        && entry.auth_fingerprint == auth_fingerprint
        && entry.profile_mtime_ms == profile_mtime_ms
        && entry.profile_size == profile_size
        && entry.profile_fingerprint == profile_fingerprint
}

pub fn load_profiles_index(codex_home: Option<&Path>) -> AppResult<ProfilesIndex> {
    let codex_home = codex_home.map(PathBuf::from).unwrap_or_else(get_codex_home);
    if let Some(cached) = try_load_cached_index(&codex_home) {
        return Ok(cached);
    }

    static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
    let _guard = LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    // Double check cache after acquiring the lock to avoid redundant disk reads/writes
    if let Some(cached) = try_load_cached_index(&codex_home) {
        return Ok(cached);
    }

    let backup_root = get_backup_root(Some(&codex_home));
    let (mut index, mut changed) = match load_profiles_index_file(&codex_home) {
        Some(index) => (index, false),
        None => (rebuild_profiles_index(&codex_home), true),
    };
    let current_profile = resolve_current_profile(&backup_root);
    changed = changed
        || index.schema_version != PROFILES_INDEX_SCHEMA_VERSION
        || index.current_profile != current_profile;

    let mut reconciled_profiles = Vec::new();
    for profile_dir in list_profile_dirs(&backup_root) {
        let Some(profile_name) = profile_dir.file_name().and_then(|name| name.to_str()) else {
            continue;
        };

        let next_entry = match index
            .profiles
            .iter()
            .find(|entry| entry.folder_name == profile_name)
        {
            Some(entry) if index_entry_matches_disk(entry, &profile_dir) => entry.clone(),
            _ => {
                changed = true;
                build_profile_index_entry(profile_name, &codex_home)
            }
        };

        reconciled_profiles.push(next_entry);
    }

    if reconciled_profiles.len() != index.profiles.len() {
        changed = true;
    }

    index.schema_version = PROFILES_INDEX_SCHEMA_VERSION;
    index.current_profile = current_profile;
    index.profiles = reconciled_profiles;

    if changed {
        index.updated_at = utc_timestamp();
        save_profiles_index(&index, &codex_home)?;
    }

    store_cached_index(&codex_home, &index);
    Ok(index)
}

fn build_profile_card(entry: &ProfileIndexEntry, current_profile: Option<&str>) -> ProfileCard {
    let status = if !entry.auth_present {
        "missing_auth"
    } else if current_profile == Some(entry.folder_name.as_str()) {
        "current"
    } else {
        "available"
    }
    .to_string();

    ProfileCard {
        folder_name: entry.folder_name.clone(),
        display_title: build_display_title(
            &entry.folder_name,
            entry.nickname.as_deref(),
            entry.account_name.as_deref(),
            entry.email.as_deref(),
            entry.account_label.as_deref(),
        ),
        account_label: entry.account_label.clone(),
        email: entry.email.clone(),
        account_name: entry.account_name.clone(),
        nickname: entry.nickname.clone(),
        status,
        auth_present: entry.auth_present,
        has_account_identity: entry.has_account_identity,
        plan_name: entry.plan_name.clone(),
        subscription_days_left: compute_subscription_days_left(
            entry.subscription_expires_at.as_deref(),
        ),
        openai_base_url: entry.openai_base_url.clone(),
        last_authorization_method: entry.last_authorization_method,
        quota: normalize_quota_summary(
            Some(entry.stored_quota.clone()),
            entry.plan_name.as_deref(),
            entry.has_account_identity,
        ),
        last_plan_check_ms: entry.last_plan_check_ms,
        refresh_token_status: entry.refresh_token_status.clone(),
    }
}

fn build_current_card(entry: &ProfileIndexEntry, codex_home: &Path) -> CurrentCard {
    let profile_dir = get_backup_root(Some(codex_home)).join(&entry.folder_name);

    CurrentCard {
        folder_name: entry.folder_name.clone(),
        display_title: build_current_display_title(
            entry.nickname.as_deref(),
            entry.account_name.as_deref(),
            entry.email.as_deref(),
            entry.account_label.as_deref(),
        ),
        account_label: entry.account_label.clone(),
        email: entry.email.clone(),
        account_name: entry.account_name.clone(),
        nickname: entry.nickname.clone(),
        has_account_identity: entry.has_account_identity,
        plan_name: entry.plan_name.clone(),
        subscription_days_left: compute_subscription_days_left(
            entry.subscription_expires_at.as_deref(),
        ),
        profile_folder_path: profile_dir.to_string_lossy().into_owned(),
        last_plan_check_ms: entry.last_plan_check_ms,
        refresh_token_status: entry.refresh_token_status.clone(),
    }
}

fn select_current_quota(
    entry: &ProfileIndexEntry,
    live_snapshot: Option<&super::session_usage::LocalQuotaSnapshot>,
) -> crate::models::QuotaSummary {
    let stored_updated_at_ms = entry.stored_quota_updated_at_ms.unwrap_or(0);

    match live_snapshot {
        Some(snapshot) if snapshot.source_mtime_ms.unwrap_or(0) > stored_updated_at_ms => {
            let mut selected = snapshot.quota.clone();
            if selected.rate_limit_reset_credits.is_none() {
                selected.rate_limit_reset_credits =
                    entry.stored_quota.rate_limit_reset_credits.clone();
            }
            selected
        }
        _ => entry.stored_quota.clone(),
    }
}

pub fn load_profiles_snapshot(codex_home: Option<&Path>) -> AppResult<ProfilesSnapshotResponse> {
    let codex_home = codex_home.map(PathBuf::from).unwrap_or_else(get_codex_home);
    let _ = compact_profile_folders(&codex_home)?;
    let index = load_profiles_index(Some(&codex_home))?;
    let backup_root = get_backup_root(Some(&codex_home));
    let unmanaged_live_account = detect_unmanaged_live_account(&backup_root, &codex_home);
    let current_profile = if unmanaged_live_account.is_some() {
        None
    } else {
        resolve_backup_target(&backup_root, &codex_home)
    };
    let current_profile_ref = current_profile.as_deref();
    let current_entry = current_profile_ref.and_then(|profile_name| {
        index
            .profiles
            .iter()
            .find(|entry| entry.folder_name == profile_name)
    });

    Ok(ProfilesSnapshotResponse {
        page_size: DEFAULT_PAGE_SIZE,
        profiles: index
            .profiles
            .iter()
            .map(|entry| build_profile_card(entry, current_profile_ref))
            .collect(),
        current_card: current_entry.map(|entry| build_current_card(entry, &codex_home)),
        current_quota_card: current_entry.map(|entry| {
            normalize_quota_summary(
                Some(entry.stored_quota.clone()),
                entry.plan_name.as_deref(),
                entry.has_account_identity,
            )
        }),
        unmanaged_live_account,
    })
}

pub fn load_current_live_quota(codex_home: Option<&Path>) -> AppResult<CurrentQuotaResponse> {
    let codex_home = codex_home.map(PathBuf::from).unwrap_or_else(get_codex_home);
    let index = load_profiles_index(Some(&codex_home))?;
    let backup_root = get_backup_root(Some(&codex_home));
    if detect_unmanaged_live_account(&backup_root, &codex_home).is_some() {
        return Ok(CurrentQuotaResponse {
            profile: None,
            quota: None,
        });
    }
    let Some(current_profile) = resolve_backup_target(&backup_root, &codex_home) else {
        return Ok(CurrentQuotaResponse {
            profile: None,
            quota: None,
        });
    };
    let Some(entry) = index
        .profiles
        .iter()
        .find(|profile| profile.folder_name == current_profile)
    else {
        return Ok(CurrentQuotaResponse {
            profile: Some(current_profile),
            quota: None,
        });
    };

    let activated_at_ms = profile_activated_at_ms(&current_profile, &backup_root);
    let live_snapshot = load_latest_local_quota_snapshot_since(Some(&codex_home), activated_at_ms);
    let quota = normalize_quota_summary(
        Some(select_current_quota(entry, live_snapshot.as_ref())),
        entry.plan_name.as_deref(),
        entry.has_account_identity,
    );

    Ok(CurrentQuotaResponse {
        profile: Some(entry.folder_name.clone()),
        quota: Some(quota),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        AuthorizationMethod, QuotaSummary, RateLimitResetCredits, RefreshTokenHealth,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestCacheGuard;

    impl TestCacheGuard {
        fn enable() -> Self {
            TEST_PROFILE_INDEX_CACHE_ENABLED.with(|enabled| enabled.set(true));
            Self
        }
    }

    impl Drop for TestCacheGuard {
        fn drop(&mut self) {
            TEST_PROFILE_INDEX_CACHE_ENABLED.with(|enabled| enabled.set(false));
        }
    }

    fn temp_codex_home(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("codex-switch-profiles-index-{name}-{unique}"))
    }

    fn auth_with_account(account_id: &str) -> String {
        format!(
            "{{\"tokens\":{{\"account_id\":{}}}}}",
            serde_json::Value::String(account_id.to_string())
        )
    }

    fn write_profile_auth(codex_home: &Path, profile: &str, refresh_expiry_ms: u64) {
        let profile_dir = get_backup_root(Some(codex_home)).join(profile);
        fs::create_dir_all(&profile_dir).unwrap();
        let auth_json = serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "refresh_token": "refresh-secret",
                "refresh_token_expires_at_ms": refresh_expiry_ms
            }
        });
        fs::write(
            profile_dir.join("auth.json"),
            serde_json::to_string(&auth_json).unwrap(),
        )
        .unwrap();
    }

    fn only_profile_health(index: &ProfilesIndex) -> RefreshTokenHealth {
        index
            .profiles
            .first()
            .expect("expected one profile")
            .refresh_token_status
            .health
    }

    #[test]
    fn compact_profile_folders_reassigns_all_cards_and_current_marker() {
        let codex_home = temp_codex_home("compact-cards");
        let backup_root = get_backup_root(Some(&codex_home));
        for (name, account) in [("c", "acct_C"), ("d", "acct_D")] {
            let dir = backup_root.join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("auth.json"), auth_with_account(account)).unwrap();
            fs::write(
                dir.join("profile.json"),
                format!(r#"{{"folder_name":"{name}","account_label":"{account}"}}"#),
            )
            .unwrap();
        }
        fs::write(backup_root.join(".current_profile"), "c\n").unwrap();

        assert!(compact_profile_folders(&codex_home).unwrap());
        assert!(backup_root.join("a").is_dir());
        assert!(backup_root.join("b").is_dir());
        assert!(!backup_root.join("c").exists());
        assert_eq!(
            fs::read_to_string(backup_root.join(".current_profile"))
                .unwrap()
                .trim(),
            "a"
        );
        assert!(
            fs::read_to_string(backup_root.join("a").join("profile.json"))
                .unwrap()
                .contains(r#""folder_name": "a"#)
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn snapshot_compacts_a_single_legacy_folder_name() {
        let codex_home = temp_codex_home("compact-single-legacy");
        let backup_root = get_backup_root(Some(&codex_home));
        let legacy_dir = backup_root.join("work");
        fs::create_dir_all(&legacy_dir).unwrap();
        fs::write(legacy_dir.join("auth.json"), auth_with_account("acct_work")).unwrap();
        fs::write(
            legacy_dir.join("profile.json"),
            r#"{"folder_name":"work","account_label":"acct_work"}"#,
        )
        .unwrap();

        let snapshot = load_profiles_snapshot(Some(&codex_home)).unwrap();
        assert_eq!(snapshot.profiles[0].folder_name, "a");
        assert!(backup_root.join("a").is_dir());
        assert!(!backup_root.join("work").exists());
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn snapshot_fails_closed_on_unknown_or_empty_lifecycle_lock() {
        let codex_home = temp_codex_home("unknown-lock-owner");
        let lock_path = get_switch_lock_path(Some(&codex_home));
        fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
        fs::write(&lock_path, b"").unwrap();

        let error = load_profiles_snapshot(Some(&codex_home)).unwrap_err();

        assert_eq!(error.error_code, "PROFILE_LOCK_UNKNOWN");
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn snapshot_fails_closed_when_switch_lock_path_is_not_a_file() {
        let codex_home = temp_codex_home("invalid-lock-path");
        let lock_path = get_switch_lock_path(Some(&codex_home));
        fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
        fs::create_dir_all(&lock_path).unwrap();

        let error = load_profiles_snapshot(Some(&codex_home)).unwrap_err();

        assert_eq!(error.error_code, "PROFILE_REPACK_BUSY");
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn snapshot_surfaces_last_authorization_method_from_profile_metadata() {
        let codex_home = temp_codex_home("authorization-method");
        let profile_dir = get_backup_root(Some(&codex_home)).join("a");
        fs::create_dir_all(&profile_dir).unwrap();
        fs::write(profile_dir.join("auth.json"), auth_with_account("acct_A")).unwrap();
        fs::write(
            profile_dir.join("profile.json"),
            r#"{"folder_name":"a","account_label":"acct_A","last_authorization_method":"device_code"}"#,
        )
        .unwrap();

        let snapshot = load_profiles_snapshot(Some(&codex_home)).unwrap();

        assert_eq!(
            snapshot.profiles[0].last_authorization_method,
            Some(AuthorizationMethod::DeviceCode)
        );

        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn reload_after_mutation_invalidates_short_lived_cache() {
        const THIRTY_DAYS_MS: u64 = 30 * 24 * 60 * 60 * 1000;
        let _cache = TestCacheGuard::enable();
        let codex_home = temp_codex_home("cache-invalidation");
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        write_profile_auth(&codex_home, "a", now_ms + THIRTY_DAYS_MS);

        let first = load_profiles_index(Some(&codex_home)).unwrap();
        assert_eq!(only_profile_health(&first), RefreshTokenHealth::Healthy);

        write_profile_auth(&codex_home, "a", now_ms.saturating_sub(60_000));
        let cached = load_profiles_index(Some(&codex_home)).unwrap();
        assert_eq!(only_profile_health(&cached), RefreshTokenHealth::Healthy);

        let reloaded = reload_profiles_index_after_mutation(Some(&codex_home)).unwrap();
        assert_eq!(
            only_profile_health(&reloaded),
            RefreshTokenHealth::NeedsLogin
        );

        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn snapshot_and_live_quota_hide_stale_current_profile_when_live_account_unmanaged() {
        let codex_home = temp_codex_home("snapshot-unmanaged");
        let backup_root = get_backup_root(Some(&codex_home));
        let profile_a_dir = backup_root.join("a");
        fs::create_dir_all(&profile_a_dir).unwrap();
        fs::write(profile_a_dir.join("auth.json"), auth_with_account("acct_A")).unwrap();
        fs::write(
            profile_a_dir.join("profile.json"),
            r#"{"folder_name":"a","account_label":"acct_A","quota":{"five_hour":{"remaining_percent":42},"weekly":{"remaining_percent":88}}}"#,
        )
        .unwrap();
        fs::write(backup_root.join(".current_profile"), "a\n").unwrap();
        fs::write(codex_home.join("auth.json"), auth_with_account("acct_Z")).unwrap();

        let snapshot = load_profiles_snapshot(Some(&codex_home)).unwrap();
        let current_quota = load_current_live_quota(Some(&codex_home)).unwrap();

        assert_eq!(snapshot.unmanaged_live_account.as_deref(), Some("acct_Z"));
        assert!(snapshot.current_card.is_none());
        assert!(snapshot.current_quota_card.is_none());
        assert!(
            snapshot
                .profiles
                .iter()
                .all(|card| card.status != "current"),
            "no stale profile card should be displayed as current"
        );
        assert_eq!(current_quota.profile, None);
        assert!(current_quota.quota.is_none());

        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn current_live_quota_hides_unmanaged_account_when_marker_is_empty_placeholder() {
        let codex_home = temp_codex_home("quota-unmanaged-placeholder");
        let backup_root = get_backup_root(Some(&codex_home));
        let profile_a_dir = backup_root.join("a");
        fs::create_dir_all(&profile_a_dir).unwrap();
        fs::write(profile_a_dir.join("auth.json"), "{}\n").unwrap();
        fs::write(
            profile_a_dir.join("profile.json"),
            r#"{"folder_name":"a","quota":{"five_hour":{"remaining_percent":42},"weekly":{"remaining_percent":88}}}"#,
        )
        .unwrap();
        fs::write(backup_root.join(".current_profile"), "a\n").unwrap();
        fs::write(codex_home.join("auth.json"), auth_with_account("acct_Z")).unwrap();

        let current_quota = load_current_live_quota(Some(&codex_home)).unwrap();

        assert_eq!(current_quota.profile, None);
        assert!(current_quota.quota.is_none());

        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn newer_live_windows_preserve_reset_credits_with_their_api_provenance() {
        let mut stored_quota = QuotaSummary::default();
        stored_quota.rate_limit_reset_credits = Some(RateLimitResetCredits {
            available_count: Some(2),
            credits: Vec::new(),
            provenance: crate::models::QuotaProvenance::chatgpt_api(Some(90)),
        });
        let entry = ProfileIndexEntry {
            stored_quota,
            stored_quota_updated_at_ms: Some(100),
            ..ProfileIndexEntry::default()
        };
        let mut live_quota = QuotaSummary::default();
        live_quota.weekly.remaining_percent = Some(1);
        let live = super::super::session_usage::LocalQuotaSnapshot {
            quota: live_quota,
            source_mtime_ms: Some(200),
        };

        let selected = select_current_quota(&entry, Some(&live));

        assert_eq!(selected.weekly.remaining_percent, Some(1));
        assert_eq!(
            selected
                .rate_limit_reset_credits
                .as_ref()
                .and_then(|credits| credits.available_count),
            Some(2)
        );
        assert_eq!(
            selected
                .rate_limit_reset_credits
                .as_ref()
                .and_then(|credits| credits.provenance.observed_at_ms),
            Some(90)
        );
    }

    #[test]
    fn snapshot_and_live_quota_follow_managed_live_identity_instead_of_stale_marker() {
        let codex_home = temp_codex_home("snapshot-managed-drift");
        let backup_root = get_backup_root(Some(&codex_home));
        let profile_a_dir = backup_root.join("a");
        let profile_b_dir = backup_root.join("b");
        fs::create_dir_all(&profile_a_dir).unwrap();
        fs::create_dir_all(&profile_b_dir).unwrap();
        fs::write(profile_a_dir.join("auth.json"), auth_with_account("acct_A")).unwrap();
        fs::write(profile_b_dir.join("auth.json"), auth_with_account("acct_B")).unwrap();
        fs::write(
            profile_a_dir.join("profile.json"),
            r#"{"folder_name":"a","account_label":"a@example.com","quota":{"five_hour":{"remaining_percent":42},"weekly":{"remaining_percent":88}}}"#,
        )
        .unwrap();
        fs::write(
            profile_b_dir.join("profile.json"),
            r#"{"folder_name":"b","account_label":"acct_B","quota":{"five_hour":{"remaining_percent":73},"weekly":{"remaining_percent":91}}}"#,
        )
        .unwrap();
        fs::write(backup_root.join(".current_profile"), "a\n").unwrap();
        fs::write(codex_home.join("auth.json"), auth_with_account("acct_B")).unwrap();

        let snapshot = load_profiles_snapshot(Some(&codex_home)).unwrap();
        let current_quota = load_current_live_quota(Some(&codex_home)).unwrap();

        assert_eq!(snapshot.unmanaged_live_account, None);
        assert_eq!(
            snapshot
                .current_card
                .as_ref()
                .map(|card| card.folder_name.as_str()),
            Some("b")
        );
        assert_eq!(
            snapshot
                .profiles
                .iter()
                .find(|card| card.status == "current")
                .map(|card| card.folder_name.as_str()),
            Some("b")
        );
        assert_eq!(current_quota.profile.as_deref(), Some("b"));
        assert_eq!(
            current_quota
                .quota
                .as_ref()
                .and_then(|quota| quota.five_hour.remaining_percent),
            Some(73)
        );

        let _ = fs::remove_dir_all(&codex_home);
    }
}
