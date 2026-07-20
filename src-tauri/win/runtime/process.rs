use std::cmp::Ordering;
use std::env;
use std::ffi::OsString;
use std::fs;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

#[cfg(target_os = "windows")]
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, APPMODEL_ERROR_NO_PACKAGE, ERROR_INSUFFICIENT_BUFFER,
    ERROR_INVALID_PARAMETER, HANDLE, HWND, LPARAM, WAIT_OBJECT_0,
};
#[cfg(target_os = "windows")]
use windows_sys::Win32::Storage::FileSystem::SYNCHRONIZE;
#[cfg(target_os = "windows")]
use windows_sys::Win32::Storage::Packaging::Appx::GetPackageFullName;
#[cfg(target_os = "windows")]
use windows_sys::Win32::System::Threading::{
    OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_TERMINATE,
};
#[cfg(target_os = "windows")]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, PostMessageW, WM_CLOSE,
};

use crate::errors::{AppError, AppResult};
use crate::models::CodexCliCandidate;
use crate::platform::hooks::{AppExitObservation, PlatformHooks};
use crate::shared::codex_app_server::{
    fetch_account_snapshot, run_chatgpt_browser_login, run_chatgpt_device_code_login,
    AppServerSnapshot, DeviceCodePrompt,
};
use crate::shared::codex_cli_path::CodexPathResolver;
pub use crate::shared::codex_cli_path::{InstallState, RealCodexPathSource};
use crate::shared::dedicated_proxy::configure_child_process;
use crate::shared::fs_ops::atomic_write_file;
use crate::shared::login_cancel::wait_for_login_or_cancel;
use crate::shared::metadata::load_profile_metadata;

use super::paths::{get_codex_home, get_install_state_file};

const WINDOWS_STORE_PACKAGE_PREFIX: &str = "OpenAI.Codex_";
const WINDOWS_STORE_PACKAGE_SUFFIX: &str = "__2p2nqsd0c76g0";
const WINDOWS_INVOKABLE_SUFFIXES: [&str; 4] = ["cmd", "exe", "bat", "com"];
const WINDOWS_APPS_PATH_SEGMENT: &str = r"\microsoft\windowsapps\";
const WINDOWS_STORE_APP_ID: &str = "OpenAI.Codex_2p2nqsd0c76g0!App";
const WINDOWS_STORE_SHELL_PREFIX: &str = r"shell:AppsFolder\";
const WHERE_CODEX_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_VERSION_MANAGER_DIRECTORIES: usize = 32;
const PROFILE_SWITCH_EXIT_STABILITY_ATTEMPTS: usize = 3;
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;
static WINDOWS_APP_TARGET_CACHE: OnceLock<String> = OnceLock::new();
static WINDOWS_PLATFORM_HOOKS: WindowsPlatformHooks = WindowsPlatformHooks;

#[derive(Debug, Clone, PartialEq, Eq)]
enum AppLaunchTarget {
    WindowsStore(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoginBrowser {
    Edge,
}

#[derive(Debug, Clone)]
struct LoginBrowserLaunch {
    browser: LoginBrowser,
    executable: PathBuf,
}

pub struct WindowsPlatformHooks;

pub fn platform_hooks() -> &'static dyn PlatformHooks {
    &WINDOWS_PLATFORM_HOOKS
}

pub fn load_install_state(codex_home: Option<&Path>) -> InstallState {
    let path = get_install_state_file(codex_home);
    let raw = match fs::read_to_string(path) {
        Ok(value) => value,
        Err(_) => return InstallState::default(),
    };

    serde_json::from_str(&raw).unwrap_or_default()
}

pub(super) fn save_install_state(codex_home: Option<&Path>, state: &InstallState) {
    let path = get_install_state_file(codex_home);
    let Ok(serialized) = serde_json::to_string_pretty(state) else {
        return;
    };
    let _ = atomic_write_file(&path, format!("{serialized}\n"));
}

fn normalize_windows_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase()
}

fn paths_match(left: &Path, right: &Path) -> bool {
    normalize_windows_path(left) == normalize_windows_path(right)
}

fn is_windows_apps_alias_path(path: &Path) -> bool {
    normalize_windows_path(path).contains(WINDOWS_APPS_PATH_SEGMENT)
}

pub(super) fn is_acceptable_real_codex_cli_path(
    path: &Path,
    managed_shim_path: Option<&Path>,
) -> bool {
    if managed_shim_path.is_some_and(|managed_shim| paths_match(path, managed_shim)) {
        return false;
    }

    !is_windows_apps_alias_path(path)
}

fn version_directory_sort_key(path: &Path) -> Option<(u64, u64, u64)> {
    let name = path.file_name()?.to_string_lossy();
    let name = name.strip_prefix('v').unwrap_or(&name);
    let mut parts = name.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.split('-').next()?.parse().ok()?;
    Some((major, minor, patch))
}

fn collect_version_directories_newest_first(root: &Path) -> Vec<PathBuf> {
    let mut directories = fs::read_dir(root)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();

    directories.sort_by(|left, right| {
        match (
            version_directory_sort_key(left),
            version_directory_sort_key(right),
        ) {
            (Some(left), Some(right)) => right.cmp(&left),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => right
                .to_string_lossy()
                .to_ascii_lowercase()
                .cmp(&left.to_string_lossy().to_ascii_lowercase()),
        }
    });
    directories.truncate(MAX_VERSION_MANAGER_DIRECTORIES);
    directories
}

fn push_codex_name_candidates<F>(root: &Path, push: &mut F)
where
    F: FnMut(PathBuf),
{
    for name in ["codex.cmd", "codex.exe", "codex.bat", "codex.com"] {
        push(root.join(name));
    }
}

fn push_versioned_install_candidates<F>(version_root: &Path, install_suffix: &Path, push: &mut F)
where
    F: FnMut(PathBuf),
{
    for version_dir in collect_version_directories_newest_first(version_root) {
        let install_dir = if install_suffix.as_os_str().is_empty() {
            version_dir
        } else {
            version_dir.join(install_suffix)
        };
        push_codex_name_candidates(&install_dir, push);
        push_codex_name_candidates(&install_dir.join("node_modules").join(".bin"), push);
    }
}

fn push_nvm_codex_candidates<F>(
    nvm_home: Option<OsString>,
    nvm_dir: Option<OsString>,
    nvm_bin: Option<OsString>,
    nvm_symlink: Option<OsString>,
    push: &mut F,
) where
    F: FnMut(PathBuf),
{
    if let Some(nvm_bin) = nvm_bin.filter(|value| !value.is_empty()) {
        push_codex_name_candidates(&PathBuf::from(nvm_bin), push);
    }
    if let Some(nvm_symlink) = nvm_symlink.filter(|value| !value.is_empty()) {
        push_codex_name_candidates(&PathBuf::from(nvm_symlink), push);
    }
    if let Some(nvm_home) = nvm_home
        .filter(|value| !value.is_empty())
        .or_else(|| nvm_dir.filter(|value| !value.is_empty()))
    {
        push_versioned_install_candidates(&PathBuf::from(nvm_home), Path::new(""), push);
    }
}

fn push_fnm_codex_candidates<F>(
    fnm_dir: Option<OsString>,
    fnm_multishell_path: Option<OsString>,
    local_app_data: Option<OsString>,
    user_profile: Option<OsString>,
    push: &mut F,
) where
    F: FnMut(PathBuf),
{
    if let Some(fnm_multishell_path) = fnm_multishell_path.filter(|value| !value.is_empty()) {
        push_codex_name_candidates(&PathBuf::from(fnm_multishell_path), push);
    }
    let root = fnm_dir
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            local_app_data
                .filter(|value| !value.is_empty())
                .map(|value| PathBuf::from(value).join("fnm"))
        })
        .or_else(|| {
            user_profile
                .filter(|value| !value.is_empty())
                .map(|value| PathBuf::from(value).join(".fnm"))
        });
    if let Some(root) = root {
        push_versioned_install_candidates(
            &root.join("node-versions"),
            Path::new("installation"),
            push,
        );
    }
}

fn push_asdf_codex_candidates<F>(
    asdf_data_dir: Option<OsString>,
    user_profile: Option<OsString>,
    push: &mut F,
) where
    F: FnMut(PathBuf),
{
    let root = asdf_data_dir
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            user_profile
                .filter(|value| !value.is_empty())
                .map(|value| PathBuf::from(value).join(".asdf"))
        });
    if let Some(root) = root {
        push_versioned_install_candidates(
            &root.join("installs").join("nodejs"),
            Path::new("bin"),
            push,
        );
    }
}

pub(super) fn resolve_windows_invokable_path(path: &Path) -> Option<PathBuf> {
    let extension = path.extension().and_then(|value| value.to_str());
    if let Some(extension) = extension {
        return WINDOWS_INVOKABLE_SUFFIXES
            .iter()
            .any(|suffix| extension.eq_ignore_ascii_case(suffix))
            .then(|| path.is_file().then(|| path.to_path_buf()))
            .flatten();
    }

    for suffix in WINDOWS_INVOKABLE_SUFFIXES {
        let candidate = path.with_extension(suffix);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

fn push_real_codex_candidate(
    candidates: &mut Vec<PathBuf>,
    path: PathBuf,
    managed_shim_path: Option<&Path>,
) {
    let Some(resolved_path) = resolve_windows_invokable_path(&path) else {
        return;
    };
    if !is_acceptable_real_codex_cli_path(&resolved_path, managed_shim_path) {
        return;
    }
    push_candidate(candidates, resolved_path);
}

fn push_candidate(candidates: &mut Vec<PathBuf>, path: PathBuf) {
    if !candidates.iter().any(|existing| existing == &path) {
        candidates.push(path);
    }
}

fn push_versioned_codex_bins<F>(bin_root: PathBuf, push: &mut F)
where
    F: FnMut(PathBuf),
{
    let Ok(entries) = fs::read_dir(bin_root) else {
        return;
    };
    let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
    entries.sort_by_key(|entry| {
        entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
    });
    entries.reverse();

    for entry in entries {
        if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            push(entry.path().join("codex.exe"));
        }
    }
}

fn push_official_codex_cli_candidates<F>(push: &mut F)
where
    F: FnMut(PathBuf),
{
    if let Some(codex_exe) = env::var_os("CODEX_EXE") {
        push(PathBuf::from(codex_exe));
    }

    if let Some(install_dir) = env::var_os("CODEX_INSTALL_DIR") {
        let install_dir = PathBuf::from(install_dir);
        push(install_dir.join("codex.exe"));
        push_versioned_codex_bins(install_dir, push);
    }

    if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
        let bin_root = PathBuf::from(local_app_data)
            .join("OpenAI")
            .join("Codex")
            .join("bin");
        push(bin_root.join("codex.exe"));
        push_versioned_codex_bins(bin_root, push);
    }
}

fn managed_codex_shim_path(codex_home: Option<&Path>) -> PathBuf {
    codex_home
        .map(Path::to_path_buf)
        .unwrap_or_else(get_codex_home)
        .join("bin")
        .join("codex.cmd")
}

pub(super) fn hide_console_window(command: &mut Command) -> &mut Command {
    #[cfg(target_os = "windows")]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }

    command
}

fn where_command_program() -> OsString {
    #[cfg(test)]
    if let Some(path) = env::var_os("CODEX_SWITCH_TEST_WHERE_EXE") {
        return path;
    }

    OsString::from("where")
}

fn where_codex_candidate_paths_with_timeout(timeout: Duration) -> Vec<PathBuf> {
    let mut command = Command::new(where_command_program());
    command.arg("codex");
    hide_console_window(&mut command);

    let Some(stdout) =
        crate::shared::codex_cli_path::run_capturing_stdout_with_timeout(command, timeout)
    else {
        return Vec::new();
    };

    stdout
        .lines()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .collect()
}

pub(super) fn discover_real_codex_cli_path(managed_shim_path: Option<&Path>) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    let mut push_candidate_path = |path: PathBuf| {
        push_real_codex_candidate(&mut candidates, path, managed_shim_path);
    };

    push_official_codex_cli_candidates(&mut push_candidate_path);

    if cfg!(target_os = "windows") {
        for path in where_codex_candidate_paths_with_timeout(WHERE_CODEX_TIMEOUT) {
            push_candidate_path(path);
        }
    }

    if let Some(path) = env::var_os("PATH") {
        for entry in env::split_paths(&path) {
            let candidate = if cfg!(target_os = "windows") {
                entry.join("codex")
            } else {
                entry.join("codex")
            };
            push_candidate_path(candidate);
        }
    }

    candidates
        .into_iter()
        .find(|path| probe_codex_version(path).is_some())
}

fn windows_store_shell_target(app_id: &str) -> String {
    format!("{WINDOWS_STORE_SHELL_PREFIX}{app_id}")
}

fn is_valid_windows_store_app_id(app_id: &str) -> bool {
    let trimmed = app_id.trim();
    trimmed.starts_with("OpenAI.Codex_") && trimmed.ends_with("!App")
}

fn detect_windows_store_app_target() -> Option<String> {
    if !cfg!(target_os = "windows") {
        return None;
    }

    let script = format!(
        "$package = Get-AppxPackage -Name 'OpenAI.Codex' -ErrorAction SilentlyContinue; \
         if ($package) {{ \
           $appId = Get-StartApps | Where-Object {{ $_.AppID -like 'OpenAI.Codex*' }} | Select-Object -First 1 -ExpandProperty AppID; \
           if ($appId) {{ $appId }} else {{ '{WINDOWS_STORE_APP_ID}' }} \
         }}"
    );
    let mut command = Command::new("powershell");
    command.args(["-NoProfile", "-Command", &script]);
    let output = hide_console_window(&mut command).output().ok()?;
    if !output.status.success() {
        return None;
    }

    let app_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    is_valid_windows_store_app_id(&app_id).then(|| windows_store_shell_target(&app_id))
}

fn resolve_windows_store_shell_target() -> String {
    WINDOWS_APP_TARGET_CACHE
        .get_or_init(|| {
            detect_windows_store_app_target()
                .unwrap_or_else(|| windows_store_shell_target(WINDOWS_STORE_APP_ID))
        })
        .clone()
}

fn resolve_windows_app_target() -> AppLaunchTarget {
    AppLaunchTarget::WindowsStore(resolve_windows_store_shell_target())
}

fn is_windows_store_codex_package(package_name: &str) -> bool {
    let package_name = package_name.to_ascii_lowercase();
    package_name.starts_with(&WINDOWS_STORE_PACKAGE_PREFIX.to_ascii_lowercase())
        && package_name.ends_with(&WINDOWS_STORE_PACKAGE_SUFFIX.to_ascii_lowercase())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WindowsStoreProcess {
    image_name: String,
    process_id: u32,
}

impl WindowsStoreProcess {
    fn new(image_name: impl Into<String>, process_id: u32) -> Self {
        Self {
            image_name: image_name.into(),
            process_id,
        }
    }

    fn is_ui_host(&self) -> bool {
        self.image_name
            .split_ascii_whitespace()
            .next()
            .unwrap_or_default()
            .eq_ignore_ascii_case("ChatGPT.exe")
    }
}

trait AppProcessOps {
    fn snapshot(&mut self) -> AppResult<Vec<WindowsStoreProcess>>;
    fn terminate(&mut self, process_ids: &[u32], force: bool) -> AppResult<()>;
    fn sleep(&mut self, duration: Duration);
}

fn parse_windows_store_processes(stdout: &str) -> AppResult<Vec<WindowsStoreProcess>> {
    let mut processes = Vec::new();

    for line in stdout.lines() {
        let line = line.trim().trim_start_matches('\u{feff}');
        let contains_codex_package = {
            let line = line.to_ascii_lowercase();
            line.contains(&WINDOWS_STORE_PACKAGE_PREFIX.to_ascii_lowercase())
                && line.contains(&WINDOWS_STORE_PACKAGE_SUFFIX.to_ascii_lowercase())
        };
        let Some(row) = line
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
        else {
            if contains_codex_package {
                return Err(AppError::new(
                    "APP_PROCESS_QUERY_FAILED",
                    "Codex Store process row was not valid tasklist CSV.",
                ));
            }
            continue;
        };
        let fields = row.split("\",\"").collect::<Vec<_>>();
        let Some(package_name) = fields.last() else {
            if contains_codex_package {
                return Err(AppError::new(
                    "APP_PROCESS_QUERY_FAILED",
                    "Codex Store process row did not contain package metadata.",
                ));
            }
            continue;
        };
        if !is_windows_store_codex_package(package_name) {
            if contains_codex_package {
                return Err(AppError::new(
                    "APP_PROCESS_QUERY_FAILED",
                    "Codex Store package identity was not in the expected tasklist column.",
                ));
            }
            continue;
        }
        let Some(image_name) = fields.first() else {
            return Err(AppError::new(
                "APP_PROCESS_QUERY_FAILED",
                "Codex Store process row did not contain an image name.",
            ));
        };
        let Some(raw_pid) = fields.get(1) else {
            return Err(AppError::new(
                "APP_PROCESS_QUERY_FAILED",
                "Codex Store process row did not contain a PID.",
            ));
        };
        let pid = raw_pid.parse::<u32>().map_err(|error| {
            AppError::new(
                "APP_PROCESS_QUERY_FAILED",
                format!("Codex Store process row contained an invalid PID: {error}"),
            )
        })?;
        processes.push(WindowsStoreProcess::new(*image_name, pid));
    }

    processes.sort_unstable_by_key(|process| process.process_id);
    processes.dedup_by_key(|process| process.process_id);
    Ok(processes)
}

fn query_windows_store_processes() -> AppResult<Vec<WindowsStoreProcess>> {
    let mut command = Command::new("tasklist");
    command.args(["/APPS", "/FO", "CSV", "/NH"]);

    let output = hide_console_window(&mut command)
        .output()
        .map_err(|error| {
            AppError::new(
                "APP_PROCESS_QUERY_FAILED",
                format!("Failed to query Codex Store processes: {error}"),
            )
        })?;
    if !output.status.success() {
        return Err(AppError::new(
            "APP_PROCESS_QUERY_FAILED",
            format!(
                "Codex Store process query exited with status {}.",
                output.status
            ),
        ));
    }

    parse_windows_store_processes(&String::from_utf8_lossy(&output.stdout))
}

pub fn open_or_activate_codex_app(_codex_home: Option<&Path>) -> AppResult<String> {
    let target = resolve_windows_app_target();

    match target {
        AppLaunchTarget::WindowsStore(shell_target) => {
            let mut command = Command::new("explorer.exe");
            command.arg(&shell_target);
            hide_console_window(&mut command).spawn().map_err(|error| {
                AppError::new("APP_OPEN_FAILED", format!("Failed to open Codex: {error}"))
            })?;

            Ok(shell_target)
        }
    }
}

fn persist_real_codex_path(
    codex_home: Option<&Path>,
    state: &mut InstallState,
    path: Option<&Path>,
) {
    let next_path = path.map(|path| path.to_string_lossy().into_owned());
    if state.real_codex_path != next_path {
        state.real_codex_path = next_path;
        save_install_state(codex_home, state);
    }
}

pub(super) fn resolve_real_codex_cli_with_source(
    codex_home: Option<&Path>,
) -> Option<(PathBuf, RealCodexPathSource)> {
    let managed_shim_path = managed_codex_shim_path(codex_home);
    let state = load_install_state(codex_home);

    // User override wins only while runnable. If it doesn't pass
    // validation any more (file moved, AV pruned the .cmd, shim points
    // at a deleted runtime, etc.) we silently fall through to the normal
    // discovery chain so the user isn't permanently stuck. The override
    // stays persisted so a stable retry will pick it up if it recovers.
    if let Some(raw_user_path) = state.user_codex_path.as_ref().map(PathBuf::from) {
        if let Some(resolved_path) = resolve_windows_invokable_path(&raw_user_path)
            .filter(|path| is_acceptable_real_codex_cli_path(path, Some(&managed_shim_path)))
            .filter(|path| probe_codex_version(path).is_some())
        {
            return Some((resolved_path, RealCodexPathSource::UserOverride));
        }
    }

    let mut state = state;
    if let Some(raw_path) = state.real_codex_path.as_ref().map(PathBuf::from) {
        if let Some(resolved_path) = resolve_windows_invokable_path(&raw_path)
            .filter(|path| is_acceptable_real_codex_cli_path(path, Some(&managed_shim_path)))
            .filter(|path| probe_codex_version(path).is_some())
        {
            persist_real_codex_path(codex_home, &mut state, Some(&resolved_path));
            return Some((resolved_path, RealCodexPathSource::InstallState));
        }
    }

    let discovered_path = discover_real_codex_cli_path(Some(&managed_shim_path));
    if let Some(path) = discovered_path.as_deref() {
        persist_real_codex_path(codex_home, &mut state, Some(path));
    } else if state.real_codex_path.is_some() {
        persist_real_codex_path(codex_home, &mut state, None);
    }
    discovered_path.map(|path| (path, RealCodexPathSource::Discovery))
}

fn resolve_real_codex_cli(codex_home: Option<&Path>) -> Option<PathBuf> {
    resolve_real_codex_cli_with_source(codex_home).map(|(path, _)| path)
}

pub fn forward_to_real_codex(args: &[String], codex_home: Option<&Path>) -> AppResult<i32> {
    let Some(real_codex_path) = resolve_real_codex_cli(codex_home) else {
        return Err(AppError::new(
            "REAL_CODEX_NOT_FOUND",
            "Real Codex CLI path not found. Run `codex_switch_cli.exe install` first.",
        ));
    };

    let mut command = Command::new(real_codex_path);
    command.args(args);
    let status = hide_console_window(&mut command)
        .status()
        .map_err(|error| {
            AppError::new(
                "REAL_CODEX_LAUNCH_FAILED",
                format!("Failed to launch real Codex CLI: {error}"),
            )
        })?;

    Ok(status.code().unwrap_or(1))
}

fn build_app_server_command(real_codex_path: &Path, runtime_codex_home: &Path) -> Command {
    let mut command = Command::new(real_codex_path);
    // `codex app-server` is the upstream control-plane subcommand; it
    // takes no sandbox/approval flags (those bind only to the TUI). See
    // `openai/codex` `codex-rs/cli/src/main.rs` for the subcommand
    // wiring.
    command.arg("app-server");
    hide_console_window(&mut command);
    command.current_dir(runtime_codex_home);
    command.env("CODEX_HOME", runtime_codex_home);
    configure_child_process(&mut command);
    command
}

/// Build the `codex login` command using a resolved real-codex path.
/// Anchoring on `cli_codex_home` (the live `~/.codex`) for resolution
/// keeps the managed-shim filter correct even when `runtime_codex_home`
/// is a sandboxed sibling. `runtime_codex_home` is what the spawned
/// process sees as `CODEX_HOME` and is where it will write `auth.json`.
///
/// Callers must resolve the path beforehand and surface
/// `REAL_CODEX_NOT_FOUND` to the user instead of falling back to
/// `cmd /C codex login` — that fallback only ever produced a Chinese
/// "command not found" message in the OEM codepage that
/// `from_utf8_lossy` then mangled into mojibake.
fn build_login_command(real_codex_path: &Path, runtime_codex_home: &Path) -> Command {
    let mut command = Command::new(real_codex_path);
    command.arg("login");
    hide_console_window(&mut command);
    command.current_dir(runtime_codex_home);
    command.env("CODEX_HOME", runtime_codex_home);
    configure_child_process(&mut command);
    command
}

fn login_browser_from_metadata(value: Option<&str>) -> Option<LoginBrowser> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| {
            if value.eq_ignore_ascii_case("edge") {
                Some(LoginBrowser::Edge)
            } else {
                None
            }
        })
}

fn login_browser_for_profile(
    codex_home: Option<&Path>,
    profile_name: Option<&str>,
) -> Option<LoginBrowser> {
    let profile_name = profile_name
        .map(str::trim)
        .filter(|name| !name.is_empty())?;
    let metadata = load_profile_metadata(profile_name, codex_home);
    login_browser_from_metadata(metadata.login_browser.as_deref())
}

impl LoginBrowser {
    fn label(self) -> &'static str {
        "Edge"
    }

    fn override_env_var(self) -> &'static str {
        "CODEX_SWITCH_EDGE_PATH"
    }
}

fn resolve_login_browser_launch(
    codex_home: Option<&Path>,
    profile_name: Option<&str>,
) -> AppResult<Option<LoginBrowserLaunch>> {
    let Some(browser) = login_browser_for_profile(codex_home, profile_name) else {
        return Ok(None);
    };
    let Some(executable) = resolve_login_browser_executable(browser) else {
        return Err(AppError::new(
            "LOGIN_BROWSER_NOT_FOUND",
            format!(
                "{} was not found. Install it or set {} before logging into this profile.",
                browser.label(),
                browser.override_env_var()
            ),
        ));
    };
    Ok(Some(LoginBrowserLaunch {
        browser,
        executable,
    }))
}

fn resolve_login_browser_app_server_launch(
    real_codex_path: &Path,
    cli_codex_home: &Path,
    runtime_codex_home: &Path,
    profile_name: Option<&str>,
) -> AppResult<Option<(Command, LoginBrowserLaunch)>> {
    let Some(launch) = resolve_login_browser_launch(Some(cli_codex_home), profile_name)? else {
        return Ok(None);
    };
    Ok(Some((
        build_app_server_command(real_codex_path, runtime_codex_home),
        launch,
    )))
}

fn resolve_login_browser_executable(browser: LoginBrowser) -> Option<PathBuf> {
    env::var_os(browser.override_env_var())
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .or_else(|| {
            common_login_browser_paths()
                .into_iter()
                .find(|path| path.is_file())
        })
}

fn common_login_browser_paths() -> Vec<PathBuf> {
    let mut paths = vec![
        PathBuf::from(r"C:\Program Files\Microsoft\Edge\Application\msedge.exe"),
        PathBuf::from(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"),
    ];

    if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
        let local_app_data = PathBuf::from(local_app_data);
        paths.push(
            local_app_data
                .join("Microsoft")
                .join("Edge")
                .join("Application")
                .join("msedge.exe"),
        );
    }

    paths
}

fn build_login_browser_command(launch: &LoginBrowserLaunch, url: &str) -> Command {
    let mut command = Command::new(&launch.executable);
    command.arg(url);
    hide_console_window(&mut command);
    command
}

fn open_login_url_in_browser(launch: &LoginBrowserLaunch, url: &str) -> std::io::Result<()> {
    let mut command = build_login_browser_command(launch, url);
    command.spawn().map(|_| ())
}

pub fn fetch_account_via_app_server(
    cli_codex_home: &Path,
    runtime_codex_home: &Path,
) -> AppResult<AppServerSnapshot> {
    let Some(real_codex_path) = resolve_real_codex_cli(Some(cli_codex_home)) else {
        return Err(AppError::new(
            "REAL_CODEX_NOT_FOUND",
            "Real Codex CLI path not found. Run `codex_switch_cli.exe install` first.",
        ));
    };

    let command = build_app_server_command(&real_codex_path, runtime_codex_home);
    fetch_account_snapshot(command)
}

pub fn run_codex_login(
    cli_codex_home: &Path,
    runtime_codex_home: &Path,
    profile_name: Option<&str>,
) -> AppResult<()> {
    let Some(real_codex_path) = resolve_real_codex_cli(Some(cli_codex_home)) else {
        return Err(AppError::new(
            "REAL_CODEX_NOT_FOUND",
            "Real Codex CLI path not found. Set the codex CLI location in the dashboard before logging in.",
        ));
    };
    if let Some((command, launch)) = resolve_login_browser_app_server_launch(
        &real_codex_path,
        cli_codex_home,
        runtime_codex_home,
        profile_name,
    )? {
        return run_chatgpt_browser_login(command, |auth_url| {
            open_login_url_in_browser(&launch, auth_url).map_err(|error| {
                AppError::new(
                    "LOGIN_BROWSER_OPEN_FAILED",
                    format!(
                        "Failed to open login URL in {}: {error}",
                        launch.browser.label()
                    ),
                )
            })
        });
    }

    // Pipe stdio so wait_with_output() captures stderr/stdout the same
    // way the previous `.output()` call did — we surface those bytes in
    // the LOGIN_FAILED toast when codex login itself errors out.
    let child = build_login_command(&real_codex_path, runtime_codex_home)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            AppError::new(
                "LOGIN_COMMAND_FAILED",
                format!("Failed to start `codex login`: {error}"),
            )
        })?;
    let output = wait_for_login_or_cancel(child)?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let message = if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        "`codex login` exited without a success status.".to_string()
    };

    Err(AppError::new("LOGIN_FAILED", message))
}

pub fn run_codex_device_code_login(
    cli_codex_home: &Path,
    runtime_codex_home: &Path,
    show_device_code: &mut dyn FnMut(&DeviceCodePrompt) -> AppResult<()>,
) -> AppResult<()> {
    let Some(real_codex_path) = resolve_real_codex_cli(Some(cli_codex_home)) else {
        return Err(AppError::new(
            "REAL_CODEX_NOT_FOUND",
            "Real Codex CLI path not found. Set the codex CLI location in the dashboard before logging in.",
        ));
    };

    let command = build_app_server_command(&real_codex_path, runtime_codex_home);
    run_chatgpt_device_code_login(command, |prompt| show_device_code(prompt))
}

/// Validate a user-provided codex CLI path. Resolves Windows extensions
/// (`.cmd` / `.exe` / etc.) so a user can paste either `C:\...\codex` or
/// `C:\...\codex.cmd`. Rejects the managed shim because pointing the
/// override at our own shim creates an infinite indirection.
pub(super) fn validate_user_codex_cli_path(
    codex_home: Option<&Path>,
    raw_input: &str,
) -> AppResult<PathBuf> {
    let trimmed = raw_input.trim();
    if trimmed.is_empty() {
        return Err(AppError::new(
            "CODEX_CLI_PATH_EMPTY",
            "Please provide the full path to the codex CLI binary.",
        ));
    }
    let candidate = PathBuf::from(trimmed);
    let resolved = resolve_windows_invokable_path(&candidate).ok_or_else(|| {
        AppError::new(
            "CODEX_CLI_PATH_INVALID",
            format!("No invokable file found at {}.", candidate.display()),
        )
    })?;

    let managed_shim_path = managed_codex_shim_path(codex_home);
    if !is_acceptable_real_codex_cli_path(&resolved, Some(&managed_shim_path)) {
        return Err(AppError::new(
            "CODEX_CLI_PATH_REJECTED",
            "That path is the codex_switch managed shim or a Windows Apps alias; pick the real codex CLI binary instead.",
        ));
    }
    if probe_codex_version(&resolved).is_none() {
        return Err(AppError::new(
            "CODEX_CLI_PATH_NOT_RUNNABLE",
            format!(
                "{} exists but did not run `codex --version` successfully.",
                resolved.display()
            ),
        ));
    }

    Ok(resolved)
}

/// Persist a user override for the real codex CLI path. Returns the
/// canonicalized path that was saved.
pub fn set_user_codex_cli_path(codex_home: Option<&Path>, raw_input: &str) -> AppResult<PathBuf> {
    let resolved = validate_user_codex_cli_path(codex_home, raw_input)?;
    let mut state = load_install_state(codex_home);
    let next = Some(resolved.to_string_lossy().into_owned());
    if state.user_codex_path != next {
        state.user_codex_path = next;
        save_install_state(codex_home, &state);
    }
    Ok(resolved)
}

/// Clear the user override and let auto-discovery take over again.
pub fn clear_user_codex_cli_path(codex_home: Option<&Path>) {
    let mut state = load_install_state(codex_home);
    if state.user_codex_path.is_some() {
        state.user_codex_path = None;
        save_install_state(codex_home, &state);
    }
}

/// Resolver impl that delegates to the per-platform helpers above. The
/// shared `codex_cli_path` module talks to this via the trait so the
/// Tauri command bridge stays OS-agnostic.
pub struct WindowsCodexPathResolver;

pub static WINDOWS_CODEX_PATH_RESOLVER: WindowsCodexPathResolver = WindowsCodexPathResolver;

impl CodexPathResolver for WindowsCodexPathResolver {
    fn resolve_with_source(&self, codex_home: &Path) -> Option<(PathBuf, RealCodexPathSource)> {
        resolve_real_codex_cli_with_source(Some(codex_home))
    }

    fn set_user_path(&self, codex_home: &Path, raw_input: &str) -> AppResult<PathBuf> {
        set_user_codex_cli_path(Some(codex_home), raw_input)
    }

    fn clear_user_path(&self, codex_home: &Path) {
        clear_user_codex_cli_path(Some(codex_home));
    }

    fn suggested_paths(&self, codex_home: &Path) -> Vec<PathBuf> {
        suggested_codex_cli_paths(Some(codex_home))
    }

    fn redetect_runnable_paths(&self, codex_home: &Path) -> Vec<CodexCliCandidate> {
        redetect_runnable_codex_cli_paths(Some(codex_home))
    }
}

/// Return common codex CLI install locations on Windows that actually
/// exist on disk right now. Used to seed clickable hints in the
/// "Codex 路径" dialog so users don't have to hunt manually.
pub fn suggested_codex_cli_paths(codex_home: Option<&Path>) -> Vec<PathBuf> {
    let mut suggestions: Vec<PathBuf> = Vec::new();
    let managed_shim = managed_codex_shim_path(codex_home);
    let mut push = |path: PathBuf| {
        if let Some(resolved) = resolve_windows_invokable_path(&path) {
            if is_acceptable_real_codex_cli_path(&resolved, Some(&managed_shim))
                && !suggestions
                    .iter()
                    .any(|existing| paths_match(existing, &resolved))
            {
                suggestions.push(resolved);
            }
        }
    };

    if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
        let base = PathBuf::from(local_app_data);
        push(base.join("Programs").join("codex").join("codex.exe"));
        push(base.join("Programs").join("codex").join("codex.cmd"));
        push(
            base.join("Programs")
                .join("codex")
                .join("bin")
                .join("codex.cmd"),
        );
    }
    push_official_codex_cli_candidates(&mut push);
    if let Some(app_data) = env::var_os("APPDATA") {
        let npm_base = PathBuf::from(app_data).join("npm");
        push(npm_base.join("codex.cmd"));
        push(npm_base.join("codex"));
    }
    if let Some(program_files) = env::var_os("ProgramFiles") {
        let base = PathBuf::from(program_files).join("codex");
        push(base.join("codex.exe"));
        push(base.join("codex.cmd"));
        push(base.join("bin").join("codex.cmd"));
    }
    if let Some(program_files_x86) = env::var_os("ProgramFiles(x86)") {
        let base = PathBuf::from(program_files_x86).join("codex");
        push(base.join("codex.exe"));
        push(base.join("codex.cmd"));
    }

    let user_profile = env::var_os("USERPROFILE");
    push_nvm_codex_candidates(
        env::var_os("NVM_HOME"),
        env::var_os("NVM_DIR"),
        env::var_os("NVM_BIN"),
        env::var_os("NVM_SYMLINK"),
        &mut push,
    );
    push_fnm_codex_candidates(
        env::var_os("FNM_DIR"),
        env::var_os("FNM_MULTISHELL_PATH"),
        env::var_os("LOCALAPPDATA"),
        user_profile.clone(),
        &mut push,
    );
    push_asdf_codex_candidates(env::var_os("ASDF_DATA_DIR"), user_profile, &mut push);

    for path in where_codex_candidate_paths_with_timeout(WHERE_CODEX_TIMEOUT) {
        push(path);
    }

    suggestions
}

/// How long a single `codex --version` probe may run before we kill it
/// and treat the candidate as unusable. A little more generous than
/// macOS: a Windows `.cmd` shim plus npm wrapper has a slower cold
/// start, but a healthy codex still answers well under this.
const RUNNABLE_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Upper bound on how many candidates the auto-detect scan will probe.
/// Each probe spawns a child (up to `RUNNABLE_PROBE_TIMEOUT`), so without
/// a cap a machine with many `where codex` hits could stall the scan.
/// Realistic machines have 1-3 candidates.
const MAX_PROBE_CANDIDATES: usize = 12;

fn looks_like_codex_version(version: &str) -> bool {
    version.split_whitespace().next().is_some_and(|name| {
        name.eq_ignore_ascii_case("codex-cli") || name.eq_ignore_ascii_case("codex")
    })
}

/// Probe whether `path` is a runnable codex CLI and capture its version.
/// `Some(version)` means it's a file that ran, exited 0, and reported a
/// Codex version line; `None` means not-a-file, not Codex, couldn't spawn,
/// exited non-zero, or timed out. The failure is logged so a broken install
/// leaves a diagnostic trail instead of looking identical to "not found".
fn probe_codex_version(path: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    let mut command = Command::new(path);
    command.arg("--version");
    hide_console_window(&mut command);
    let result =
        crate::shared::codex_cli_path::probe_version_with_timeout(command, RUNNABLE_PROBE_TIMEOUT);
    match result {
        Some(version) if looks_like_codex_version(&version) => Some(version),
        Some(version) => {
            eprintln!(
                "codex probe: {} did not report a Codex version: {version:?}",
                path.display()
            );
            None
        }
        None => {
            eprintln!(
                "codex probe: {} is not a runnable codex (spawn / non-zero exit / timeout)",
                path.display()
            );
            None
        }
    }
}

/// Force a fresh scan for the Settings auto-detect button, keeping only
/// candidates that pass the runnable probe. Reuses
/// `suggested_codex_cli_paths`, which already resolves Windows
/// extensions, filters the managed shim / Windows Apps aliases, and
/// folds in `where codex` (every PATH match) — so it is the full
/// candidate set. Ignores the cached/override path so a wrong saved
/// path can be corrected.
pub fn redetect_runnable_codex_cli_paths(codex_home: Option<&Path>) -> Vec<CodexCliCandidate> {
    suggested_codex_cli_paths(codex_home)
        .into_iter()
        .take(MAX_PROBE_CANDIDATES)
        .filter_map(|path| {
            probe_codex_version(&path).map(|version| CodexCliCandidate {
                path: path.to_string_lossy().into_owned(),
                version: (!version.is_empty()).then_some(version),
            })
        })
        .collect()
}

#[cfg(target_os = "windows")]
struct VerifiedCodexProcessHandle(HANDLE);

#[cfg(target_os = "windows")]
impl Drop for VerifiedCodexProcessHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

#[cfg(target_os = "windows")]
fn package_full_name_for_process(handle: HANDLE) -> AppResult<Option<String>> {
    let mut buffer_length = 0u32;
    let first_status =
        unsafe { GetPackageFullName(handle, &mut buffer_length, std::ptr::null_mut()) };
    if first_status == APPMODEL_ERROR_NO_PACKAGE || first_status == ERROR_INVALID_PARAMETER {
        return Ok(None);
    }
    if first_status != ERROR_INSUFFICIENT_BUFFER || buffer_length == 0 {
        return Err(AppError::new(
            "APP_PROCESS_QUERY_FAILED",
            format!("Failed to size Store package identity buffer: {first_status}"),
        ));
    }

    let mut buffer = vec![0u16; buffer_length as usize];
    let second_status =
        unsafe { GetPackageFullName(handle, &mut buffer_length, buffer.as_mut_ptr()) };
    if second_status == APPMODEL_ERROR_NO_PACKAGE || second_status == ERROR_INVALID_PARAMETER {
        return Ok(None);
    }
    if second_status != 0 {
        return Err(AppError::new(
            "APP_PROCESS_QUERY_FAILED",
            format!("Failed to read Store package identity: {second_status}"),
        ));
    }

    let end = buffer
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(buffer.len());
    String::from_utf16(&buffer[..end])
        .map(Some)
        .map_err(|error| {
            AppError::new(
                "APP_PROCESS_QUERY_FAILED",
                format!("Store package identity was not valid UTF-16: {error}"),
            )
        })
}

#[cfg(target_os = "windows")]
fn open_verified_codex_process(process_id: u32) -> AppResult<Option<VerifiedCodexProcessHandle>> {
    let desired_access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | SYNCHRONIZE;
    let handle = unsafe { OpenProcess(desired_access, 0, process_id) };
    if handle.is_null() {
        let error = unsafe { GetLastError() };
        if error == ERROR_INVALID_PARAMETER {
            return Ok(None);
        }
        return Err(AppError::new(
            "APP_PROCESS_QUERY_FAILED",
            format!("Failed to open Codex Store process {process_id}: {error}"),
        ));
    }

    let handle = VerifiedCodexProcessHandle(handle);
    let Some(package_name) = package_full_name_for_process(handle.0)? else {
        return Ok(None);
    };
    if !is_windows_store_codex_package(&package_name) {
        return Ok(None);
    }

    Ok(Some(handle))
}

#[cfg(target_os = "windows")]
struct CloseWindowsContext {
    process_id: u32,
    process_handle: HANDLE,
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn close_verified_process_window(window: HWND, context: LPARAM) -> i32 {
    let context = unsafe { &*(context as *const CloseWindowsContext) };
    if unsafe { WaitForSingleObject(context.process_handle, 0) } == WAIT_OBJECT_0 {
        return 1;
    }

    let mut window_process_id = 0u32;
    unsafe {
        GetWindowThreadProcessId(window, &mut window_process_id);
    }
    if window_process_id == context.process_id
        && unsafe { WaitForSingleObject(context.process_handle, 0) } != WAIT_OBJECT_0
    {
        unsafe {
            PostMessageW(window, WM_CLOSE, 0, 0);
        }
    }
    1
}

#[cfg(target_os = "windows")]
fn request_graceful_codex_app_exit(
    process: &VerifiedCodexProcessHandle,
    process_id: u32,
) -> AppResult<()> {
    let context = CloseWindowsContext {
        process_id,
        process_handle: process.0,
    };
    if unsafe {
        EnumWindows(
            Some(close_verified_process_window),
            &context as *const CloseWindowsContext as LPARAM,
        )
    } == 0
    {
        let error = unsafe { GetLastError() };
        return Err(AppError::new(
            "APP_EXIT_COMMAND_FAILED",
            format!("Failed to enumerate Codex Store windows for process {process_id}: {error}"),
        ));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn request_codex_app_exit(process_ids: &[u32], force: bool) -> AppResult<()> {
    for process_id in process_ids {
        let Some(process) = open_verified_codex_process(*process_id)? else {
            continue;
        };
        if !force {
            request_graceful_codex_app_exit(&process, *process_id)?;
            continue;
        }

        let exit_code = u32::from(force);
        if unsafe { TerminateProcess(process.0, exit_code) } == 0 {
            let error = unsafe { GetLastError() };
            if unsafe { WaitForSingleObject(process.0, 0) } != WAIT_OBJECT_0 {
                return Err(AppError::new(
                    "APP_EXIT_COMMAND_FAILED",
                    format!(
                        "Failed to terminate verified Codex Store process {process_id}: {error}"
                    ),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn request_codex_app_exit(_process_ids: &[u32], _force: bool) -> AppResult<()> {
    Err(AppError::new(
        "APP_EXIT_COMMAND_FAILED",
        "Codex Store process termination is only supported on Windows.",
    ))
}

struct SystemAppProcessOps;

impl AppProcessOps for SystemAppProcessOps {
    fn snapshot(&mut self) -> AppResult<Vec<WindowsStoreProcess>> {
        query_windows_store_processes()
    }

    fn terminate(&mut self, process_ids: &[u32], force: bool) -> AppResult<()> {
        request_codex_app_exit(process_ids, force)
    }

    fn sleep(&mut self, duration: Duration) {
        thread::sleep(duration);
    }
}

fn record_app_exit_observation(
    observation: &mut AppExitObservation,
    processes: &[WindowsStoreProcess],
) {
    observation.had_processes |= !processes.is_empty();
    observation.app_was_running |= processes.iter().any(WindowsStoreProcess::is_ui_host);
}

fn wait_for_stable_app_exit<O: AppProcessOps + ?Sized>(
    ops: &mut O,
    attempts: usize,
    observation: &mut AppExitObservation,
) -> AppResult<bool> {
    let mut consecutive_empty_snapshots = 0;
    for attempt in 0..attempts {
        let processes = ops.snapshot()?;
        record_app_exit_observation(observation, &processes);
        if processes.is_empty() {
            consecutive_empty_snapshots += 1;
            if consecutive_empty_snapshots >= 2 {
                return Ok(true);
            }
        } else {
            consecutive_empty_snapshots = 0;
        }
        if attempt + 1 < attempts {
            ops.sleep(Duration::from_millis(200));
        }
    }
    Ok(false)
}

fn force_remaining_app_processes_until_stable<O: AppProcessOps + ?Sized>(
    ops: &mut O,
    attempts: usize,
    observation: &mut AppExitObservation,
) -> AppResult<bool> {
    let mut consecutive_empty_snapshots = 0;
    for attempt in 0..attempts {
        let processes = ops.snapshot()?;
        record_app_exit_observation(observation, &processes);
        if processes.is_empty() {
            consecutive_empty_snapshots += 1;
            if consecutive_empty_snapshots >= 2 {
                return Ok(true);
            }
        } else {
            consecutive_empty_snapshots = 0;
            let process_ids = processes
                .iter()
                .map(|process| process.process_id)
                .collect::<Vec<_>>();
            ops.terminate(&process_ids, true)?;
        }
        if attempt + 1 < attempts {
            ops.sleep(Duration::from_millis(200));
        }
    }

    ops.sleep(Duration::from_millis(200));
    wait_for_stable_app_exit(ops, 2, observation)
}

fn quit_codex_app_with_ops_observed<O: AppProcessOps + ?Sized>(
    ops: &mut O,
    observation: &mut AppExitObservation,
) -> AppResult<bool> {
    let mut initial_processes = ops.snapshot()?;
    record_app_exit_observation(observation, &initial_processes);
    if initial_processes.is_empty() {
        ops.sleep(Duration::from_millis(200));
        initial_processes = ops.snapshot()?;
        record_app_exit_observation(observation, &initial_processes);
        if initial_processes.is_empty() {
            return Ok(false);
        }
    }

    let ui_process_ids = initial_processes
        .iter()
        .filter(|process| process.is_ui_host())
        .map(|process| process.process_id)
        .collect::<Vec<_>>();

    if !ui_process_ids.is_empty() {
        ops.terminate(&ui_process_ids, false)?;
        if wait_for_stable_app_exit(ops, 20, observation)? {
            return Ok(observation.app_was_running);
        }
    }

    if force_remaining_app_processes_until_stable(ops, 10, observation)? {
        return Ok(observation.app_was_running);
    }

    Err(AppError::new(
        "APP_EXIT_FAILED",
        "Codex did not exit cleanly. Close it manually and retry.",
    ))
}

#[cfg(test)]
fn quit_codex_app_with_ops<O: AppProcessOps + ?Sized>(ops: &mut O) -> AppResult<bool> {
    quit_codex_app_with_ops_observed(ops, &mut AppExitObservation::default())
}

pub fn quit_codex_app_and_observe(observation: &mut AppExitObservation) -> AppResult<()> {
    quit_codex_app_with_ops_observed(&mut SystemAppProcessOps, observation).map(|_| ())
}

pub fn quit_codex_app_if_running() -> AppResult<bool> {
    let mut observation = AppExitObservation::default();
    match quit_codex_app_and_observe(&mut observation) {
        Ok(()) => Ok(observation.app_was_running),
        Err(mut error) => {
            let warnings = reopen_codex_app_if_needed(observation.app_was_running, None);
            if !warnings.is_empty() {
                error.message = format!("{} {}", error.message, warnings.join(" "));
            }
            Err(error)
        }
    }
}

pub fn reopen_codex_app_if_needed(
    app_was_running: bool,
    _codex_home: Option<&Path>,
) -> Vec<String> {
    if !app_was_running {
        return Vec::new();
    }

    let target = resolve_windows_app_target();

    let result = match target {
        AppLaunchTarget::WindowsStore(shell_target) => {
            let mut command = Command::new("explorer.exe");
            command.arg(shell_target);
            hide_console_window(&mut command).spawn()
        }
    };

    if let Err(error) = result {
        return vec![format!("Warning: failed to relaunch Codex: {error}")];
    }

    Vec::new()
}

/// Confirm that Shell activation actually produced a process belonging to the
/// managed Codex Store package. A successful `explorer.exe` spawn alone does
/// not prove that the app was installed or activated.
fn wait_for_codex_app_start_with_ops<O: AppProcessOps + ?Sized>(
    ops: &mut O,
    attempts: usize,
) -> AppResult<()> {
    for attempt in 0..attempts {
        if !ops.snapshot()?.is_empty() {
            return Ok(());
        }
        if attempt + 1 < attempts {
            ops.sleep(Duration::from_millis(200));
        }
    }
    Err(AppError::new(
        "APP_RELAUNCH_FAILED",
        "Codex launch was requested, but no managed Windows Store process appeared.",
    ))
}

pub fn wait_for_codex_app_start() -> AppResult<()> {
    wait_for_codex_app_start_with_ops(&mut SystemAppProcessOps, 25)
}

impl PlatformHooks for WindowsPlatformHooks {
    fn open_or_activate_codex_app(&self, codex_home: Option<&Path>) -> AppResult<String> {
        open_or_activate_codex_app(codex_home)
    }

    fn quit_codex_app_if_running(&self) -> AppResult<bool> {
        quit_codex_app_if_running()
    }

    fn quit_codex_app_and_observe(&self, observation: &mut AppExitObservation) -> AppResult<()> {
        quit_codex_app_and_observe(observation)
    }

    fn profile_switch_exit_stability_attempts(&self) -> usize {
        PROFILE_SWITCH_EXIT_STABILITY_ATTEMPTS
    }

    fn reopen_codex_app_if_needed(
        &self,
        app_was_running: bool,
        codex_home: Option<&Path>,
    ) -> Vec<String> {
        reopen_codex_app_if_needed(app_was_running, codex_home)
    }

    fn run_codex_login(
        &self,
        cli_codex_home: &Path,
        runtime_codex_home: &Path,
        profile_name: Option<&str>,
    ) -> AppResult<()> {
        run_codex_login(cli_codex_home, runtime_codex_home, profile_name)
    }

    fn run_codex_device_code_login(
        &self,
        cli_codex_home: &Path,
        runtime_codex_home: &Path,
        _profile_name: Option<&str>,
        show_device_code: &mut dyn FnMut(&DeviceCodePrompt) -> AppResult<()>,
    ) -> AppResult<()> {
        run_codex_device_code_login(cli_codex_home, runtime_codex_home, show_device_code)
    }

    fn fetch_account_via_app_server(
        &self,
        cli_codex_home: &Path,
        runtime_codex_home: &Path,
    ) -> AppResult<AppServerSnapshot> {
        fetch_account_via_app_server(cli_codex_home, runtime_codex_home)
    }

    fn sync_root_openai_base_url_for_profile(
        &self,
        profile_name: &str,
        codex_home: Option<&Path>,
    ) -> AppResult<()> {
        crate::shared::config::sync_root_openai_base_url_for_profile(profile_name, codex_home)
    }

    fn acquire_background_refresh_lock(
        &self,
        codex_home: &Path,
    ) -> AppResult<Option<crate::shared::process_lock::ProcessLockGuard>> {
        super::refresh_runtime::acquire_refresh_lock(codex_home).map(Some)
    }

    fn sync_live_auth_if_current_profile(
        &self,
        profile_name: &str,
        codex_home: &Path,
        refreshed_auth_path: &Path,
    ) -> AppResult<()> {
        super::refresh_runtime::sync_live_auth_if_current_profile(
            profile_name,
            codex_home,
            refreshed_auth_path,
        )
    }

    fn sync_on_window_close(&self) -> AppResult<()> {
        crate::windows::bootstrap::sync_root_state_to_current_profile(None).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "windows")]
    use super::open_verified_codex_process;
    use super::{
        build_app_server_command, build_login_browser_command, discover_real_codex_cli_path,
        is_acceptable_real_codex_cli_path, load_install_state, login_browser_for_profile,
        looks_like_codex_version, parse_windows_store_processes, quit_codex_app_with_ops,
        quit_codex_app_with_ops_observed, redetect_runnable_codex_cli_paths,
        resolve_login_browser_app_server_launch, resolve_real_codex_cli,
        resolve_windows_app_target, suggested_codex_cli_paths, wait_for_codex_app_start_with_ops,
        where_codex_candidate_paths_with_timeout, windows_store_shell_target, AppExitObservation,
        AppLaunchTarget, AppProcessOps, InstallState, LoginBrowser, LoginBrowserLaunch,
        WindowsPlatformHooks, WindowsStoreProcess, WINDOWS_STORE_APP_ID,
    };
    use crate::errors::{AppError, AppResult};
    use crate::platform::hooks::PlatformHooks;
    use crate::windows::{env_guard, EnvVarGuard};
    use serde_json::to_string_pretty;
    use std::collections::VecDeque;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    fn temp_codex_home(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("codex-switch-process-{name}-{unique}"))
    }

    fn assert_same_existing_path(actual: Option<PathBuf>, expected: PathBuf) {
        let actual = actual.expect("expected a discovered CLI path");
        assert_eq!(
            fs::canonicalize(actual).unwrap(),
            fs::canonicalize(expected).unwrap()
        );
    }

    fn write_profile_metadata(codex_home: &Path, profile_name: &str, account_label: &str) {
        write_profile_metadata_with_login_browser(codex_home, profile_name, account_label, None);
    }

    fn write_profile_metadata_with_login_browser(
        codex_home: &Path,
        profile_name: &str,
        account_label: &str,
        login_browser: Option<&str>,
    ) {
        let profile_dir = codex_home.join("account_backup").join(profile_name);
        fs::create_dir_all(&profile_dir).unwrap();
        let login_browser = login_browser
            .map(|value| format!(",\n  \"login_browser\": \"{value}\""))
            .unwrap_or_default();
        fs::write(
            profile_dir.join("profile.json"),
            format!(
                "{{\n  \"folder_name\": \"{profile_name}\",\n  \"account_label\": \"{account_label}\"{login_browser}\n}}\n"
            ),
        )
        .unwrap();
    }

    fn write_runnable_codex(path: &Path, version: &str) {
        if cfg!(target_os = "windows") {
            fs::write(path, format!("@echo off\r\necho {version}\r\n")).unwrap();
        } else {
            fs::write(path, format!("#!/bin/sh\necho {version}\n")).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;

                let mut permissions = fs::metadata(path).unwrap().permissions();
                permissions.set_mode(0o755);
                fs::set_permissions(path, permissions).unwrap();
            }
        }
    }

    #[cfg(target_os = "windows")]
    fn write_rust_exe(path: &Path, source: &str) {
        let source_path = path.with_extension("rs");
        fs::write(&source_path, source).unwrap();
        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| std::ffi::OsString::from("rustc"));
        let output = std::process::Command::new(rustc)
            .arg(&source_path)
            .arg("-o")
            .arg(path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "failed to compile codex.exe fixture\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(target_os = "windows")]
    fn write_runnable_codex_exe(path: &Path, version: &str) {
        write_rust_exe(path, &format!("fn main() {{ println!({version:?}); }}\n"));
    }

    #[cfg(target_os = "windows")]
    fn write_hanging_where_exe(path: &Path) {
        write_rust_exe(
            path,
            "fn main() { std::thread::sleep(std::time::Duration::from_secs(5)); }\n",
        );
    }

    fn write_failing_codex(path: &Path) {
        if cfg!(target_os = "windows") {
            fs::write(path, "@echo off\r\nexit /b 127\r\n").unwrap();
        } else {
            fs::write(path, "#!/bin/sh\nexit 127\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;

                let mut permissions = fs::metadata(path).unwrap().permissions();
                permissions.set_mode(0o755);
                fs::set_permissions(path, permissions).unwrap();
            }
        }
    }

    #[test]
    fn login_browser_for_profile_routes_explicit_edge_metadata_only() {
        let codex_home = temp_codex_home("explicit-edge-account");
        write_profile_metadata_with_login_browser(
            &codex_home,
            "a",
            "edge-enabled@profile.example",
            Some("  EdGe  "),
        );
        write_profile_metadata(&codex_home, "b", "plain@profile.example");
        write_profile_metadata_with_login_browser(
            &codex_home,
            "c",
            "unknown@profile.example",
            Some("firefox"),
        );

        assert_eq!(
            login_browser_for_profile(Some(&codex_home), Some(" a ")),
            Some(LoginBrowser::Edge)
        );
        assert_eq!(
            login_browser_for_profile(Some(&codex_home), Some("b")),
            None
        );
        assert_eq!(
            login_browser_for_profile(Some(&codex_home), Some(" C ")),
            None
        );
        assert_eq!(login_browser_for_profile(Some(&codex_home), None), None);

        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn build_login_browser_command_uses_existing_browser_profile() {
        let launch = LoginBrowserLaunch {
            browser: LoginBrowser::Edge,
            executable: PathBuf::from(r"C:\Program Files\Microsoft\Edge\Application\msedge.exe"),
        };
        let command = build_login_browser_command(&launch, "https://auth.example/login");
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(command.get_program(), launch.executable.as_os_str());
        assert_eq!(args, vec!["https://auth.example/login".to_string()]);
        assert!(!args.iter().any(|arg| arg.starts_with("--user-data-dir")));
        assert!(!args.iter().any(|arg| arg == "--new-window"));
    }

    #[test]
    fn login_browser_app_server_launch_routes_explicit_edge_metadata_without_legacy_profile_dir() {
        let _guard = env_guard();
        let codex_home = temp_codex_home("browser-app-server-launch");
        let browser_env_vars: &[&'static str] = &["CODEX_SWITCH_EDGE_PATH", "LOCALAPPDATA"];
        let _env = EnvVarGuard::capture(browser_env_vars);
        let browser_dir = codex_home.join("browsers");
        fs::create_dir_all(&browser_dir).unwrap();
        let edge_path = browser_dir.join("msedge.exe");
        fs::write(&edge_path, "").unwrap();
        std::env::set_var("CODEX_SWITCH_EDGE_PATH", &edge_path);
        std::env::remove_var("LOCALAPPDATA");
        write_profile_metadata_with_login_browser(
            &codex_home,
            "a",
            "edge-enabled@profile.example",
            Some("edge"),
        );

        let real_codex_path = codex_home.join("codex.exe");
        let runtime_codex_home = codex_home
            .join("account_backup")
            .join("windows")
            .join("login_runtime");
        let legacy_browser_profile_dir = codex_home
            .join("account_backup")
            .join("windows")
            .join("login_browser_profiles");

        let (a_command, a_launch) = resolve_login_browser_app_server_launch(
            &real_codex_path,
            &codex_home,
            &runtime_codex_home,
            Some("a"),
        )
        .unwrap()
        .expect("the Edge-marked profile should use app-server browser login");
        let a_args = a_command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let a_envs = a_command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(a_launch.browser, LoginBrowser::Edge);
        assert_eq!(a_launch.executable, edge_path);
        assert_eq!(a_command.get_program(), real_codex_path.as_os_str());
        assert_eq!(a_args, vec!["app-server".to_string()]);
        assert_eq!(
            a_command.get_current_dir(),
            Some(runtime_codex_home.as_path())
        );
        assert!(a_envs.iter().any(|(key, value)| {
            key == "CODEX_HOME"
                && value.as_deref() == Some(runtime_codex_home.to_string_lossy().as_ref())
        }));

        assert!(
            resolve_login_browser_app_server_launch(
                &real_codex_path,
                &codex_home,
                &runtime_codex_home,
                Some("b")
            )
            .unwrap()
            .is_none(),
            "slot b alone should no longer force Chrome"
        );
        assert!(
            resolve_login_browser_app_server_launch(
                &real_codex_path,
                &codex_home,
                &runtime_codex_home,
                Some("c")
            )
            .unwrap()
            .is_none(),
            "slot c alone should no longer force Edge"
        );
        assert!(
            !legacy_browser_profile_dir.exists(),
            "browser login routing must not create the old login_browser_profiles directory"
        );

        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn looks_like_codex_version_requires_codex_binary_name() {
        assert!(looks_like_codex_version("codex-cli 0.142.0-alpha.6"));
        assert!(looks_like_codex_version("codex 0.142.0-alpha.6"));
        assert!(!looks_like_codex_version(
            "Microsoft Windows [Version 10.0]"
        ));
        assert!(!looks_like_codex_version(""));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn probe_codex_version_rejects_non_codex_zero_exit_exe() {
        use super::probe_codex_version;

        let cmd_exe = std::env::var_os("COMSPEC")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("SystemRoot")
                    .map(PathBuf::from)
                    .map(|root| root.join("System32").join("cmd.exe"))
            })
            .filter(|path| path.is_file())
            .expect("Windows tests require cmd.exe");

        assert_eq!(probe_codex_version(&cmd_exe), None);
    }

    // Runs on the Linux `cargo test --lib` job (the win module compiles on
    // non-macOS): a `#!/bin/sh` candidate is spawnable there, and
    // `hide_console_window` is a no-op off Windows. Pins the three
    // behaviours auto-detect depends on: a non-file is rejected without
    // spawning, a binary that runs but exits non-zero is rejected (broken
    // install), and only a zero-exit Codex version line is accepted.
    #[cfg(unix)]
    #[test]
    fn probe_codex_version_rejects_missing_and_failing_captures_zero_exit() {
        use super::probe_codex_version;
        use std::os::unix::fs::PermissionsExt;

        let codex_home = temp_codex_home("probe-runnable");
        fs::create_dir_all(&codex_home).unwrap();

        // (a) non-file path → None, never spawned.
        assert_eq!(
            probe_codex_version(&codex_home.join("does-not-exist")),
            None
        );

        let set_exec = |path: &std::path::Path| {
            let mut perm = fs::metadata(path).unwrap().permissions();
            perm.set_mode(0o755);
            fs::set_permissions(path, perm).unwrap();
        };

        // (b) exists + runs but exits non-zero → None (broken install).
        let bad = codex_home.join("bad-codex");
        fs::write(&bad, "#!/bin/sh\nexit 3\n").unwrap();
        set_exec(&bad);
        assert_eq!(probe_codex_version(&bad), None);

        // (c) exists + exits zero, prints a version → Some(version).
        let good = codex_home.join("good-codex");
        fs::write(&good, "#!/bin/sh\necho codex-cli 0.133.0\n").unwrap();
        set_exec(&good);
        assert_eq!(
            probe_codex_version(&good).as_deref(),
            Some("codex-cli 0.133.0")
        );

        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn discover_real_codex_cli_path_prefers_cmd_and_skips_managed_shim() {
        let _guard = env_guard();
        let codex_home = temp_codex_home("discover-real-cli");
        let _env =
            EnvVarGuard::capture(&["PATH", "CODEX_EXE", "CODEX_INSTALL_DIR", "LOCALAPPDATA"]);
        let managed_bin = codex_home.join("bin");
        let npm_dir = codex_home.join("npm");
        fs::create_dir_all(&managed_bin).unwrap();
        fs::create_dir_all(&npm_dir).unwrap();
        fs::write(managed_bin.join("codex.cmd"), "@echo off\r\n").unwrap();
        fs::write(npm_dir.join("codex"), "#!/bin/sh\n").unwrap();
        write_runnable_codex(&npm_dir.join("codex.cmd"), "codex-cli 0.133.0");

        std::env::remove_var("CODEX_EXE");
        std::env::remove_var("CODEX_INSTALL_DIR");
        std::env::set_var("LOCALAPPDATA", codex_home.join("local-app-data"));
        std::env::set_var(
            "PATH",
            std::env::join_paths([managed_bin.clone(), npm_dir.clone()]).unwrap(),
        );

        let resolved = discover_real_codex_cli_path(Some(&managed_bin.join("codex.cmd")));

        assert_same_existing_path(resolved, npm_dir.join("codex.cmd"));
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn discover_real_codex_cli_path_skips_windows_apps_aliases() {
        let _guard = env_guard();
        let codex_home = temp_codex_home("discover-real-cli-windowsapps");
        let _env =
            EnvVarGuard::capture(&["PATH", "CODEX_EXE", "CODEX_INSTALL_DIR", "LOCALAPPDATA"]);
        let managed_bin = codex_home.join("bin");
        let alias_dir = codex_home
            .join("AppData")
            .join("Local")
            .join("Microsoft")
            .join("WindowsApps");
        let npm_dir = codex_home.join("npm");
        fs::create_dir_all(&managed_bin).unwrap();
        fs::create_dir_all(&alias_dir).unwrap();
        fs::create_dir_all(&npm_dir).unwrap();
        fs::write(managed_bin.join("codex.cmd"), "@echo off\r\n").unwrap();
        fs::write(alias_dir.join("codex.exe"), "alias").unwrap();
        write_runnable_codex(&npm_dir.join("codex.cmd"), "codex-cli 0.133.0");

        std::env::remove_var("CODEX_EXE");
        std::env::remove_var("CODEX_INSTALL_DIR");
        std::env::set_var("LOCALAPPDATA", codex_home.join("local-app-data"));
        std::env::set_var(
            "PATH",
            std::env::join_paths([alias_dir.clone(), npm_dir.clone()]).unwrap(),
        );

        let resolved = discover_real_codex_cli_path(Some(&managed_bin.join("codex.cmd")));

        assert_same_existing_path(resolved, npm_dir.join("codex.cmd"));
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn windows_hook_syncs_root_base_url_only_for_api_key_profiles() {
        let codex_home = temp_codex_home("hook-base-url-auth-mode");
        let backup_root = codex_home.join("account_backup");
        let chat_dir = backup_root.join("chat");
        let api_dir = backup_root.join("api");
        fs::create_dir_all(&chat_dir).unwrap();
        fs::create_dir_all(&api_dir).unwrap();
        fs::write(chat_dir.join("auth.json"), r#"{"auth_mode":"chatgpt"}"#).unwrap();
        fs::write(
            chat_dir.join("profile.json"),
            r#"{"folder_name":"chat","openai_base_url":"https://chat.example/v1"}"#,
        )
        .unwrap();
        fs::write(api_dir.join("auth.json"), r#"{"auth_mode":"apikey"}"#).unwrap();
        fs::write(
            api_dir.join("profile.json"),
            r#"{"folder_name":"api","openai_base_url":"https://api.example/v1"}"#,
        )
        .unwrap();
        fs::write(
            codex_home.join("config.toml"),
            "openai_base_url = \"https://stale.example/v1\"\nmodel = \"gpt-5.4\"\n",
        )
        .unwrap();
        let hooks = WindowsPlatformHooks;

        hooks
            .sync_root_openai_base_url_for_profile("chat", Some(&codex_home))
            .unwrap();
        let config = fs::read_to_string(codex_home.join("config.toml")).unwrap();
        assert!(!config.contains("openai_base_url"));
        assert!(config.contains("model = \"gpt-5.4\""));

        hooks
            .sync_root_openai_base_url_for_profile("api", Some(&codex_home))
            .unwrap();
        let config = fs::read_to_string(codex_home.join("config.toml")).unwrap();
        assert!(config.contains("openai_base_url = \"https://api.example/v1\""));
        assert!(config.contains("model = \"gpt-5.4\""));
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn where_codex_candidate_paths_times_out_hanging_where() {
        let _guard = env_guard();
        let codex_home = temp_codex_home("where-timeout");
        let _env = EnvVarGuard::capture(&["CODEX_SWITCH_TEST_WHERE_EXE"]);
        fs::create_dir_all(&codex_home).unwrap();
        let fake_where = codex_home.join("where.exe");
        write_hanging_where_exe(&fake_where);
        std::env::set_var("CODEX_SWITCH_TEST_WHERE_EXE", &fake_where);

        let started = Instant::now();
        let paths = where_codex_candidate_paths_with_timeout(Duration::from_millis(100));

        assert!(paths.is_empty());
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "where timeout helper should return promptly"
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn redetect_runnable_codex_cli_paths_skips_hanging_where() {
        let _guard = env_guard();
        let codex_home = temp_codex_home("redetect-where-timeout");
        let _env = EnvVarGuard::capture(&[
            "PATH",
            "CODEX_EXE",
            "CODEX_INSTALL_DIR",
            "LOCALAPPDATA",
            "APPDATA",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "CODEX_SWITCH_TEST_WHERE_EXE",
        ]);
        fs::create_dir_all(&codex_home).unwrap();
        let fake_where = codex_home.join("where.exe");
        write_hanging_where_exe(&fake_where);
        std::env::set_var("CODEX_SWITCH_TEST_WHERE_EXE", &fake_where);
        std::env::remove_var("PATH");
        std::env::remove_var("CODEX_EXE");
        std::env::remove_var("CODEX_INSTALL_DIR");
        std::env::remove_var("LOCALAPPDATA");
        std::env::remove_var("APPDATA");
        std::env::remove_var("ProgramFiles");
        std::env::remove_var("ProgramFiles(x86)");

        let started = Instant::now();
        let candidates = redetect_runnable_codex_cli_paths(Some(&codex_home));

        assert!(candidates.is_empty());
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "redetect should be bounded by the where timeout, not the hanging process"
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn suggested_codex_cli_paths_scans_nvm_fnm_and_asdf_without_deep_recursion() {
        let _guard = env_guard();
        let codex_home = temp_codex_home("version-manager-candidates");
        let env_names = [
            "PATH",
            "CODEX_EXE",
            "CODEX_INSTALL_DIR",
            "LOCALAPPDATA",
            "APPDATA",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "NVM_HOME",
            "NVM_DIR",
            "NVM_BIN",
            "NVM_SYMLINK",
            "FNM_DIR",
            "FNM_MULTISHELL_PATH",
            "ASDF_DATA_DIR",
            "USERPROFILE",
            "CODEX_SWITCH_TEST_WHERE_EXE",
        ];
        let _env = EnvVarGuard::capture(&env_names);
        let root = codex_home.join("managers");
        let nvm_home = root.join("nvm");
        let fnm_dir = root.join("fnm");
        let asdf_dir = root.join("asdf");

        let nvm_old = nvm_home.join("v20.11.1");
        let nvm_new = nvm_home.join("v22.14.0");
        fs::create_dir_all(&nvm_old).unwrap();
        fs::create_dir_all(&nvm_new).unwrap();
        write_runnable_codex(&nvm_old.join("codex.cmd"), "codex-cli 0.120.0");
        write_runnable_codex(&nvm_new.join("codex.cmd"), "codex-cli 0.122.0");
        fs::create_dir_all(nvm_new.join("nested").join("deeper")).unwrap();
        write_runnable_codex(
            &nvm_new.join("nested").join("deeper").join("codex.cmd"),
            "codex-cli 0.999.0",
        );

        let fnm_installation = fnm_dir
            .join("node-versions")
            .join("v21.7.0")
            .join("installation");
        fs::create_dir_all(&fnm_installation).unwrap();
        write_runnable_codex(&fnm_installation.join("codex.cmd"), "codex-cli 0.121.0");

        let asdf_bin = asdf_dir
            .join("installs")
            .join("nodejs")
            .join("22.10.0")
            .join("bin");
        fs::create_dir_all(&asdf_bin).unwrap();
        write_runnable_codex(&asdf_bin.join("codex.cmd"), "codex-cli 0.123.0");

        std::env::remove_var("PATH");
        std::env::remove_var("CODEX_EXE");
        std::env::remove_var("CODEX_INSTALL_DIR");
        std::env::set_var("LOCALAPPDATA", codex_home.join("local-app-data"));
        std::env::set_var("APPDATA", codex_home.join("app-data"));
        std::env::set_var("ProgramFiles", codex_home.join("program-files"));
        std::env::set_var("ProgramFiles(x86)", codex_home.join("program-files-x86"));
        std::env::set_var("NVM_HOME", &nvm_home);
        std::env::remove_var("NVM_DIR");
        std::env::remove_var("NVM_BIN");
        std::env::set_var("FNM_DIR", &fnm_dir);
        std::env::remove_var("FNM_MULTISHELL_PATH");
        std::env::set_var("ASDF_DATA_DIR", &asdf_dir);
        std::env::set_var("USERPROFILE", codex_home.join("user-profile"));
        std::env::remove_var("NVM_SYMLINK");

        let candidates = suggested_codex_cli_paths(Some(&codex_home));
        let manager_candidates = candidates
            .iter()
            .filter(|path| path.starts_with(&root))
            .cloned()
            .collect::<Vec<_>>();

        assert_eq!(
            manager_candidates,
            vec![
                nvm_new.join("codex.cmd"),
                nvm_old.join("codex.cmd"),
                fnm_installation.join("codex.cmd"),
                asdf_bin.join("codex.cmd"),
            ],
            "version-manager candidates must be stable and version-first"
        );
        assert!(
            !candidates
                .iter()
                .any(|path| path.ends_with("nested\\deeper\\codex.cmd")),
            "the scan must not recurse below the version installation root"
        );

        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn resolve_real_codex_cli_repairs_legacy_extensionless_state() {
        let codex_home = temp_codex_home("repair-legacy-state");
        let runtime_dir = codex_home.join("account_backup").join("windows");
        let npm_dir = codex_home.join("npm");
        fs::create_dir_all(&runtime_dir).unwrap();
        fs::create_dir_all(&npm_dir).unwrap();
        fs::write(npm_dir.join("codex"), "#!/bin/sh\n").unwrap();
        write_runnable_codex(&npm_dir.join("codex.cmd"), "codex-cli 0.133.0");
        let install_state = InstallState {
            real_codex_path: Some(npm_dir.join("codex").to_string_lossy().into_owned()),
            path_added_by_installer: false,
            user_codex_path: None,
        };
        fs::write(
            runtime_dir.join("install_state.json"),
            format!("{}\n", to_string_pretty(&install_state).unwrap()),
        )
        .unwrap();

        let resolved = resolve_real_codex_cli(Some(&codex_home));
        let persisted_state = load_install_state(Some(&codex_home));

        assert_same_existing_path(resolved, npm_dir.join("codex.cmd"));
        assert_same_existing_path(
            persisted_state.real_codex_path.map(PathBuf::from),
            npm_dir.join("codex.cmd"),
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn resolve_real_codex_cli_skips_cached_windows_apps_alias_and_repairs_state() {
        let _guard = env_guard();
        let codex_home = temp_codex_home("repair-windowsapps-state");
        let _env =
            EnvVarGuard::capture(&["PATH", "CODEX_EXE", "CODEX_INSTALL_DIR", "LOCALAPPDATA"]);
        let runtime_dir = codex_home.join("account_backup").join("windows");
        let alias_dir = codex_home
            .join("AppData")
            .join("Local")
            .join("Microsoft")
            .join("WindowsApps");
        let npm_dir = codex_home.join("npm");
        fs::create_dir_all(&runtime_dir).unwrap();
        fs::create_dir_all(&alias_dir).unwrap();
        fs::create_dir_all(&npm_dir).unwrap();
        fs::write(alias_dir.join("codex.exe"), "alias").unwrap();
        write_runnable_codex(&npm_dir.join("codex.cmd"), "codex-cli 0.133.0");
        let install_state = InstallState {
            real_codex_path: Some(alias_dir.join("codex.exe").to_string_lossy().into_owned()),
            path_added_by_installer: false,
            user_codex_path: None,
        };
        fs::write(
            runtime_dir.join("install_state.json"),
            format!("{}\n", to_string_pretty(&install_state).unwrap()),
        )
        .unwrap();

        std::env::remove_var("CODEX_EXE");
        std::env::remove_var("CODEX_INSTALL_DIR");
        std::env::set_var("LOCALAPPDATA", codex_home.join("local-app-data"));
        std::env::set_var("PATH", &npm_dir);

        let resolved = resolve_real_codex_cli(Some(&codex_home));
        let persisted_state = load_install_state(Some(&codex_home));

        assert_same_existing_path(resolved, npm_dir.join("codex.cmd"));
        assert_same_existing_path(
            persisted_state.real_codex_path.map(PathBuf::from),
            npm_dir.join("codex.cmd"),
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn resolve_real_codex_cli_skips_unrunnable_cached_shim_and_repairs_state() {
        let _guard = env_guard();
        let codex_home = temp_codex_home("repair-broken-shim-state");
        let _env =
            EnvVarGuard::capture(&["PATH", "CODEX_EXE", "CODEX_INSTALL_DIR", "LOCALAPPDATA"]);
        let runtime_dir = codex_home.join("account_backup").join("windows");
        let npm_dir = codex_home.join("npm");
        let local_app_data = codex_home.join("local-app-data");
        let official_bin = local_app_data
            .join("OpenAI")
            .join("Codex")
            .join("bin")
            .join("current-runtime");
        fs::create_dir_all(&runtime_dir).unwrap();
        fs::create_dir_all(&npm_dir).unwrap();
        fs::create_dir_all(&official_bin).unwrap();
        let broken_shim = npm_dir.join("codex.cmd");
        let official_codex = official_bin.join("codex.exe");
        write_failing_codex(&broken_shim);
        #[cfg(target_os = "windows")]
        write_runnable_codex_exe(&official_codex, "codex-cli 0.142.0-alpha.6");
        #[cfg(not(target_os = "windows"))]
        write_runnable_codex(&official_codex, "codex-cli 0.142.0-alpha.6");
        let install_state = InstallState {
            real_codex_path: Some(broken_shim.to_string_lossy().into_owned()),
            path_added_by_installer: false,
            user_codex_path: None,
        };
        fs::write(
            runtime_dir.join("install_state.json"),
            format!("{}\n", to_string_pretty(&install_state).unwrap()),
        )
        .unwrap();

        std::env::set_var("PATH", &npm_dir);
        std::env::set_var("LOCALAPPDATA", &local_app_data);
        std::env::remove_var("CODEX_EXE");
        std::env::remove_var("CODEX_INSTALL_DIR");

        let resolved = resolve_real_codex_cli(Some(&codex_home));
        let persisted_state = load_install_state(Some(&codex_home));

        assert_eq!(resolved, Some(official_codex.clone()));
        assert_eq!(
            persisted_state.real_codex_path,
            Some(official_codex.to_string_lossy().into_owned())
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn validate_user_codex_cli_path_rejects_existing_but_unrunnable_path() {
        let codex_home = temp_codex_home("reject-unrunnable-user-path");
        fs::create_dir_all(&codex_home).unwrap();
        let broken_shim = codex_home.join("codex.cmd");
        write_failing_codex(&broken_shim);

        let error = super::validate_user_codex_cli_path(
            Some(&codex_home),
            &broken_shim.display().to_string(),
        )
        .expect_err("broken shim should not be accepted");

        assert_eq!(error.error_code, "CODEX_CLI_PATH_NOT_RUNNABLE");
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn is_acceptable_real_codex_cli_path_rejects_windows_apps_aliases() {
        let alias_path =
            PathBuf::from(r"C:\Users\demo\AppData\Local\Microsoft\WindowsApps\codex.exe");

        assert!(!is_acceptable_real_codex_cli_path(&alias_path, None));
    }

    #[test]
    fn build_app_server_command_targets_runtime_codex_home() {
        let runtime_codex_home = temp_codex_home("app-server-command");
        let real_codex_path = runtime_codex_home.join("bin").join("codex.exe");
        let command = build_app_server_command(&real_codex_path, &runtime_codex_home);

        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let envs = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(command.get_program(), real_codex_path.as_os_str());
        assert_eq!(args, vec!["app-server".to_string()]);
        assert_eq!(
            command.get_current_dir(),
            Some(runtime_codex_home.as_path())
        );
        assert!(envs.iter().any(|(key, value)| {
            key == "CODEX_HOME"
                && value.as_deref() == Some(runtime_codex_home.to_string_lossy().as_ref())
        }));
    }

    #[test]
    fn resolve_windows_app_target_returns_windows_store_target() {
        let codex_home = temp_codex_home("windows-store-app-target");

        let target = resolve_windows_app_target();

        assert_eq!(
            target,
            AppLaunchTarget::WindowsStore(windows_store_shell_target(WINDOWS_STORE_APP_ID))
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn windows_store_process_query_is_package_scoped() {
        let tasklist = concat!(
            "\u{feff}\"ChatGPT.exe (App)\",\"46048\",\"514,408 K\",\"OpenAI.Codex_26.707.3748.0_x64__2p2nqsd0c76g0\"\r\n",
            "\"codex.exe (App)\",\"31664\",\"334,712 K\",\"OpenAI.Codex_26.707.3748.0_x64__2p2nqsd0c76g0\"\n",
            "\"ChatGPT.exe (App)\",\"777\",\"100 K\",\"OpenAI.ChatGPT_9.9.9.0_x64__2p2nqsd0c76g0\"\n",
            "INFO: No matching tasks are running.\n",
        );

        let processes = parse_windows_store_processes(tasklist).unwrap();
        assert_eq!(
            processes,
            vec![
                WindowsStoreProcess::new("codex.exe (App)", 31664),
                WindowsStoreProcess::new("ChatGPT.exe (App)", 46048),
            ]
        );
    }

    #[test]
    fn windows_store_process_query_rejects_malformed_codex_package_rows() {
        let malformed =
            "ChatGPT.exe (App),46048,514 K,OpenAI.Codex_26.707.3748.0_x64__2p2nqsd0c76g0\n";

        let error = parse_windows_store_processes(malformed).unwrap_err();

        assert_eq!(error.error_code, "APP_PROCESS_QUERY_FAILED");

        let shifted_package_column = concat!(
            "\"ChatGPT.exe (App)\",\"46048\",\"514 K\",",
            "\"OpenAI.Codex_26.707.3748.0_x64__2p2nqsd0c76g0\",\"unexpected\"\n",
        );
        let error = parse_windows_store_processes(shifted_package_column).unwrap_err();
        assert_eq!(error.error_code, "APP_PROCESS_QUERY_FAILED");
    }

    #[test]
    fn windows_store_ui_host_detection_ignores_tasklist_app_label() {
        assert!(WindowsStoreProcess::new("ChatGPT.exe (App)", 10).is_ui_host());
        assert!(WindowsStoreProcess::new("ChatGPT.exe (应用)", 11).is_ui_host());
        assert!(!WindowsStoreProcess::new("codex.exe (App)", 12).is_ui_host());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_store_process_handle_verification_rejects_unrelated_process() {
        assert!(open_verified_codex_process(std::process::id())
            .unwrap()
            .is_none());
    }

    enum FakeSnapshot {
        Processes(Vec<WindowsStoreProcess>),
        Error,
    }

    struct FakeAppProcessOps {
        snapshots: VecDeque<FakeSnapshot>,
        last_processes: Vec<WindowsStoreProcess>,
        terminations: Vec<(Vec<u32>, bool)>,
        sleeps: usize,
    }

    impl FakeAppProcessOps {
        fn new(snapshots: Vec<FakeSnapshot>) -> Self {
            Self {
                snapshots: snapshots.into(),
                last_processes: Vec::new(),
                terminations: Vec::new(),
                sleeps: 0,
            }
        }
    }

    impl AppProcessOps for FakeAppProcessOps {
        fn snapshot(&mut self) -> AppResult<Vec<WindowsStoreProcess>> {
            match self.snapshots.pop_front() {
                Some(FakeSnapshot::Processes(processes)) => {
                    self.last_processes = processes.clone();
                    Ok(processes)
                }
                Some(FakeSnapshot::Error) => Err(AppError::new(
                    "APP_PROCESS_QUERY_FAILED",
                    "scripted process query failure",
                )),
                None => Ok(self.last_processes.clone()),
            }
        }

        fn terminate(&mut self, process_ids: &[u32], force: bool) -> AppResult<()> {
            self.terminations.push((process_ids.to_vec(), force));
            Ok(())
        }

        fn sleep(&mut self, _duration: Duration) {
            self.sleeps += 1;
        }
    }

    fn ui_process(process_id: u32) -> WindowsStoreProcess {
        WindowsStoreProcess::new("ChatGPT.exe (App)", process_id)
    }

    fn backend_process(process_id: u32) -> WindowsStoreProcess {
        WindowsStoreProcess::new("codex.exe (App)", process_id)
    }

    #[test]
    fn app_start_wait_accepts_only_a_managed_package_snapshot() {
        let mut ops = FakeAppProcessOps::new(vec![
            FakeSnapshot::Processes(Vec::new()),
            FakeSnapshot::Processes(vec![backend_process(42)]),
        ]);

        wait_for_codex_app_start_with_ops(&mut ops, 3).unwrap();
        assert_eq!(ops.sleeps, 1);
    }

    #[test]
    fn app_start_wait_times_out_after_bounded_empty_snapshots() {
        let mut ops = FakeAppProcessOps::new(vec![
            FakeSnapshot::Processes(Vec::new()),
            FakeSnapshot::Processes(Vec::new()),
            FakeSnapshot::Processes(Vec::new()),
        ]);

        let error = wait_for_codex_app_start_with_ops(&mut ops, 3).unwrap_err();
        assert_eq!(error.error_code, "APP_RELAUNCH_FAILED");
        assert_eq!(ops.sleeps, 2);
    }

    #[test]
    fn app_start_wait_propagates_package_query_failure() {
        let mut ops = FakeAppProcessOps::new(vec![FakeSnapshot::Error]);
        let error = wait_for_codex_app_start_with_ops(&mut ops, 3).unwrap_err();
        assert_eq!(error.error_code, "APP_PROCESS_QUERY_FAILED");
        assert_eq!(ops.sleeps, 0);
    }

    #[test]
    fn windows_store_shutdown_closes_ui_before_backend_and_requires_stable_empty_snapshots() {
        let mut ops = FakeAppProcessOps::new(vec![
            FakeSnapshot::Processes(vec![ui_process(10), backend_process(20)]),
            FakeSnapshot::Processes(Vec::new()),
            FakeSnapshot::Processes(vec![backend_process(21)]),
            FakeSnapshot::Processes(Vec::new()),
            FakeSnapshot::Processes(Vec::new()),
        ]);

        assert!(quit_codex_app_with_ops(&mut ops).unwrap());
        assert_eq!(ops.terminations, vec![(vec![10], false)]);
        assert_eq!(ops.sleeps, 3);
    }

    #[test]
    fn windows_store_shutdown_rechecks_an_initial_empty_snapshot() {
        let mut ops = FakeAppProcessOps::new(vec![
            FakeSnapshot::Processes(Vec::new()),
            FakeSnapshot::Processes(vec![backend_process(20)]),
            FakeSnapshot::Processes(vec![backend_process(20)]),
            FakeSnapshot::Processes(Vec::new()),
            FakeSnapshot::Processes(Vec::new()),
        ]);

        assert!(!quit_codex_app_with_ops(&mut ops).unwrap());
        assert_eq!(ops.terminations, vec![(vec![20], true)]);
    }

    #[test]
    fn windows_store_force_shutdown_rescans_and_cleans_new_package_pids() {
        let mut ops = FakeAppProcessOps::new(vec![
            FakeSnapshot::Processes(vec![backend_process(20)]),
            FakeSnapshot::Processes(vec![backend_process(20)]),
            FakeSnapshot::Processes(vec![ui_process(21)]),
            FakeSnapshot::Processes(Vec::new()),
            FakeSnapshot::Processes(Vec::new()),
        ]);

        assert!(quit_codex_app_with_ops(&mut ops).unwrap());
        assert_eq!(ops.terminations, vec![(vec![20], true), (vec![21], true)]);
    }

    #[test]
    fn windows_store_shutdown_confirms_exit_after_the_last_force_attempt() {
        let mut snapshots = vec![FakeSnapshot::Processes(vec![backend_process(20)])];
        snapshots.extend((0..10).map(|_| FakeSnapshot::Processes(vec![backend_process(20)])));
        snapshots.push(FakeSnapshot::Processes(Vec::new()));
        snapshots.push(FakeSnapshot::Processes(Vec::new()));
        let mut ops = FakeAppProcessOps::new(snapshots);

        assert!(!quit_codex_app_with_ops(&mut ops).unwrap());
        assert_eq!(ops.terminations.len(), 10);
        assert!(ops.terminations.iter().all(|(_, force)| *force));
    }

    #[test]
    fn windows_store_shutdown_fails_closed_on_query_error() {
        let mut ops = FakeAppProcessOps::new(vec![FakeSnapshot::Error]);

        let error = quit_codex_app_with_ops(&mut ops).unwrap_err();

        assert_eq!(error.error_code, "APP_PROCESS_QUERY_FAILED");
        assert!(ops.terminations.is_empty());
    }

    #[test]
    fn windows_store_shutdown_errors_when_package_processes_never_exit() {
        let mut ops = FakeAppProcessOps::new(vec![FakeSnapshot::Processes(vec![
            ui_process(10),
            backend_process(20),
        ])]);

        let mut observation = AppExitObservation::default();
        let error = quit_codex_app_with_ops_observed(&mut ops, &mut observation).unwrap_err();

        assert_eq!(error.error_code, "APP_EXIT_FAILED");
        assert!(observation.app_was_running);
        assert!(observation.had_processes);
        assert_eq!(ops.terminations.first(), Some(&(vec![10], false)));
        assert!(ops.terminations.iter().skip(1).all(|(_, force)| *force));
    }
}
