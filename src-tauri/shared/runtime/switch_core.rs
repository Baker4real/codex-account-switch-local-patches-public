use std::path::{Path, PathBuf};

use crate::errors::{AppError, AppResult};
use crate::models::SwitchResponse;
use crate::platform::hooks::PlatformHooks;

use super::fs_ops::{
    autosave_auth, backup_root_state_to_profile, clear_active_markers, overlay_directory_contents,
    set_active_marker,
};
use super::paths::{get_backup_root, get_codex_home, is_profile_dir, validate_profile_name};
use super::process_lock::{acquire_process_lock, ProcessLockGuard};
use super::profiles::{
    detect_unmanaged_live_account, resolve_backup_target, resolve_current_profile,
};
use super::profiles_index::reload_profiles_index_after_mutation;

fn acquire_switch_lock(codex_home: Option<&Path>) -> AppResult<ProcessLockGuard> {
    acquire_process_lock(
        codex_home,
        "SWITCH_IN_PROGRESS",
        "A profile switch is already in progress.",
    )
}

pub fn switch_profile_with_home<H: PlatformHooks + ?Sized>(
    hooks: &H,
    profile_name: &str,
    codex_home: Option<&Path>,
) -> AppResult<SwitchResponse> {
    let codex_home = codex_home.map(PathBuf::from).unwrap_or_else(get_codex_home);
    let backup_root = get_backup_root(Some(&codex_home));
    if !backup_root.is_dir() {
        return Err(AppError::new(
            "BACKUP_ROOT_MISSING",
            format!("Backup folder not found: {}", backup_root.display()),
        ));
    }

    let profile_name = validate_profile_name(profile_name)?;
    let _guard = acquire_switch_lock(Some(&codex_home))?;
    let profile_dir = backup_root.join(&profile_name);
    if !is_profile_dir(&profile_dir) {
        return Err(AppError::new(
            "PROFILE_NOT_FOUND",
            format!("Profile not found: {profile_name}"),
        ));
    }
    if !profile_dir.join("auth.json").is_file() {
        return Err(AppError::new(
            "PROFILE_AUTH_MISSING",
            format!(
                "Missing auth file: {}",
                profile_dir.join("auth.json").display()
            ),
        ));
    }

    let app_was_running = hooks.quit_codex_app_if_running()?;
    if let Some(backup_target) = resolve_backup_target(&backup_root, &codex_home) {
        backup_root_state_to_profile(&backup_target, &codex_home, &backup_root)?;
    }

    autosave_auth(&codex_home)?;
    overlay_directory_contents(&profile_dir, &codex_home)?;
    hooks.sync_root_openai_base_url_for_profile(&profile_name, Some(&codex_home))?;
    set_active_marker(&profile_name, &backup_root)?;
    reload_profiles_index_after_mutation(Some(&codex_home))?;
    let warnings = hooks.reopen_codex_app_if_needed(app_was_running, Some(&codex_home));

    Ok(SwitchResponse {
        ok: true,
        profile: profile_name.clone(),
        message: format!("Switched to profile: {profile_name}"),
        warnings,
    })
}

pub fn sync_root_state_to_current_profile_with_home(
    codex_home: Option<&Path>,
) -> AppResult<Option<String>> {
    let codex_home = codex_home.map(PathBuf::from).unwrap_or_else(get_codex_home);
    let backup_root = get_backup_root(Some(&codex_home));

    let Some(target) = resolve_backup_target(&backup_root, &codex_home) else {
        if detect_unmanaged_live_account(&backup_root, &codex_home).is_some() {
            clear_active_markers(&backup_root)?;
        }
        reload_profiles_index_after_mutation(Some(&codex_home))?;
        return Ok(None);
    };

    if resolve_current_profile(&backup_root).as_deref() != Some(target.as_str()) {
        set_active_marker(&target, &backup_root)?;
    }

    backup_root_state_to_profile(&target, &codex_home, &backup_root)?;
    reload_profiles_index_after_mutation(Some(&codex_home))?;
    Ok(Some(target))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::errors::AppResult;
    use crate::platform::hooks::PlatformHooks;

    use super::switch_profile_with_home;
    use crate::shared::paths::{get_current_profile_file, get_profiles_index_path};

    struct FakeHooks {
        app_was_running: bool,
        reopen_calls: Mutex<Vec<bool>>,
    }

    impl FakeHooks {
        fn new(app_was_running: bool) -> Self {
            Self {
                app_was_running,
                reopen_calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl PlatformHooks for FakeHooks {
        fn open_or_activate_codex_app(&self, _codex_home: Option<&Path>) -> AppResult<String> {
            unreachable!("not used in switch_core tests")
        }

        fn quit_codex_app_if_running(&self) -> AppResult<bool> {
            Ok(self.app_was_running)
        }

        fn reopen_codex_app_if_needed(
            &self,
            app_was_running: bool,
            _codex_home: Option<&Path>,
        ) -> Vec<String> {
            self.reopen_calls.lock().unwrap().push(app_was_running);
            Vec::new()
        }

        fn run_codex_login(
            &self,
            _cli_codex_home: &Path,
            _runtime_codex_home: &Path,
            _profile_name: Option<&str>,
        ) -> AppResult<()> {
            unreachable!("not used in switch_core tests")
        }

        fn fetch_account_via_app_server(
            &self,
            _cli_codex_home: &Path,
            _runtime_codex_home: &Path,
        ) -> AppResult<crate::shared::codex_app_server::AppServerSnapshot> {
            unreachable!("not used in switch_core tests")
        }

        fn sync_on_window_close(&self) -> AppResult<()> {
            unreachable!("not used in switch_core tests")
        }
    }

    fn temp_codex_home(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("codex-switch-shared-switch-core-{name}-{unique}"))
    }

    fn auth_with_account(account_id: &str) -> String {
        format!(
            "{{\"tokens\":{{\"account_id\":{}}}}}",
            serde_json::Value::String(account_id.to_string())
        )
    }

    #[test]
    fn switch_profile_preserves_windows_behavior_through_hooks() {
        let codex_home = temp_codex_home("switch-success");
        let backup_root = codex_home.join("account_backup");
        let profile_a_dir = backup_root.join("a");
        let profile_b_dir = backup_root.join("b");

        fs::create_dir_all(&profile_a_dir).unwrap();
        fs::create_dir_all(&profile_b_dir).unwrap();
        fs::write(codex_home.join("auth.json"), "root-auth-before-switch\n").unwrap();
        fs::write(profile_a_dir.join("auth.json"), "profile-a-auth\n").unwrap();
        fs::write(profile_b_dir.join("auth.json"), "profile-b-auth\n").unwrap();
        fs::write(get_current_profile_file(Some(&codex_home)), "a\n").unwrap();

        let hooks = FakeHooks::new(true);
        let response = switch_profile_with_home(&hooks, "b", Some(&codex_home)).unwrap();

        assert!(response.ok);
        assert_eq!(response.profile, "b");
        assert_eq!(
            fs::read_to_string(codex_home.join("auth.json")).unwrap(),
            "profile-b-auth\n"
        );
        assert_eq!(
            fs::read_to_string(profile_a_dir.join("auth.json")).unwrap(),
            "root-auth-before-switch\n"
        );
        assert_eq!(
            fs::read_to_string(get_current_profile_file(Some(&codex_home))).unwrap(),
            "b\n"
        );
        assert!(profile_b_dir.join(".active_profile").is_file());
        assert!(get_profiles_index_path(Some(&codex_home)).is_file());
        assert_eq!(*hooks.reopen_calls.lock().unwrap(), vec![true]);

        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn switch_profile_does_not_overlay_profile_skills_into_live_home() {
        let codex_home = temp_codex_home("preserve-live-skills");
        let backup_root = codex_home.join("account_backup");
        let profile_a_dir = backup_root.join("a");
        let profile_b_dir = backup_root.join("b");
        let profile_a_skill_dir = profile_a_dir.join("skills").join("old-skill");
        let live_skill_dir = codex_home.join("skills").join("local-skill");
        let live_system_skill_dir = codex_home.join("skills").join(".system");
        let profile_system_skill_dir = profile_b_dir.join("skills").join(".system");

        fs::create_dir_all(&profile_a_dir).unwrap();
        fs::create_dir_all(&profile_a_skill_dir).unwrap();
        fs::create_dir_all(&profile_system_skill_dir).unwrap();
        fs::create_dir_all(&live_skill_dir).unwrap();
        fs::create_dir_all(&live_system_skill_dir).unwrap();
        fs::write(codex_home.join("auth.json"), "root-auth-before-switch\n").unwrap();
        fs::write(
            codex_home.join("profile.json"),
            "root-profile-before-switch\n",
        )
        .unwrap();
        fs::write(profile_a_dir.join("auth.json"), "profile-a-auth\n").unwrap();
        fs::write(profile_a_dir.join("profile.json"), "profile-a-profile\n").unwrap();
        fs::write(profile_b_dir.join("auth.json"), "profile-b-auth\n").unwrap();
        fs::write(profile_b_dir.join("profile.json"), "profile-b-profile\n").unwrap();
        fs::write(
            profile_a_skill_dir.join("SKILL.md"),
            "stale profile skill\n",
        )
        .unwrap();
        fs::write(live_skill_dir.join("SKILL.md"), "live local skill\n").unwrap();
        fs::write(
            live_system_skill_dir.join("SKILL.md"),
            "live system skill\n",
        )
        .unwrap();
        fs::write(
            profile_system_skill_dir.join("SKILL.md"),
            "profile system skill\n",
        )
        .unwrap();
        fs::write(get_current_profile_file(Some(&codex_home)), "a\n").unwrap();

        let hooks = FakeHooks::new(false);
        let response = switch_profile_with_home(&hooks, "b", Some(&codex_home)).unwrap();

        assert!(response.ok);
        assert_eq!(
            fs::read_to_string(codex_home.join("auth.json")).unwrap(),
            "profile-b-auth\n"
        );
        assert_eq!(
            fs::read_to_string(codex_home.join("profile.json")).unwrap(),
            "profile-b-profile\n"
        );
        assert_eq!(
            fs::read_to_string(profile_a_dir.join("profile.json")).unwrap(),
            "root-profile-before-switch\n"
        );
        assert_eq!(
            fs::read_to_string(profile_a_skill_dir.join("SKILL.md")).unwrap(),
            "stale profile skill\n"
        );
        assert_eq!(
            fs::read_to_string(live_skill_dir.join("SKILL.md")).unwrap(),
            "live local skill\n"
        );
        assert_eq!(
            fs::read_to_string(live_system_skill_dir.join("SKILL.md")).unwrap(),
            "live system skill\n"
        );

        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn switch_does_not_contaminate_stale_marker_profile_with_drifted_account() {
        let codex_home = temp_codex_home("drift-no-contaminate");
        let backup_root = codex_home.join("account_backup");
        let profile_a_dir = backup_root.join("a");
        let profile_b_dir = backup_root.join("b");
        fs::create_dir_all(&profile_a_dir).unwrap();
        fs::create_dir_all(&profile_b_dir).unwrap();

        fs::write(profile_a_dir.join("auth.json"), auth_with_account("acct_X")).unwrap();
        fs::write(profile_b_dir.join("auth.json"), auth_with_account("acct_B")).unwrap();
        fs::write(get_current_profile_file(Some(&codex_home)), "a\n").unwrap();
        fs::write(codex_home.join("auth.json"), auth_with_account("acct_Z")).unwrap();

        let hooks = FakeHooks::new(false);
        let response = switch_profile_with_home(&hooks, "b", Some(&codex_home)).unwrap();

        assert!(response.ok);
        assert_eq!(
            fs::read_to_string(profile_a_dir.join("auth.json")).unwrap(),
            auth_with_account("acct_X"),
            "stale-marker profile must keep its own account, not the drifted live one"
        );
        assert_eq!(
            fs::read_to_string(codex_home.join("auth.json")).unwrap(),
            auth_with_account("acct_B")
        );

        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn bootstrap_sync_heals_marker_and_routes_backup_on_drift_to_managed_profile() {
        let codex_home = temp_codex_home("bootstrap-heal");
        let backup_root = codex_home.join("account_backup");
        let profile_a_dir = backup_root.join("a");
        let profile_b_dir = backup_root.join("b");
        fs::create_dir_all(&profile_a_dir).unwrap();
        fs::create_dir_all(&profile_b_dir).unwrap();
        fs::write(profile_a_dir.join("auth.json"), auth_with_account("acct_X")).unwrap();
        let b_stale = "{\"tokens\":{\"account_id\":\"acct_B\"},\"last_refresh\":\"old\"}";
        fs::write(profile_b_dir.join("auth.json"), b_stale).unwrap();
        fs::write(get_current_profile_file(Some(&codex_home)), "a\n").unwrap();
        let b_fresh = "{\"tokens\":{\"account_id\":\"acct_B\"},\"last_refresh\":\"new\"}";
        fs::write(codex_home.join("auth.json"), b_fresh).unwrap();

        let synced =
            super::sync_root_state_to_current_profile_with_home(Some(&codex_home)).unwrap();

        assert_eq!(synced.as_deref(), Some("b"));
        assert_eq!(
            fs::read_to_string(get_current_profile_file(Some(&codex_home))).unwrap(),
            "b\n"
        );
        assert_eq!(
            fs::read_to_string(profile_b_dir.join("auth.json")).unwrap(),
            b_fresh
        );
        assert_eq!(
            fs::read_to_string(profile_a_dir.join("auth.json")).unwrap(),
            auth_with_account("acct_X"),
            "the stale-marker profile must be left untouched"
        );

        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn bootstrap_sync_clears_marker_and_preserves_slots_when_live_account_unmanaged() {
        let codex_home = temp_codex_home("bootstrap-clear-unmanaged");
        let backup_root = codex_home.join("account_backup");
        let profile_a_dir = backup_root.join("a");
        fs::create_dir_all(&profile_a_dir).unwrap();
        fs::write(profile_a_dir.join("auth.json"), auth_with_account("acct_X")).unwrap();
        fs::write(get_current_profile_file(Some(&codex_home)), "a\n").unwrap();
        fs::write(profile_a_dir.join(".active_profile"), "x\n").unwrap();
        fs::write(codex_home.join("auth.json"), auth_with_account("acct_Z")).unwrap();

        let synced =
            super::sync_root_state_to_current_profile_with_home(Some(&codex_home)).unwrap();

        assert_eq!(synced, None);
        assert_eq!(
            fs::read_to_string(profile_a_dir.join("auth.json")).unwrap(),
            auth_with_account("acct_X")
        );
        assert!(!get_current_profile_file(Some(&codex_home)).exists());
        assert!(!profile_a_dir.join(".active_profile").exists());

        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn switch_profile_rejects_current_profile_marker_path_traversal() {
        let codex_home = temp_codex_home("current-profile-traversal");
        let backup_root = codex_home.join("account_backup");
        let profile_b_dir = backup_root.join("b");
        let skills_dir = codex_home.join("skills");

        fs::create_dir_all(&profile_b_dir).unwrap();
        fs::create_dir_all(&skills_dir).unwrap();
        fs::write(codex_home.join("auth.json"), "root-auth-before-switch\n").unwrap();
        fs::write(
            codex_home.join("profile.json"),
            "root-profile-before-switch\n",
        )
        .unwrap();
        fs::write(profile_b_dir.join("auth.json"), "profile-b-auth\n").unwrap();
        fs::write(profile_b_dir.join("profile.json"), "profile-b-profile\n").unwrap();
        fs::write(
            get_current_profile_file(Some(&codex_home)),
            format!("..{}skills\n", std::path::MAIN_SEPARATOR),
        )
        .unwrap();

        let hooks = FakeHooks::new(false);
        let response = switch_profile_with_home(&hooks, "b", Some(&codex_home)).unwrap();

        assert!(response.ok);
        assert_eq!(
            fs::read_to_string(codex_home.join("auth.json")).unwrap(),
            "profile-b-auth\n"
        );
        assert!(!skills_dir.join("auth.json").exists());
        assert!(!skills_dir.join("profile.json").exists());

        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn switch_profile_rejects_directory_profile_metadata_overlay() {
        let codex_home = temp_codex_home("directory-profile-metadata");
        let backup_root = codex_home.join("account_backup");
        let profile_a_dir = backup_root.join("a");
        let profile_b_dir = backup_root.join("b");
        let profile_b_metadata_dir = profile_b_dir.join("profile.json");

        fs::create_dir_all(&profile_a_dir).unwrap();
        fs::create_dir_all(&profile_b_metadata_dir).unwrap();
        fs::write(codex_home.join("auth.json"), "root-auth-before-switch\n").unwrap();
        fs::write(
            codex_home.join("profile.json"),
            "root-profile-before-switch\n",
        )
        .unwrap();
        fs::write(profile_a_dir.join("auth.json"), "profile-a-auth\n").unwrap();
        fs::write(profile_a_dir.join("profile.json"), "profile-a-profile\n").unwrap();
        fs::write(profile_b_dir.join("auth.json"), "profile-b-auth\n").unwrap();
        fs::write(profile_b_metadata_dir.join("payload"), "not metadata\n").unwrap();
        fs::write(get_current_profile_file(Some(&codex_home)), "a\n").unwrap();

        let hooks = FakeHooks::new(false);
        let error = switch_profile_with_home(&hooks, "b", Some(&codex_home)).unwrap_err();

        assert_eq!(error.error_code, "PROFILE_STATE_INVALID");
        assert_eq!(
            fs::read_to_string(codex_home.join("auth.json")).unwrap(),
            "root-auth-before-switch\n"
        );
        assert_eq!(
            fs::read_to_string(codex_home.join("profile.json")).unwrap(),
            "root-profile-before-switch\n"
        );
        assert!(codex_home.join("profile.json").is_file());

        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn switch_profile_rejects_missing_profile_auth_before_running_hooks() {
        let codex_home = temp_codex_home("missing-auth");
        let backup_root = codex_home.join("account_backup");
        fs::create_dir_all(backup_root.join("b")).unwrap();

        let hooks = FakeHooks::new(false);
        let error = switch_profile_with_home(&hooks, "b", Some(&codex_home)).unwrap_err();

        assert_eq!(error.error_code, "PROFILE_AUTH_MISSING");
        assert!(hooks.reopen_calls.lock().unwrap().is_empty());
        let _ = fs::remove_dir_all(&codex_home);
    }
}
