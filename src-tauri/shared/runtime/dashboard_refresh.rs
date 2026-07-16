use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::errors::{AppResult, CommandError};
use crate::models::{CurrentQuotaResponse, PlanSource, QuotaProvenance};
use crate::platform::hooks::PlatformHooks;

use super::chatgpt_api::{ChatGptApiRefreshResult, RefreshOptions, PLAN_FRESHNESS_TTL_MS};
use super::paths::{get_backup_root, get_codex_home};
use super::runtime_isolation::RUNTIME_AUTH_FILENAME;

/// Minimum age of the cached quota before the silent background tick will
/// pay for an HTTP refresh. Tuned to be longer than the UI's local 15s
/// JSONL poll (so we don't double-refresh) but short enough that the 5h
/// window stays meaningfully in sync (the window itself updates roughly
/// every minute on the OpenAI side).
const SILENT_REFRESH_MIN_AGE_MS: u64 = 5 * 60 * 1000;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|value| u64::try_from(value.as_millis()).ok())
        .unwrap_or(0)
}

fn load_current_live_quota(
    codex_home: &Path,
    load_current_live_quota_fn: &impl Fn(&Path) -> Result<CurrentQuotaResponse, CommandError>,
) -> Result<CurrentQuotaResponse, CommandError> {
    load_current_live_quota_fn(codex_home)
}

fn acquire_background_refresh_lock(
    hooks: &dyn PlatformHooks,
    codex_home: &Path,
    context: &str,
) -> AppResult<Option<crate::shared::process_lock::ProcessLockGuard>> {
    hooks
        .acquire_background_refresh_lock(codex_home)
        .map_err(|error| {
            eprintln!(
                "{context}: skipped because refresh lock is busy/unavailable ({}): {}",
                error.error_code, error.message
            );
            error
        })
}

fn sync_live_auth_after_token_rotation(
    hooks: &dyn PlatformHooks,
    profile_name: &str,
    codex_home: &Path,
    oauth_token_rotated: bool,
) {
    if !oauth_token_rotated {
        return;
    }

    let auth_path = get_backup_root(Some(codex_home))
        .join(profile_name)
        .join(RUNTIME_AUTH_FILENAME);
    if let Err(error) =
        hooks.sync_live_auth_if_current_profile(profile_name, codex_home, &auth_path)
    {
        eprintln!(
            "dashboard refresh: failed to sync live auth for {profile_name}: {}",
            error.message
        );
    }
}

pub fn refresh_active_profile_quota_silent(
    hooks: &dyn PlatformHooks,
    mut refresh_api: impl FnMut(&str, &Path) -> ChatGptApiRefreshResult,
    load_current_live_quota_fn: impl Fn(&Path) -> Result<CurrentQuotaResponse, CommandError>,
) -> Result<CurrentQuotaResponse, CommandError> {
    let codex_home = get_codex_home();
    let backup_root = get_backup_root(Some(&codex_home));
    let index = super::profiles_index::load_profiles_index(Some(&codex_home))
        .map_err(CommandError::from)?;
    let Some(profile_name) = index.current_profile.clone() else {
        return load_current_live_quota(&codex_home, &load_current_live_quota_fn);
    };
    let Some(entry) = index
        .profiles
        .iter()
        .find(|profile| profile.folder_name == profile_name)
    else {
        return load_current_live_quota(&codex_home, &load_current_live_quota_fn);
    };

    let profile_dir = backup_root.join(&entry.folder_name);
    if !super::chatgpt_api::profile_supports_api_refresh(&profile_dir) {
        return load_current_live_quota(&codex_home, &load_current_live_quota_fn);
    }

    let stored_age_ms = entry
        .stored_quota_updated_at_ms
        .map(|stored| now_ms().saturating_sub(stored))
        .unwrap_or(u64::MAX);
    if stored_age_ms < SILENT_REFRESH_MIN_AGE_MS {
        return load_current_live_quota(&codex_home, &load_current_live_quota_fn);
    }

    let _refresh_guard =
        match acquire_background_refresh_lock(hooks, &codex_home, "active quota refresh") {
            Ok(guard) => guard,
            Err(_) => return load_current_live_quota(&codex_home, &load_current_live_quota_fn),
        };

    match refresh_api(&profile_name, &codex_home) {
        Ok(snapshot) => {
            let plan_type_from_api = snapshot.plan_type.clone();
            let observed_at_ms = Some(now_ms());
            sync_live_auth_after_token_rotation(
                hooks,
                &profile_name,
                &codex_home,
                snapshot.oauth_token_rotated,
            );
            let _ = super::metadata::sync_profile_quota_with_provenance(
                &profile_name,
                snapshot.quota.unwrap_or_default(),
                observed_at_ms,
                Some(QuotaProvenance::chatgpt_api(observed_at_ms)),
                Some(&codex_home),
            );
            let _ = super::metadata::sync_profile_metadata_from_auth(
                &profile_name,
                plan_type_from_api,
                PlanSource::ChatGptApi,
                Some(&codex_home),
            );
            let _ = super::profiles_index::reload_profiles_index_after_mutation(Some(&codex_home));
        }
        Err(error) => {
            sync_live_auth_after_token_rotation(
                hooks,
                &profile_name,
                &codex_home,
                error.oauth_token_rotated,
            );
        }
    }

    load_current_live_quota(&codex_home, &load_current_live_quota_fn)
}

pub fn refresh_all_oauth_profile_plans_silent(
    hooks: &dyn PlatformHooks,
    mut refresh_api: impl FnMut(&str, &Path, RefreshOptions) -> ChatGptApiRefreshResult,
) -> Result<u32, CommandError> {
    // Invoked by the frontend on startup and local-day rollover. A profile
    // whose plan was checked within PLAN_FRESHNESS_TTL_MS (6h) is skipped, and
    // RefreshOptions keeps OAuth token rotation lazy: only expired access
    // tokens or a usage 401 trigger rotation.
    let codex_home = get_codex_home();
    let backup_root = get_backup_root(Some(&codex_home));
    let index = super::profiles_index::load_profiles_index(Some(&codex_home))
        .map_err(CommandError::from)?;
    let now_ms = now_ms();

    let _refresh_guard =
        match acquire_background_refresh_lock(hooks, &codex_home, "bulk plan refresh") {
            Ok(guard) => guard,
            Err(_) => return Ok(0),
        };

    let mut refreshed: u32 = 0;
    for entry in &index.profiles {
        let profile_dir = backup_root.join(&entry.folder_name);
        if !super::chatgpt_api::profile_supports_api_refresh(&profile_dir) {
            continue;
        }

        if let Some(last_check) = entry.last_plan_check_ms {
            if now_ms.saturating_sub(last_check) < PLAN_FRESHNESS_TTL_MS {
                continue;
            }
        }

        let snapshot = match refresh_api(
            &entry.folder_name,
            &codex_home,
            RefreshOptions {
                force_token_rotation: false,
            },
        ) {
            Ok(value) => value,
            Err(error) => {
                sync_live_auth_after_token_rotation(
                    hooks,
                    &entry.folder_name,
                    &codex_home,
                    error.oauth_token_rotated,
                );
                eprintln!(
                    "bulk plan refresh skipped {} ({}): {}",
                    entry.folder_name, error.error.error_code, error.error.message
                );
                continue;
            }
        };

        let plan_type_from_api = snapshot.plan_type.clone();
        let observed_at_ms = Some(now_ms);
        sync_live_auth_after_token_rotation(
            hooks,
            &entry.folder_name,
            &codex_home,
            snapshot.oauth_token_rotated,
        );
        if let Err(error) = super::metadata::sync_profile_quota_with_provenance(
            &entry.folder_name,
            snapshot.quota.unwrap_or_default(),
            observed_at_ms,
            Some(QuotaProvenance::chatgpt_api(observed_at_ms)),
            Some(&codex_home),
        ) {
            eprintln!(
                "bulk plan refresh: failed to persist quota for {}: {}",
                entry.folder_name, error.message
            );
        }
        if let Err(error) = super::metadata::sync_profile_metadata_from_auth(
            &entry.folder_name,
            plan_type_from_api,
            PlanSource::ChatGptApi,
            Some(&codex_home),
        ) {
            eprintln!(
                "bulk plan refresh: failed to persist metadata for {}: {}",
                entry.folder_name, error.message
            );
        }
        refreshed += 1;
    }

    if refreshed > 0 {
        if let Err(error) =
            super::profiles_index::reload_profiles_index_after_mutation(Some(&codex_home))
        {
            eprintln!(
                "bulk plan refresh: post-loop index reload failed: {}",
                error.message
            );
        }
    }
    Ok(refreshed)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::errors::AppError;
    use crate::models::CurrentQuotaResponse;
    use crate::platform;
    use crate::shared::chatgpt_api::{ChatGptApiRefreshFailure, ChatGptApiSnapshot};
    use crate::shared::process_lock::acquire_process_lock;
    use crate::windows::{env_guard, EnvVarGuard};
    use std::cell::Cell;
    use std::fs;
    use std::path::PathBuf;

    fn temp_codex_home(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("codex-switch-dashboard-refresh-{name}-{unique}"))
    }

    fn quota_response(_: &Path) -> Result<CurrentQuotaResponse, CommandError> {
        Ok(CurrentQuotaResponse {
            profile: None,
            quota: None,
        })
    }

    fn write_profile(codex_home: &Path, profile_name: &str, auth: &str, metadata: &str) {
        let profile_dir = get_backup_root(Some(codex_home)).join(profile_name);
        fs::create_dir_all(&profile_dir).unwrap();
        fs::write(profile_dir.join("auth.json"), auth).unwrap();
        fs::write(profile_dir.join("profile.json"), metadata).unwrap();
    }

    fn set_current_profile(codex_home: &Path, profile_name: &str) {
        fs::create_dir_all(get_backup_root(Some(codex_home))).unwrap();
        fs::write(
            get_backup_root(Some(codex_home)).join(".current_profile"),
            format!("{profile_name}\n"),
        )
        .unwrap();
    }

    fn chatgpt_auth(access_token: &str, refresh_token: &str) -> String {
        format!(
            r#"{{"auth_mode":"chatgpt","tokens":{{"access_token":"{access_token}","refresh_token":"{refresh_token}","id_token":"","account_id":"acct"}}}}"#
        )
    }

    fn api_key_auth() -> &'static str {
        r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-test"}"#
    }

    fn snapshot(oauth_token_rotated: bool) -> ChatGptApiSnapshot {
        ChatGptApiSnapshot {
            oauth_token_rotated,
            ..ChatGptApiSnapshot::default()
        }
    }

    fn refresh_failure(oauth_token_rotated: bool) -> ChatGptApiRefreshFailure {
        ChatGptApiRefreshFailure::new(
            AppError::new(
                "CHATGPT_USAGE_HTTP_ERROR",
                "usage failed after token refresh",
            ),
            oauth_token_rotated,
        )
    }

    #[test]
    fn silent_active_refresh_syncs_live_auth_after_token_rotation() {
        let _guard = env_guard();
        let _env = EnvVarGuard::capture(&["CODEX_HOME"]);
        let codex_home = temp_codex_home("active-rotation");
        std::env::set_var("CODEX_HOME", &codex_home);
        let old_profile_auth = chatgpt_auth("old-profile", "refresh");
        let new_profile_auth = chatgpt_auth("new-profile", "refresh-2");
        write_profile(
            &codex_home,
            "a",
            &old_profile_auth,
            r#"{"folder_name":"a","account_label":"a@example.com"}"#,
        );
        fs::write(
            codex_home.join("auth.json"),
            chatgpt_auth("old-live", "live"),
        )
        .unwrap();
        set_current_profile(&codex_home, "a");

        refresh_active_profile_quota_silent(
            platform::current_hooks(),
            |profile_name, codex_home| {
                fs::write(
                    get_backup_root(Some(codex_home))
                        .join(profile_name)
                        .join("auth.json"),
                    &new_profile_auth,
                )
                .unwrap();
                Ok(snapshot(true))
            },
            quota_response,
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(codex_home.join("auth.json")).unwrap(),
            new_profile_auth
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn silent_active_refresh_syncs_live_auth_when_rotated_refresh_fails() {
        let _guard = env_guard();
        let _env = EnvVarGuard::capture(&["CODEX_HOME"]);
        let codex_home = temp_codex_home("active-rotation-error");
        std::env::set_var("CODEX_HOME", &codex_home);
        let old_profile_auth = chatgpt_auth("old-profile", "refresh");
        let new_profile_auth = chatgpt_auth("new-profile", "refresh-2");
        write_profile(
            &codex_home,
            "a",
            &old_profile_auth,
            r#"{"folder_name":"a","account_label":"a@example.com"}"#,
        );
        fs::write(
            codex_home.join("auth.json"),
            chatgpt_auth("old-live", "live"),
        )
        .unwrap();
        set_current_profile(&codex_home, "a");

        refresh_active_profile_quota_silent(
            platform::current_hooks(),
            |profile_name, codex_home| {
                fs::write(
                    get_backup_root(Some(codex_home))
                        .join(profile_name)
                        .join("auth.json"),
                    &new_profile_auth,
                )
                .unwrap();
                Err(refresh_failure(true))
            },
            quota_response,
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(codex_home.join("auth.json")).unwrap(),
            new_profile_auth
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn silent_active_refresh_without_token_rotation_does_not_touch_live_auth() {
        let _guard = env_guard();
        let _env = EnvVarGuard::capture(&["CODEX_HOME"]);
        let codex_home = temp_codex_home("active-no-rotation");
        std::env::set_var("CODEX_HOME", &codex_home);
        let old_live_auth = chatgpt_auth("old-live", "live");
        write_profile(
            &codex_home,
            "a",
            &chatgpt_auth("old-profile", "refresh"),
            r#"{"folder_name":"a","account_label":"a@example.com"}"#,
        );
        fs::write(codex_home.join("auth.json"), &old_live_auth).unwrap();
        set_current_profile(&codex_home, "a");

        refresh_active_profile_quota_silent(
            platform::current_hooks(),
            |profile_name, codex_home| {
                fs::write(
                    get_backup_root(Some(codex_home))
                        .join(profile_name)
                        .join("auth.json"),
                    chatgpt_auth("unexpected-profile-write", "refresh"),
                )
                .unwrap();
                Ok(snapshot(false))
            },
            quota_response,
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(codex_home.join("auth.json")).unwrap(),
            old_live_auth
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn bulk_refresh_syncs_live_auth_for_current_profile_token_rotation() {
        let _guard = env_guard();
        let _env = EnvVarGuard::capture(&["CODEX_HOME"]);
        let codex_home = temp_codex_home("bulk-active-rotation");
        std::env::set_var("CODEX_HOME", &codex_home);
        let new_profile_auth = chatgpt_auth("new-profile", "refresh-2");
        write_profile(
            &codex_home,
            "a",
            &chatgpt_auth("old-profile", "refresh"),
            r#"{"folder_name":"a","account_label":"a@example.com"}"#,
        );
        fs::write(
            codex_home.join("auth.json"),
            chatgpt_auth("old-live", "live"),
        )
        .unwrap();
        set_current_profile(&codex_home, "a");

        let refreshed = refresh_all_oauth_profile_plans_silent(
            platform::current_hooks(),
            |profile_name, codex_home, _| {
                fs::write(
                    get_backup_root(Some(codex_home))
                        .join(profile_name)
                        .join("auth.json"),
                    &new_profile_auth,
                )
                .unwrap();
                Ok(snapshot(true))
            },
        )
        .unwrap();

        assert_eq!(refreshed, 1);
        assert_eq!(
            fs::read_to_string(codex_home.join("auth.json")).unwrap(),
            new_profile_auth
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn bulk_refresh_syncs_live_auth_when_current_profile_rotation_fails() {
        let _guard = env_guard();
        let _env = EnvVarGuard::capture(&["CODEX_HOME"]);
        let codex_home = temp_codex_home("bulk-active-rotation-error");
        std::env::set_var("CODEX_HOME", &codex_home);
        let new_profile_auth = chatgpt_auth("new-profile", "refresh-2");
        write_profile(
            &codex_home,
            "a",
            &chatgpt_auth("old-profile", "refresh"),
            r#"{"folder_name":"a","account_label":"a@example.com"}"#,
        );
        fs::write(
            codex_home.join("auth.json"),
            chatgpt_auth("old-live", "live"),
        )
        .unwrap();
        set_current_profile(&codex_home, "a");

        let refreshed = refresh_all_oauth_profile_plans_silent(
            platform::current_hooks(),
            |profile_name, codex_home, _| {
                fs::write(
                    get_backup_root(Some(codex_home))
                        .join(profile_name)
                        .join("auth.json"),
                    &new_profile_auth,
                )
                .unwrap();
                Err(refresh_failure(true))
            },
        )
        .unwrap();

        assert_eq!(refreshed, 0);
        assert_eq!(
            fs::read_to_string(codex_home.join("auth.json")).unwrap(),
            new_profile_auth
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn bulk_refresh_does_not_sync_live_auth_for_inactive_profile() {
        let _guard = env_guard();
        let _env = EnvVarGuard::capture(&["CODEX_HOME"]);
        let codex_home = temp_codex_home("bulk-inactive-rotation");
        std::env::set_var("CODEX_HOME", &codex_home);
        let old_live_auth = api_key_auth().to_string();
        let new_inactive_auth = chatgpt_auth("new-inactive", "refresh-2");
        write_profile(
            &codex_home,
            "a",
            api_key_auth(),
            r#"{"folder_name":"a","account_label":"a@example.com"}"#,
        );
        write_profile(
            &codex_home,
            "b",
            &chatgpt_auth("old-inactive", "refresh"),
            r#"{"folder_name":"b","account_label":"b@example.com"}"#,
        );
        fs::write(codex_home.join("auth.json"), &old_live_auth).unwrap();
        set_current_profile(&codex_home, "a");

        let refreshed = refresh_all_oauth_profile_plans_silent(
            platform::current_hooks(),
            |profile_name, codex_home, _| {
                fs::write(
                    get_backup_root(Some(codex_home))
                        .join(profile_name)
                        .join("auth.json"),
                    &new_inactive_auth,
                )
                .unwrap();
                Ok(snapshot(true))
            },
        )
        .unwrap();

        assert_eq!(refreshed, 1);
        assert_eq!(
            fs::read_to_string(codex_home.join("auth.json")).unwrap(),
            old_live_auth
        );
        assert_eq!(
            fs::read_to_string(
                get_backup_root(Some(&codex_home))
                    .join("b")
                    .join("auth.json")
            )
            .unwrap(),
            new_inactive_auth
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn bulk_refresh_lock_busy_returns_zero_without_refresh_or_write() {
        let _guard = env_guard();
        let _env = EnvVarGuard::capture(&["CODEX_HOME"]);
        let codex_home = temp_codex_home("bulk-busy");
        std::env::set_var("CODEX_HOME", &codex_home);
        let old_live_auth = chatgpt_auth("old-live", "live");
        write_profile(
            &codex_home,
            "a",
            &chatgpt_auth("old-profile", "refresh"),
            r#"{"folder_name":"a","account_label":"a@example.com"}"#,
        );
        fs::write(codex_home.join("auth.json"), &old_live_auth).unwrap();
        set_current_profile(&codex_home, "a");
        let called = Cell::new(false);
        let _lock = acquire_process_lock(Some(&codex_home), "SWITCH_IN_PROGRESS", "busy").unwrap();

        let refreshed =
            refresh_all_oauth_profile_plans_silent(platform::current_hooks(), |_, _, _| {
                called.set(true);
                Ok(snapshot(true))
            })
            .unwrap();

        assert_eq!(refreshed, 0);
        assert!(!called.get());
        assert_eq!(
            fs::read_to_string(codex_home.join("auth.json")).unwrap(),
            old_live_auth
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn silent_active_refresh_lock_busy_does_not_refresh_or_write() {
        let _guard = env_guard();
        let _env = EnvVarGuard::capture(&["CODEX_HOME"]);
        let codex_home = temp_codex_home("busy");
        std::env::set_var("CODEX_HOME", &codex_home);
        let old_live_auth = chatgpt_auth("old-live", "live");
        write_profile(
            &codex_home,
            "a",
            &chatgpt_auth("old-profile", "refresh"),
            r#"{"folder_name":"a","account_label":"a@example.com"}"#,
        );
        fs::write(codex_home.join("auth.json"), &old_live_auth).unwrap();
        set_current_profile(&codex_home, "a");
        let called = Cell::new(false);
        let _lock = acquire_process_lock(Some(&codex_home), "SWITCH_IN_PROGRESS", "busy").unwrap();

        refresh_active_profile_quota_silent(
            platform::current_hooks(),
            |_, _| {
                called.set(true);
                Ok(snapshot(true))
            },
            quota_response,
        )
        .unwrap();

        assert!(!called.get());
        assert_eq!(
            fs::read_to_string(codex_home.join("auth.json")).unwrap(),
            old_live_auth
        );
        let _ = fs::remove_dir_all(&codex_home);
    }
}
