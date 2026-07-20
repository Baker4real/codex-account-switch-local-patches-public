use std::path::Path;

use crate::errors::AppResult;
use crate::shared::codex_app_server::{AppServerSnapshot, DeviceCodePrompt};
use crate::shared::process_lock::ProcessLockGuard;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AppExitObservation {
    pub app_was_running: bool,
    pub had_processes: bool,
}

pub trait PlatformHooks: Send + Sync {
    fn open_or_activate_codex_app(&self, codex_home: Option<&Path>) -> AppResult<String>;
    fn quit_codex_app_if_running(&self) -> AppResult<bool>;
    fn quit_codex_app_and_observe(&self, observation: &mut AppExitObservation) -> AppResult<()> {
        let app_was_running = self.quit_codex_app_if_running()?;
        observation.app_was_running |= app_was_running;
        observation.had_processes |= app_was_running;
        Ok(())
    }
    fn profile_switch_exit_stability_attempts(&self) -> usize {
        0
    }
    fn reopen_codex_app_if_needed(
        &self,
        app_was_running: bool,
        codex_home: Option<&Path>,
    ) -> Vec<String>;
    /// Run `codex login`. `cli_codex_home` is the live `~/.codex` and
    /// drives codex-binary resolution (so the user's managed shim is
    /// filtered out correctly). `runtime_codex_home` is what the spawned
    /// codex sees as `CODEX_HOME` — for the legacy "log in as the
    /// active profile" flow they are the same, but the per-card login
    /// flow points it at a sandboxed sibling.
    fn run_codex_login(
        &self,
        cli_codex_home: &Path,
        runtime_codex_home: &Path,
        profile_name: Option<&str>,
    ) -> AppResult<()>;
    fn run_codex_device_code_login(
        &self,
        _cli_codex_home: &Path,
        _runtime_codex_home: &Path,
        _profile_name: Option<&str>,
        _show_device_code: &mut dyn FnMut(&DeviceCodePrompt) -> AppResult<()>,
    ) -> AppResult<()> {
        Err(crate::errors::AppError::new(
            "DEVICE_CODE_LOGIN_UNSUPPORTED",
            "Device-code login is not supported on this platform.",
        ))
    }
    /// Drive `codex app-server` to fetch the live account plan + rate
    /// limits without paying for an LLM round-trip. Replaces the
    /// historical `codex exec "Reply with the single word OK."` hack
    /// that would burn user quota for ~30–90 s on every refresh
    /// fallback. Implementations resolve the real codex binary via
    /// `cli_codex_home` and point the spawned child at
    /// `runtime_codex_home` as its `CODEX_HOME` (sandboxed sibling),
    /// matching the existing login/refresh isolation model.
    fn fetch_account_via_app_server(
        &self,
        cli_codex_home: &Path,
        runtime_codex_home: &Path,
    ) -> AppResult<AppServerSnapshot>;
    fn sync_root_openai_base_url_for_profile(
        &self,
        profile_name: &str,
        codex_home: Option<&Path>,
    ) -> AppResult<()> {
        crate::shared::config::sync_root_openai_base_url_for_profile(profile_name, codex_home)
    }
    fn sync_root_openai_base_url_for_current_profile(
        &self,
        codex_home: Option<&Path>,
    ) -> AppResult<()> {
        let codex_home = codex_home
            .map(Path::to_path_buf)
            .unwrap_or_else(crate::shared::paths::get_codex_home);
        let backup_root = crate::shared::paths::get_backup_root(Some(&codex_home));
        let Some(current_profile) = crate::shared::profiles::resolve_current_profile(&backup_root)
        else {
            return Ok(());
        };

        self.sync_root_openai_base_url_for_profile(&current_profile, Some(&codex_home))
    }
    fn acquire_background_refresh_lock(
        &self,
        _codex_home: &Path,
    ) -> AppResult<Option<ProcessLockGuard>> {
        Ok(None)
    }
    fn sync_live_auth_if_current_profile(
        &self,
        _profile_name: &str,
        _codex_home: &Path,
        _refreshed_auth_path: &Path,
    ) -> AppResult<()> {
        Ok(())
    }
    fn sync_on_window_close(&self) -> AppResult<()>;
}
