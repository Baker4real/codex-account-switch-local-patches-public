use std::fs;
use std::path::{Path, PathBuf};

use crate::errors::{AppError, AppResult};
use crate::models::SwitchResponse;
use crate::platform::hooks::{AppExitObservation, PlatformHooks};

use super::fs_ops::{
    atomic_write_file, autosave_auth, backup_root_state_to_profile, clear_active_markers,
    overlay_directory_contents, remove_path, set_active_marker,
};
use super::openai_usage::record_current_openai_usage_attribution;
use super::paths::{
    get_backup_root, get_codex_home, get_current_profile_file, get_openai_usage_attribution_path,
    get_profiles_index_path, is_profile_dir, list_profile_dirs, validate_profile_name,
    ACTIVE_MARKER_FILE,
};
use super::process_lock::{acquire_process_lock, ProcessLockGuard};
use super::profiles::{
    detect_unmanaged_live_account, resolve_backup_target, resolve_current_profile,
};
use super::profiles_index::reload_profiles_index_after_mutation;

#[derive(Debug, PartialEq, Eq)]
struct SwitchFileSnapshot {
    path: PathBuf,
    contents: Option<Vec<u8>>,
}

impl SwitchFileSnapshot {
    fn capture(path: PathBuf) -> AppResult<Self> {
        let contents = match fs::read(&path) {
            Ok(contents) => Some(contents),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(AppError::new(
                    "SWITCH_SNAPSHOT_FAILED",
                    format!("Failed to snapshot {}: {error}", path.display()),
                ));
            }
        };
        Ok(Self { path, contents })
    }

    fn restore(&self) -> AppResult<()> {
        match &self.contents {
            Some(contents) => atomic_write_file(&self.path, contents),
            None => remove_path(&self.path),
        }
        .map_err(|error| {
            AppError::new(
                "SWITCH_ROLLBACK_FAILED",
                format!(
                    "Failed to restore {}: {}",
                    self.path.display(),
                    error.message
                ),
            )
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
struct SwitchStateSnapshot(Vec<SwitchFileSnapshot>);

impl SwitchStateSnapshot {
    fn capture(
        codex_home: &Path,
        backup_root: &Path,
        backup_target: Option<&str>,
    ) -> AppResult<Self> {
        let mut paths = vec![
            codex_home.join("auth.json"),
            codex_home.join("profile.json"),
            codex_home.join("config.toml"),
            get_current_profile_file(Some(codex_home)),
            get_profiles_index_path(Some(codex_home)),
            get_openai_usage_attribution_path(Some(codex_home)),
        ];
        if let Some(profile) = backup_target {
            paths.extend(
                ["auth.json", "profile.json"]
                    .into_iter()
                    .map(|name| backup_root.join(profile).join(name)),
            );
        }
        paths.extend(
            list_profile_dirs(backup_root)
                .into_iter()
                .map(|profile_dir| profile_dir.join(ACTIVE_MARKER_FILE)),
        );
        paths.sort();
        paths.dedup();

        paths
            .into_iter()
            .map(SwitchFileSnapshot::capture)
            .collect::<AppResult<Vec<_>>>()
            .map(Self)
    }

    fn restore(&self) -> AppResult<()> {
        let mut failures = Vec::new();
        for snapshot in &self.0 {
            if let Err(error) = snapshot.restore() {
                failures.push(error.message);
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(AppError::new(
                "SWITCH_ROLLBACK_FAILED",
                failures.join(" | "),
            ))
        }
    }
}

fn observe_app_exit<H: PlatformHooks + ?Sized>(
    hooks: &H,
    app_was_running: &mut bool,
) -> AppResult<AppExitObservation> {
    let mut observation = AppExitObservation::default();
    let result = hooks.quit_codex_app_and_observe(&mut observation);
    *app_was_running |= observation.app_was_running;
    result?;
    Ok(observation)
}

fn capture_stable_switch_snapshot<H: PlatformHooks + ?Sized>(
    hooks: &H,
    stability_attempts: usize,
    codex_home: &Path,
    backup_root: &Path,
    app_was_running: &mut bool,
) -> AppResult<(SwitchStateSnapshot, Option<String>)> {
    if stability_attempts == 0 {
        let backup_target = resolve_backup_target(backup_root, codex_home);
        let snapshot =
            SwitchStateSnapshot::capture(codex_home, backup_root, backup_target.as_deref())?;
        return Ok((snapshot, backup_target));
    }

    for _ in 0..stability_attempts {
        let before_target = resolve_backup_target(backup_root, codex_home);
        let before =
            SwitchStateSnapshot::capture(codex_home, backup_root, before_target.as_deref())?;
        let observation = observe_app_exit(hooks, app_was_running)?;
        let after_target = resolve_backup_target(backup_root, codex_home);
        let after = SwitchStateSnapshot::capture(codex_home, backup_root, after_target.as_deref())?;
        if !observation.had_processes && before_target == after_target && before == after {
            return Ok((after, after_target));
        }
    }

    Err(AppError::new(
        "APP_RESTART_RACE",
        "Codex kept changing account state while the profile switch was preparing. Close it and retry.",
    ))
}

fn commit_profile_state<H: PlatformHooks + ?Sized>(
    hooks: &H,
    profile_name: &str,
    profile_dir: &Path,
    backup_root: &Path,
    codex_home: &Path,
) -> AppResult<()> {
    overlay_directory_contents(profile_dir, codex_home)?;
    hooks.sync_root_openai_base_url_for_profile(profile_name, Some(codex_home))?;
    set_active_marker(profile_name, backup_root)?;
    reload_profiles_index_after_mutation(Some(codex_home)).map(|_| ())
}

fn append_reopen_warnings(mut error: AppError, warnings: &[String]) -> AppError {
    if !warnings.is_empty() {
        error.message = format!("{} {}", error.message, warnings.join(" "));
    }
    error
}

pub(crate) fn acquire_switch_lock(codex_home: Option<&Path>) -> AppResult<ProcessLockGuard> {
    acquire_process_lock(
        codex_home,
        "SWITCH_IN_PROGRESS",
        "A profile switch is already in progress.",
    )
}

pub(crate) fn switch_profile_with_home_locked<H: PlatformHooks + ?Sized>(
    hooks: &H,
    profile_name: &str,
    codex_home: &Path,
) -> AppResult<SwitchResponse> {
    let backup_root = get_backup_root(Some(codex_home));
    if !backup_root.is_dir() {
        return Err(AppError::new(
            "BACKUP_ROOT_MISSING",
            format!("Backup folder not found: {}", backup_root.display()),
        ));
    }

    let profile_name = validate_profile_name(profile_name)?;
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

    let stability_attempts = hooks.profile_switch_exit_stability_attempts();
    let mut app_was_running = false;
    if let Err(error) = observe_app_exit(hooks, &mut app_was_running) {
        let warnings = hooks.reopen_codex_app_if_needed(app_was_running, Some(codex_home));
        return Err(append_reopen_warnings(error, &warnings));
    }

    let (snapshot, backup_target) = match capture_stable_switch_snapshot(
        hooks,
        stability_attempts,
        codex_home,
        &backup_root,
        &mut app_was_running,
    ) {
        Ok(stable_state) => stable_state,
        Err(error) => {
            let warnings = hooks.reopen_codex_app_if_needed(app_was_running, Some(codex_home));
            return Err(append_reopen_warnings(error, &warnings));
        }
    };

    let switch_result = (|| -> AppResult<()> {
        if stability_attempts == 0 {
            if let Some(backup_target) = backup_target.as_deref() {
                backup_root_state_to_profile(backup_target, codex_home, &backup_root)?;
            }
            autosave_auth(codex_home)?;
            commit_profile_state(hooks, &profile_name, &profile_dir, &backup_root, codex_home)?;
        } else {
            let mut committed_without_restart = false;
            for attempt in 0..stability_attempts {
                observe_app_exit(hooks, &mut app_was_running)?;
                if attempt == 0 {
                    let current_backup_target = resolve_backup_target(&backup_root, codex_home);
                    if current_backup_target != backup_target {
                        return Err(AppError::new(
                            "APP_IDENTITY_RACE",
                            "The live Codex account changed while the profile switch was preparing. Retry after the account state is stable.",
                        ));
                    }
                    if let Some(backup_target) = backup_target.as_deref() {
                        backup_root_state_to_profile(backup_target, codex_home, &backup_root)?;
                    }
                    autosave_auth(codex_home)?;
                }
                commit_profile_state(hooks, &profile_name, &profile_dir, &backup_root, codex_home)?;
                let post_commit_exit = observe_app_exit(hooks, &mut app_was_running)?;
                if !post_commit_exit.had_processes {
                    committed_without_restart = true;
                    break;
                }
            }
            if !committed_without_restart {
                return Err(AppError::new(
                    "APP_RESTART_RACE",
                    "Codex kept restarting during the profile switch. Close it and retry.",
                ));
            }
        }

        record_current_openai_usage_attribution(Some(codex_home))
    })();

    if let Err(error) = switch_result {
        let mut recovery_failures = Vec::new();
        if stability_attempts > 0 {
            if let Err(exit_error) = observe_app_exit(hooks, &mut app_was_running) {
                recovery_failures.push(format!(
                    "could not quiesce Codex before rollback: {}",
                    exit_error.message
                ));
            }
        }
        if recovery_failures.is_empty() {
            if let Err(rollback_error) = snapshot.restore() {
                recovery_failures.push(rollback_error.message);
            }
            if let Err(index_error) = reload_profiles_index_after_mutation(Some(codex_home)) {
                recovery_failures.push(format!("index recovery failed: {}", index_error.message));
            }
        }

        if !recovery_failures.is_empty() {
            return Err(AppError::new(
                "SWITCH_RECOVERY_FAILED",
                format!(
                    "Profile switch failed [{}]: {} Recovery was incomplete: {} Codex was not relaunched; close it manually before repairing the account files.",
                    error.error_code,
                    error.message,
                    recovery_failures.join(" | ")
                ),
            ));
        }

        let warnings = hooks.reopen_codex_app_if_needed(app_was_running, Some(codex_home));
        return Err(append_reopen_warnings(error, &warnings));
    }

    let warnings = hooks.reopen_codex_app_if_needed(app_was_running, Some(codex_home));

    Ok(SwitchResponse {
        ok: true,
        profile: profile_name.clone(),
        message: format!("Switched to profile: {profile_name}"),
        warnings,
    })
}

pub fn switch_profile_with_home<H: PlatformHooks + ?Sized>(
    hooks: &H,
    profile_name: &str,
    codex_home: Option<&Path>,
) -> AppResult<SwitchResponse> {
    let codex_home = codex_home.map(PathBuf::from).unwrap_or_else(get_codex_home);
    let profile_name = validate_profile_name(profile_name)?;
    let _guard = acquire_switch_lock(Some(&codex_home))?;
    switch_profile_with_home_locked(hooks, &profile_name, &codex_home)
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
    record_current_openai_usage_attribution(Some(&codex_home))?;
    Ok(Some(target))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::errors::{AppError, AppResult};
    use crate::platform::hooks::{AppExitObservation, PlatformHooks};

    use super::switch_profile_with_home;
    use crate::shared::paths::{get_current_profile_file, get_profiles_index_path};

    #[derive(Clone, Copy)]
    enum FakeQuitResult {
        Success(AppExitObservation),
        Failure(AppExitObservation),
    }

    struct FakeHooks {
        fallback_observation: AppExitObservation,
        quit_results: Mutex<VecDeque<FakeQuitResult>>,
        stability_attempts: usize,
        sync_error: bool,
        corrupt_auth_before_sync_error: bool,
        sync_calls: Mutex<usize>,
        reopen_calls: Mutex<Vec<bool>>,
        reopen_auth_contents: Mutex<Vec<Option<String>>>,
        auth_rewrite_path: Option<PathBuf>,
        auth_rewrites: Mutex<VecDeque<Option<String>>>,
    }

    impl FakeHooks {
        fn observation(app_was_running: bool, had_processes: bool) -> AppExitObservation {
            AppExitObservation {
                app_was_running,
                had_processes,
            }
        }

        fn new(app_was_running: bool) -> Self {
            Self {
                fallback_observation: AppExitObservation::default(),
                quit_results: Mutex::new(
                    [FakeQuitResult::Success(Self::observation(
                        app_was_running,
                        app_was_running,
                    ))]
                    .into_iter()
                    .collect(),
                ),
                stability_attempts: 0,
                sync_error: false,
                corrupt_auth_before_sync_error: false,
                sync_calls: Mutex::new(0),
                reopen_calls: Mutex::new(Vec::new()),
                reopen_auth_contents: Mutex::new(Vec::new()),
                auth_rewrite_path: None,
                auth_rewrites: Mutex::new(VecDeque::new()),
            }
        }

        fn failing_quit() -> Self {
            Self {
                fallback_observation: AppExitObservation::default(),
                quit_results: Mutex::new(
                    [FakeQuitResult::Failure(Self::observation(true, true))]
                        .into_iter()
                        .collect(),
                ),
                stability_attempts: 0,
                sync_error: false,
                corrupt_auth_before_sync_error: false,
                sync_calls: Mutex::new(0),
                reopen_calls: Mutex::new(Vec::new()),
                reopen_auth_contents: Mutex::new(Vec::new()),
                auth_rewrite_path: None,
                auth_rewrites: Mutex::new(VecDeque::new()),
            }
        }

        fn failing_after_quit() -> Self {
            Self {
                fallback_observation: AppExitObservation::default(),
                quit_results: Mutex::new(
                    [FakeQuitResult::Success(Self::observation(true, true))]
                        .into_iter()
                        .collect(),
                ),
                stability_attempts: 0,
                sync_error: true,
                corrupt_auth_before_sync_error: false,
                sync_calls: Mutex::new(0),
                reopen_calls: Mutex::new(Vec::new()),
                reopen_auth_contents: Mutex::new(Vec::new()),
                auth_rewrite_path: None,
                auth_rewrites: Mutex::new(VecDeque::new()),
            }
        }

        fn with_quit_observations(results: impl IntoIterator<Item = (bool, bool)>) -> Self {
            Self {
                fallback_observation: AppExitObservation::default(),
                quit_results: Mutex::new(
                    results
                        .into_iter()
                        .map(|(app_was_running, had_processes)| {
                            FakeQuitResult::Success(Self::observation(
                                app_was_running,
                                had_processes,
                            ))
                        })
                        .collect(),
                ),
                stability_attempts: 3,
                sync_error: false,
                corrupt_auth_before_sync_error: false,
                sync_calls: Mutex::new(0),
                reopen_calls: Mutex::new(Vec::new()),
                reopen_auth_contents: Mutex::new(Vec::new()),
                auth_rewrite_path: None,
                auth_rewrites: Mutex::new(VecDeque::new()),
            }
        }

        fn failing_during_commit_exit() -> Self {
            Self {
                fallback_observation: AppExitObservation::default(),
                quit_results: Mutex::new(
                    [
                        FakeQuitResult::Success(Self::observation(true, true)),
                        FakeQuitResult::Success(Self::observation(false, false)),
                        FakeQuitResult::Success(Self::observation(false, false)),
                        FakeQuitResult::Failure(Self::observation(false, true)),
                        FakeQuitResult::Success(Self::observation(false, false)),
                    ]
                    .into_iter()
                    .collect(),
                ),
                stability_attempts: 3,
                sync_error: false,
                corrupt_auth_before_sync_error: false,
                sync_calls: Mutex::new(0),
                reopen_calls: Mutex::new(Vec::new()),
                reopen_auth_contents: Mutex::new(Vec::new()),
                auth_rewrite_path: None,
                auth_rewrites: Mutex::new(VecDeque::new()),
            }
        }

        fn failing_with_unrecoverable_auth() -> Self {
            Self {
                fallback_observation: AppExitObservation::default(),
                quit_results: Mutex::new(
                    [FakeQuitResult::Success(Self::observation(true, true))]
                        .into_iter()
                        .collect(),
                ),
                stability_attempts: 0,
                sync_error: true,
                corrupt_auth_before_sync_error: true,
                sync_calls: Mutex::new(0),
                reopen_calls: Mutex::new(Vec::new()),
                reopen_auth_contents: Mutex::new(Vec::new()),
                auth_rewrite_path: None,
                auth_rewrites: Mutex::new(VecDeque::new()),
            }
        }

        fn with_identity_change(codex_home: &Path, rewritten_auth: String) -> Self {
            Self {
                fallback_observation: AppExitObservation::default(),
                quit_results: Mutex::new(
                    (0..5)
                        .map(|_| FakeQuitResult::Success(Self::observation(false, false)))
                        .collect(),
                ),
                stability_attempts: 3,
                sync_error: false,
                corrupt_auth_before_sync_error: false,
                sync_calls: Mutex::new(0),
                reopen_calls: Mutex::new(Vec::new()),
                reopen_auth_contents: Mutex::new(Vec::new()),
                auth_rewrite_path: Some(codex_home.join("auth.json")),
                auth_rewrites: Mutex::new(
                    [None, Some(rewritten_auth), None, None, None]
                        .into_iter()
                        .collect(),
                ),
            }
        }
    }

    impl PlatformHooks for FakeHooks {
        fn open_or_activate_codex_app(&self, _codex_home: Option<&Path>) -> AppResult<String> {
            unreachable!("not used in switch_core tests")
        }

        fn quit_codex_app_if_running(&self) -> AppResult<bool> {
            Ok(self.fallback_observation.app_was_running)
        }

        fn quit_codex_app_and_observe(
            &self,
            observation: &mut AppExitObservation,
        ) -> AppResult<()> {
            let result = self
                .quit_results
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(FakeQuitResult::Success(self.fallback_observation));
            let scripted_observation = match result {
                FakeQuitResult::Success(scripted) | FakeQuitResult::Failure(scripted) => scripted,
            };
            observation.app_was_running |= scripted_observation.app_was_running;
            observation.had_processes |= scripted_observation.had_processes;
            if let Some(Some(contents)) = self.auth_rewrites.lock().unwrap().pop_front() {
                fs::write(self.auth_rewrite_path.as_ref().unwrap(), contents).unwrap();
            }
            match result {
                FakeQuitResult::Success(_) => Ok(()),
                FakeQuitResult::Failure(_) => Err(AppError::new(
                    "TEST_APP_EXIT_UNCONFIRMED",
                    "scripted process query failure",
                )),
            }
        }

        fn profile_switch_exit_stability_attempts(&self) -> usize {
            self.stability_attempts
        }

        fn reopen_codex_app_if_needed(
            &self,
            app_was_running: bool,
            codex_home: Option<&Path>,
        ) -> Vec<String> {
            self.reopen_calls.lock().unwrap().push(app_was_running);
            self.reopen_auth_contents
                .lock()
                .unwrap()
                .push(codex_home.and_then(|home| fs::read_to_string(home.join("auth.json")).ok()));
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

        fn sync_root_openai_base_url_for_profile(
            &self,
            profile_name: &str,
            codex_home: Option<&Path>,
        ) -> AppResult<()> {
            *self.sync_calls.lock().unwrap() += 1;
            if self.sync_error {
                if self.corrupt_auth_before_sync_error {
                    if let Some(codex_home) = codex_home {
                        let auth_path = codex_home.join("auth.json");
                        let _ = fs::remove_file(&auth_path);
                        fs::create_dir(&auth_path).unwrap();
                    }
                }
                return Err(AppError::new(
                    "TEST_POST_QUIT_FAILURE",
                    "scripted post-quit failure",
                ));
            }
            crate::shared::config::sync_root_openai_base_url_for_profile(profile_name, codex_home)
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
        fs::write(profile_a_dir.join(".active_profile"), "active\n").unwrap();

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
    fn switch_profile_keeps_auth_and_markers_when_app_exit_cannot_be_confirmed() {
        let codex_home = temp_codex_home("switch-exit-unconfirmed");
        let backup_root = codex_home.join("account_backup");
        let profile_a_dir = backup_root.join("a");
        let profile_b_dir = backup_root.join("b");

        fs::create_dir_all(&profile_a_dir).unwrap();
        fs::create_dir_all(&profile_b_dir).unwrap();
        fs::write(codex_home.join("auth.json"), "root-auth-before-switch\n").unwrap();
        fs::write(profile_a_dir.join("auth.json"), "profile-a-auth\n").unwrap();
        fs::write(profile_b_dir.join("auth.json"), "profile-b-auth\n").unwrap();
        fs::write(get_current_profile_file(Some(&codex_home)), "a\n").unwrap();
        fs::write(profile_a_dir.join(".active_profile"), "active\n").unwrap();

        let hooks = FakeHooks::failing_quit();
        let error = switch_profile_with_home(&hooks, "b", Some(&codex_home)).unwrap_err();

        assert_eq!(error.error_code, "TEST_APP_EXIT_UNCONFIRMED");
        assert_eq!(
            fs::read_to_string(codex_home.join("auth.json")).unwrap(),
            "root-auth-before-switch\n"
        );
        assert_eq!(
            fs::read_to_string(profile_a_dir.join("auth.json")).unwrap(),
            "profile-a-auth\n"
        );
        assert_eq!(
            fs::read_to_string(profile_b_dir.join("auth.json")).unwrap(),
            "profile-b-auth\n"
        );
        assert_eq!(
            fs::read_to_string(get_current_profile_file(Some(&codex_home))).unwrap(),
            "a\n"
        );
        assert!(profile_a_dir.join(".active_profile").is_file());
        assert!(!profile_b_dir.join(".active_profile").exists());
        assert_eq!(*hooks.reopen_calls.lock().unwrap(), vec![true]);
        assert_eq!(
            *hooks.reopen_auth_contents.lock().unwrap(),
            vec![Some("root-auth-before-switch\n".to_string())]
        );

        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn switch_profile_reopens_codex_when_a_post_quit_step_fails() {
        let codex_home = temp_codex_home("switch-post-quit-failure");
        let backup_root = codex_home.join("account_backup");
        let profile_a_dir = backup_root.join("a");
        let profile_b_dir = backup_root.join("b");

        fs::create_dir_all(&profile_a_dir).unwrap();
        fs::create_dir_all(&profile_b_dir).unwrap();
        fs::write(codex_home.join("auth.json"), "root-auth-before-switch\n").unwrap();
        fs::write(profile_a_dir.join("auth.json"), "profile-a-auth\n").unwrap();
        fs::write(profile_b_dir.join("auth.json"), "profile-b-auth\n").unwrap();
        fs::write(get_current_profile_file(Some(&codex_home)), "a\n").unwrap();
        fs::write(profile_a_dir.join(".active_profile"), "active\n").unwrap();

        let hooks = FakeHooks::failing_after_quit();
        let error = switch_profile_with_home(&hooks, "b", Some(&codex_home)).unwrap_err();

        assert_eq!(error.error_code, "TEST_POST_QUIT_FAILURE");
        assert_eq!(*hooks.reopen_calls.lock().unwrap(), vec![true]);
        assert_eq!(
            fs::read_to_string(codex_home.join("auth.json")).unwrap(),
            "root-auth-before-switch\n"
        );
        assert_eq!(
            fs::read_to_string(profile_a_dir.join("auth.json")).unwrap(),
            "profile-a-auth\n"
        );
        assert_eq!(
            fs::read_to_string(get_current_profile_file(Some(&codex_home))).unwrap(),
            "a\n"
        );
        assert!(profile_a_dir.join(".active_profile").is_file());
        assert!(!profile_b_dir.join(".active_profile").exists());
        assert_eq!(
            *hooks.reopen_auth_contents.lock().unwrap(),
            vec![Some("root-auth-before-switch\n".to_string())]
        );

        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn switch_profile_rolls_back_before_reopening_after_commit_exit_failure() {
        let codex_home = temp_codex_home("switch-commit-exit-failure");
        let backup_root = codex_home.join("account_backup");
        let profile_a_dir = backup_root.join("a");
        let profile_b_dir = backup_root.join("b");

        fs::create_dir_all(&profile_a_dir).unwrap();
        fs::create_dir_all(&profile_b_dir).unwrap();
        fs::write(codex_home.join("auth.json"), "root-auth-before-switch\n").unwrap();
        fs::write(profile_a_dir.join("auth.json"), "profile-a-auth\n").unwrap();
        fs::write(profile_b_dir.join("auth.json"), "profile-b-auth\n").unwrap();
        fs::write(get_current_profile_file(Some(&codex_home)), "a\n").unwrap();
        fs::write(profile_a_dir.join(".active_profile"), "active\n").unwrap();

        let hooks = FakeHooks::failing_during_commit_exit();
        let error = switch_profile_with_home(&hooks, "b", Some(&codex_home)).unwrap_err();

        assert_eq!(error.error_code, "TEST_APP_EXIT_UNCONFIRMED");
        assert_eq!(*hooks.reopen_calls.lock().unwrap(), vec![true]);
        assert_eq!(
            *hooks.reopen_auth_contents.lock().unwrap(),
            vec![Some("root-auth-before-switch\n".to_string())]
        );
        assert_eq!(
            fs::read_to_string(codex_home.join("auth.json")).unwrap(),
            "root-auth-before-switch\n"
        );
        assert_eq!(
            fs::read_to_string(profile_a_dir.join("auth.json")).unwrap(),
            "profile-a-auth\n"
        );
        assert!(profile_a_dir.join(".active_profile").is_file());
        assert!(!profile_b_dir.join(".active_profile").exists());

        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn switch_profile_does_not_reopen_when_rollback_is_incomplete() {
        let codex_home = temp_codex_home("switch-rollback-failure");
        let backup_root = codex_home.join("account_backup");
        let profile_a_dir = backup_root.join("a");
        let profile_b_dir = backup_root.join("b");

        fs::create_dir_all(&profile_a_dir).unwrap();
        fs::create_dir_all(&profile_b_dir).unwrap();
        fs::write(codex_home.join("auth.json"), "root-auth-before-switch\n").unwrap();
        fs::write(profile_a_dir.join("auth.json"), "profile-a-auth\n").unwrap();
        fs::write(profile_b_dir.join("auth.json"), "profile-b-auth\n").unwrap();
        fs::write(get_current_profile_file(Some(&codex_home)), "a\n").unwrap();

        let hooks = FakeHooks::failing_with_unrecoverable_auth();
        let error = switch_profile_with_home(&hooks, "b", Some(&codex_home)).unwrap_err();

        assert_eq!(error.error_code, "SWITCH_RECOVERY_FAILED");
        assert!(error.message.contains("TEST_POST_QUIT_FAILURE"));
        assert!(hooks.reopen_calls.lock().unwrap().is_empty());
        assert!(codex_home.join("auth.json").is_dir());

        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn switch_profile_rescans_for_restarts_before_and_after_live_state_mutation() {
        let codex_home = temp_codex_home("switch-restart-race");
        let backup_root = codex_home.join("account_backup");
        let profile_a_dir = backup_root.join("a");
        let profile_b_dir = backup_root.join("b");

        fs::create_dir_all(&profile_a_dir).unwrap();
        fs::create_dir_all(&profile_b_dir).unwrap();
        fs::write(codex_home.join("auth.json"), "root-auth-before-switch\n").unwrap();
        fs::write(profile_a_dir.join("auth.json"), "profile-a-auth\n").unwrap();
        fs::write(profile_b_dir.join("auth.json"), "profile-b-auth\n").unwrap();
        fs::write(get_current_profile_file(Some(&codex_home)), "a\n").unwrap();

        let hooks = FakeHooks::with_quit_observations([
            (false, false),
            (true, true),
            (false, false),
            (false, false),
            (false, false),
        ]);
        let response = switch_profile_with_home(&hooks, "b", Some(&codex_home)).unwrap();

        assert!(response.ok);
        assert!(hooks.quit_results.lock().unwrap().is_empty());
        assert_eq!(*hooks.reopen_calls.lock().unwrap(), vec![true]);

        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn switch_profile_reapplies_live_state_when_codex_restarts_during_commit() {
        let codex_home = temp_codex_home("switch-reapply-after-restart");
        let backup_root = codex_home.join("account_backup");
        let profile_a_dir = backup_root.join("a");
        let profile_b_dir = backup_root.join("b");

        fs::create_dir_all(&profile_a_dir).unwrap();
        fs::create_dir_all(&profile_b_dir).unwrap();
        fs::write(codex_home.join("auth.json"), "root-auth-before-switch\n").unwrap();
        fs::write(profile_a_dir.join("auth.json"), "profile-a-auth\n").unwrap();
        fs::write(profile_b_dir.join("auth.json"), "profile-b-auth\n").unwrap();
        fs::write(get_current_profile_file(Some(&codex_home)), "a\n").unwrap();

        let hooks = FakeHooks::with_quit_observations([
            (false, false),
            (false, false),
            (false, false),
            (false, true),
            (false, false),
            (false, false),
        ]);
        let response = switch_profile_with_home(&hooks, "b", Some(&codex_home)).unwrap();

        assert!(response.ok);
        assert!(hooks.quit_results.lock().unwrap().is_empty());
        assert_eq!(*hooks.sync_calls.lock().unwrap(), 2);
        assert_eq!(
            fs::read_to_string(codex_home.join("auth.json")).unwrap(),
            "profile-b-auth\n"
        );

        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn switch_profile_binds_backup_target_to_the_final_stable_identity() {
        let codex_home = temp_codex_home("switch-stable-identity-route");
        let backup_root = codex_home.join("account_backup");
        let profile_a_dir = backup_root.join("a");
        let profile_b_dir = backup_root.join("b");
        let profile_c_dir = backup_root.join("c");
        let auth_a = auth_with_account("acct_A");
        let auth_b_old =
            r#"{"tokens":{"account_id":"acct_B"},"revision":"slot-b-old"}"#.to_string();
        let auth_b_live =
            r#"{"tokens":{"account_id":"acct_B"},"revision":"root-b-new"}"#.to_string();
        let auth_c = auth_with_account("acct_C");

        fs::create_dir_all(&profile_a_dir).unwrap();
        fs::create_dir_all(&profile_b_dir).unwrap();
        fs::create_dir_all(&profile_c_dir).unwrap();
        fs::write(codex_home.join("auth.json"), &auth_a).unwrap();
        fs::write(profile_a_dir.join("auth.json"), &auth_a).unwrap();
        fs::write(profile_b_dir.join("auth.json"), &auth_b_old).unwrap();
        fs::write(profile_c_dir.join("auth.json"), &auth_c).unwrap();
        fs::write(get_current_profile_file(Some(&codex_home)), "a\n").unwrap();
        fs::write(profile_a_dir.join(".active_profile"), "active\n").unwrap();

        let hooks = FakeHooks::with_identity_change(&codex_home, auth_b_live.clone());
        let response = switch_profile_with_home(&hooks, "c", Some(&codex_home)).unwrap();

        assert!(response.ok);
        assert_eq!(
            fs::read_to_string(profile_a_dir.join("auth.json")).unwrap(),
            auth_a
        );
        assert_eq!(
            fs::read_to_string(profile_b_dir.join("auth.json")).unwrap(),
            auth_b_live
        );
        assert_eq!(
            fs::read_to_string(codex_home.join("auth.json")).unwrap(),
            auth_c
        );
        assert_eq!(
            fs::read_to_string(get_current_profile_file(Some(&codex_home))).unwrap(),
            "c\n"
        );
        assert!(hooks.quit_results.lock().unwrap().is_empty());

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
