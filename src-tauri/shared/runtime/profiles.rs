use std::path::Path;

use chrono::{DateTime, Local, NaiveDate};

use super::fs_ops::read_text_stripped;
use super::metadata::{
    auth_is_empty_placeholder, load_account_identity_from_path, AccountIdentity,
};
use super::paths::{
    get_current_profile_file, is_profile_dir, list_profile_dirs, validate_profile_name,
    ACTIVE_MARKER_FILE,
};

pub fn build_display_title(profile_name: &str, account_label: Option<&str>) -> String {
    let account_label = account_label
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("--");

    format!("{profile_name} / {account_label}")
}

pub fn compute_subscription_days_left(subscription_expires_at: Option<&str>) -> Option<i64> {
    let value = subscription_expires_at?;
    let parsed = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .or_else(|| {
            DateTime::parse_from_rfc3339(value)
                .ok()
                .map(|datetime| datetime.with_timezone(&Local).date_naive())
        })?;

    let today = Local::now().date_naive();
    Some((parsed - today).num_days().max(0))
}

pub fn resolve_current_profile(backup_root: &Path) -> Option<String> {
    let current_profile_file = get_current_profile_file(backup_root.parent());
    let profile = read_text_stripped(&current_profile_file);
    if let Ok(profile) = validate_profile_name(&profile) {
        let profile_dir = backup_root.join(&profile);
        if is_profile_dir(&profile_dir) {
            return Some(profile);
        }
    }

    for profile_dir in list_profile_dirs(backup_root) {
        if profile_dir.join(ACTIVE_MARKER_FILE).is_file() {
            if let Some(name) = profile_dir.file_name().and_then(|value| value.to_str()) {
                return Some(name.to_string());
            }
        }
    }

    None
}

/// One-shot marker for a corrupt activation timestamp. A missing timestamp
/// deliberately falls back to legacy display behavior, while write-back skips
/// the unsafe live-session fold until the marker is re-stamped.
static ACTIVATION_PARSE_WARNING: std::sync::OnceLock<()> = std::sync::OnceLock::new();

/// Epoch milliseconds when this profile became active, recorded by
/// `set_active_marker`. Session JSONL is shared by profiles, so entries before
/// this boundary cannot safely be attributed to the active account.
pub fn profile_activated_at_ms(profile: &str, backup_root: &Path) -> Option<u64> {
    let marker_path = backup_root.join(profile).join(ACTIVE_MARKER_FILE);
    let raw = std::fs::read_to_string(&marker_path).ok()?;
    let parsed = raw
        .lines()
        .find_map(|line| line.strip_prefix("activated_at="))
        .and_then(|value| DateTime::parse_from_rfc3339(value.trim()).ok())
        .and_then(|value| u64::try_from(value.timestamp_millis()).ok());
    if parsed.is_none() {
        ACTIVATION_PARSE_WARNING.get_or_init(|| {
            eprintln!(
                "codex_switch: active marker {} has no parseable activated_at; activation scoping is disabled until it is re-stamped",
                marker_path.display()
            );
        });
    }
    parsed
}

/// Decide which profile slot the live `CODEX_HOME` state should be written back
/// into, verified against the account identity in `auth.json` rather than
/// blindly trusting `.current_profile`.
pub fn resolve_backup_target(backup_root: &Path, codex_home: &Path) -> Option<String> {
    let marked = resolve_current_profile(backup_root);

    let Some(root_identity) = load_account_identity_from_path(&codex_home.join("auth.json")) else {
        return marked;
    };

    let slot_identity = |profile: &str| {
        load_account_identity_from_path(&backup_root.join(profile).join("auth.json"))
    };

    if let Some(marked_profile) = marked.as_deref() {
        if slot_identity(marked_profile).is_some_and(|slot| slot.same_account(&root_identity)) {
            return marked;
        }
    }

    if let Some(owner) = find_profile_owning_identity(backup_root, &root_identity) {
        return Some(owner);
    }

    if let Some(marked_profile) = marked.as_deref() {
        if auth_is_empty_placeholder(&backup_root.join(marked_profile).join("auth.json")) {
            return marked;
        }
    }

    None
}

fn find_profile_owning_identity(backup_root: &Path, identity: &AccountIdentity) -> Option<String> {
    for profile_dir in list_profile_dirs(backup_root) {
        let Some(name) = profile_dir.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if load_account_identity_from_path(&profile_dir.join("auth.json"))
            .is_some_and(|slot| slot.same_account(identity))
        {
            return Some(name.to_string());
        }
    }
    None
}

/// Return a display label when the live root account is identifiable but no
/// managed profile owns it.
pub fn detect_unmanaged_live_account(backup_root: &Path, codex_home: &Path) -> Option<String> {
    let root_identity = load_account_identity_from_path(&codex_home.join("auth.json"))?;
    if find_profile_owning_identity(backup_root, &root_identity).is_some() {
        return None;
    }
    root_identity.label()
}

#[cfg(test)]
mod tests {
    use super::{detect_unmanaged_live_account, resolve_backup_target};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_codex_home(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("codex-switch-resolve-backup-{name}-{unique}"))
    }

    fn auth_with_account(account_id: &str) -> String {
        format!(
            "{{\"tokens\":{{\"account_id\":{}}}}}",
            serde_json::Value::String(account_id.to_string())
        )
    }

    fn write_profile(backup_root: &Path, profile: &str, auth_body: &str) {
        let dir = backup_root.join(profile);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("auth.json"), auth_body).unwrap();
    }

    fn set_marker(backup_root: &Path, profile: &str) {
        fs::write(backup_root.join(".current_profile"), format!("{profile}\n")).unwrap();
    }

    fn setup(name: &str) -> (PathBuf, PathBuf) {
        let codex_home = temp_codex_home(name);
        let backup_root = codex_home.join("account_backup");
        fs::create_dir_all(&backup_root).unwrap();
        (codex_home, backup_root)
    }

    #[test]
    fn returns_marked_when_root_identity_matches_marker_slot() {
        let (codex_home, backup_root) = setup("happy-path");
        write_profile(&backup_root, "a", &auth_with_account("acct_X"));
        write_profile(&backup_root, "b", &auth_with_account("acct_Y"));
        set_marker(&backup_root, "a");
        fs::write(codex_home.join("auth.json"), auth_with_account("acct_X")).unwrap();

        assert_eq!(
            resolve_backup_target(&backup_root, &codex_home).as_deref(),
            Some("a")
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn returns_none_when_root_drifted_to_unmanaged_account() {
        let (codex_home, backup_root) = setup("drift-unmanaged");
        write_profile(&backup_root, "a", &auth_with_account("acct_X"));
        write_profile(&backup_root, "b", &auth_with_account("acct_Y"));
        set_marker(&backup_root, "a");
        fs::write(codex_home.join("auth.json"), auth_with_account("acct_Z")).unwrap();

        assert_eq!(resolve_backup_target(&backup_root, &codex_home), None);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn reassigns_to_profile_that_actually_owns_the_live_account() {
        let (codex_home, backup_root) = setup("reassign");
        write_profile(&backup_root, "a", &auth_with_account("acct_X"));
        write_profile(&backup_root, "b", &auth_with_account("acct_Y"));
        set_marker(&backup_root, "a");
        fs::write(codex_home.join("auth.json"), auth_with_account("acct_Y")).unwrap();

        assert_eq!(
            resolve_backup_target(&backup_root, &codex_home).as_deref(),
            Some("b")
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn seats_new_account_into_placeholder_marker_slot() {
        let (codex_home, backup_root) = setup("placeholder-seat");
        write_profile(
            &backup_root,
            "a",
            r#"{"tokens":{"account_id":"replace-me"}}"#,
        );
        set_marker(&backup_root, "a");
        fs::write(codex_home.join("auth.json"), auth_with_account("acct_NEW")).unwrap();

        assert_eq!(
            resolve_backup_target(&backup_root, &codex_home).as_deref(),
            Some("a")
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn does_not_seat_drifted_oauth_account_onto_apikey_marker_slot() {
        let (codex_home, backup_root) = setup("apikey-not-seatable");
        let apikey = r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-real-key"}"#;
        write_profile(&backup_root, "a", apikey);
        set_marker(&backup_root, "a");
        fs::write(
            codex_home.join("auth.json"),
            auth_with_account("acct_OAUTH"),
        )
        .unwrap();

        assert_eq!(resolve_backup_target(&backup_root, &codex_home), None);
        assert_eq!(
            fs::read_to_string(backup_root.join("a").join("auth.json")).unwrap(),
            apikey
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn detect_flags_unmanaged_live_account_with_label() {
        let (codex_home, backup_root) = setup("detect-unmanaged");
        write_profile(&backup_root, "a", &auth_with_account("acct_X"));
        fs::write(codex_home.join("auth.json"), auth_with_account("acct_Z")).unwrap();

        assert_eq!(
            detect_unmanaged_live_account(&backup_root, &codex_home).as_deref(),
            Some("acct_Z")
        );
        let _ = fs::remove_dir_all(&codex_home);
    }
}
