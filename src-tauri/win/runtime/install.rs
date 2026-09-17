use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::errors::{AppError, AppResult};

use super::fs_ops::{atomic_copy_file, atomic_write_file, remove_path};
use super::paths::{
    get_backup_root, get_codex_home, get_install_state_file, get_runtime_dir, utc_timestamp,
    ACTIVE_MARKER_FILE, CURRENT_PROFILE_FILENAME, DEFAULT_PROFILES,
};
use super::process::{
    discover_real_codex_cli_path, hide_console_window, is_acceptable_real_codex_cli_path,
    load_install_state, resolve_windows_invokable_path, save_install_state, InstallState,
};
use super::profiles::resolve_current_profile;

const AUTH_TEMPLATE: &str = include_str!("../../../examples/account_backup/demo/auth.json.example");
const CLI_RUNTIME_FILENAME: &str = "codex_switch_cli.exe";
pub const RESTART_SHORTCUT_FILENAME: &str = "重启 Codex.lnk";
// The shortcut is created through `Shell.Application` (`GetLink` on a parsed
// folder item) instead of `WScript.Shell` because the WScript.Shell shortcut
// pipeline internally converts the .lnk path through the system ANSI codepage:
// on a non-CJK system locale (e.g. en-US) the CJK filename gets best-fit
// mapped to `?`, which is an illegal filename character, and `Save()` then
// throws FileNotFoundException. `Shell.Application` resolves items via PIDLs
// and stays Unicode end to end, so this works regardless of system locale.
const CREATE_RESTART_SHORTCUT_COMMAND: &str = "$ErrorActionPreference = 'Stop'; $lnkPath = $env:CODEX_SWITCH_SHORTCUT_PATH; [System.IO.File]::WriteAllBytes($lnkPath, [byte[]]@()); $lnk = (New-Object -ComObject Shell.Application).NameSpace([System.IO.Path]::GetDirectoryName($lnkPath)).ParseName([System.IO.Path]::GetFileName($lnkPath)).GetLink; $lnk.Path = $env:CODEX_SWITCH_SHORTCUT_TARGET; $lnk.Arguments = 'restart'; $lnk.SetIconLocation($env:CODEX_SWITCH_SHORTCUT_TARGET, 0); $lnk.Save($lnkPath)";

#[allow(dead_code)]
pub struct InstallSummary {
    pub seeded_auth: bool,
    pub placeholder_auth_files: Vec<PathBuf>,
    pub initialized_default_profile: bool,
    pub runtime_cli_path: PathBuf,
    pub managed_shim_path: PathBuf,
    pub path_added_by_installer: bool,
    pub path_changed: bool,
    pub real_codex_path: PathBuf,
}

pub struct UninstallSummary {
    pub removed_shim: bool,
    pub removed_install_state: bool,
    pub removed_runtime_cli: bool,
    pub removed_path_entry: bool,
    pub removed_restart_shortcut: bool,
    pub managed_shim_path: PathBuf,
    pub install_state_path: PathBuf,
    pub runtime_cli_path: PathBuf,
}

fn runtime_cli_path(codex_home: &Path) -> PathBuf {
    get_runtime_dir(Some(codex_home)).join(CLI_RUNTIME_FILENAME)
}

fn managed_shim_path(codex_home: &Path) -> PathBuf {
    codex_home.join("bin").join("codex.cmd")
}

fn normalize_windows_path_entry(entry: impl AsRef<str>) -> String {
    let mut normalized = entry
        .as_ref()
        .trim()
        .replace('/', "\\")
        .to_ascii_lowercase();
    while normalized.ends_with('\\') {
        normalized.pop();
    }
    normalized
}

fn split_windows_path(value: &str) -> Vec<String> {
    value
        .split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect()
}

fn ensure_dir_on_path_entries(entries: &[String], target: &Path) -> (Vec<String>, bool) {
    let target_text = target.to_string_lossy().into_owned();
    let target_key = normalize_windows_path_entry(&target_text);
    let filtered = entries
        .iter()
        .filter(|entry| normalize_windows_path_entry(entry) != target_key)
        .cloned()
        .collect::<Vec<_>>();

    let mut next_entries = Vec::with_capacity(filtered.len() + 1);
    next_entries.push(target_text);
    next_entries.extend(filtered);

    let changed = next_entries != entries;
    (next_entries, changed)
}

fn remove_dir_from_path_entries(entries: &[String], target: &Path) -> (Vec<String>, bool) {
    let target_key = normalize_windows_path_entry(target.to_string_lossy());
    let next_entries = entries
        .iter()
        .filter(|entry| normalize_windows_path_entry(entry) != target_key)
        .cloned()
        .collect::<Vec<_>>();

    let changed = next_entries.len() != entries.len();
    (next_entries, changed)
}

fn read_user_path_value() -> AppResult<String> {
    if !cfg!(target_os = "windows") {
        return Ok(String::new());
    }

    let mut command = Command::new("powershell");
    command.args([
        "-NoProfile",
        "-Command",
        "[Environment]::GetEnvironmentVariable('Path', 'User')",
    ]);

    read_user_path_value_with_command(command)
}

fn read_user_path_value_with_command(mut command: Command) -> AppResult<String> {
    let output = hide_console_window(&mut command)
        .output()
        .map_err(|error| {
            AppError::new(
                "PATH_READ_FAILED",
                format!("Failed to read user PATH: {error}"),
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(AppError::new(
            "PATH_READ_FAILED",
            if stderr.is_empty() {
                "Failed to read user PATH.".to_string()
            } else {
                stderr
            },
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn write_user_path_value(value: &str) -> AppResult<()> {
    if !cfg!(target_os = "windows") {
        let _ = value;
        return Ok(());
    }

    let mut command = Command::new("powershell");
    command.args([
        "-NoProfile",
        "-Command",
        "[Environment]::SetEnvironmentVariable('Path', $env:CODEX_SWITCH_USER_PATH, 'User')",
    ]);
    command.env("CODEX_SWITCH_USER_PATH", value);

    let output = hide_console_window(&mut command)
        .output()
        .map_err(|error| {
            AppError::new(
                "PATH_WRITE_FAILED",
                format!("Failed to update user PATH: {error}"),
            )
        })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(AppError::new(
        "PATH_WRITE_FAILED",
        if stderr.is_empty() {
            "Failed to update user PATH.".to_string()
        } else {
            stderr
        },
    ))
}

fn ensure_dir_on_user_path(path: &Path) -> AppResult<bool> {
    let entries = split_windows_path(&read_user_path_value()?);
    let (next_entries, changed) = ensure_dir_on_path_entries(&entries, path);
    if changed {
        write_user_path_value(&next_entries.join(";"))?;
    }
    Ok(changed)
}

fn remove_dir_from_user_path(path: &Path) -> AppResult<bool> {
    let entries = split_windows_path(&read_user_path_value()?);
    let (next_entries, changed) = remove_dir_from_path_entries(&entries, path);
    if changed {
        write_user_path_value(&next_entries.join(";"))?;
    }
    Ok(changed)
}

fn resolve_real_codex_path(codex_home: &Path) -> AppResult<PathBuf> {
    let managed_shim_path = managed_shim_path(codex_home);
    let state = load_install_state(Some(codex_home));
    if let Some(existing) = state.real_codex_path.as_deref() {
        let path = PathBuf::from(existing);
        if let Some(path) = resolve_windows_invokable_path(&path)
            .filter(|path| is_acceptable_real_codex_cli_path(path, Some(&managed_shim_path)))
        {
            return Ok(path);
        }
    }

    discover_real_codex_cli_path(Some(&managed_shim_path)).ok_or_else(|| {
        AppError::new(
            "REAL_CODEX_NOT_FOUND",
            "Unable to resolve the real Codex CLI. Make sure `codex` is installed first.",
        )
    })
}

fn has_initialized_active_profile(backup_root: &Path) -> bool {
    resolve_current_profile(backup_root).is_some()
}

pub(super) fn initialize_default_active_profile(backup_root: &Path) -> AppResult<()> {
    let current_profile_file = backup_root.join(CURRENT_PROFILE_FILENAME);
    atomic_write_file(&current_profile_file, "a\n").map_err(|error| {
        AppError::new(
            "FS_WRITE_FAILED",
            format!(
                "Failed to write current profile marker {}: {}",
                current_profile_file.display(),
                error.message
            ),
        )
    })?;

    let marker_path = backup_root.join("a").join(ACTIVE_MARKER_FILE);
    atomic_write_file(&marker_path, format!("activated_at={}\n", utc_timestamp())).map_err(
        |error| {
            AppError::new(
                "FS_WRITE_FAILED",
                format!(
                    "Failed to write active marker {}: {}",
                    marker_path.display(),
                    error.message
                ),
            )
        },
    )
}

pub(super) fn ensure_default_profiles(backup_root: &Path) -> AppResult<()> {
    for profile in DEFAULT_PROFILES {
        let profile_dir = backup_root.join(profile);
        fs::create_dir_all(&profile_dir).map_err(|error| {
            AppError::new(
                "FS_CREATE_FAILED",
                format!(
                    "Failed to create profile directory {}: {error}",
                    profile_dir.display()
                ),
            )
        })?;
    }

    Ok(())
}

pub(super) fn ensure_placeholder_auth_files(backup_root: &Path) -> AppResult<Vec<PathBuf>> {
    let mut created_files = Vec::new();
    for profile in DEFAULT_PROFILES {
        let auth_file = backup_root.join(profile).join("auth.json");
        if auth_file.is_file() {
            continue;
        }

        atomic_write_file(&auth_file, AUTH_TEMPLATE).map_err(|error| {
            AppError::new(
                "AUTH_TEMPLATE_WRITE_FAILED",
                format!(
                    "Failed to write placeholder auth {}: {}",
                    auth_file.display(),
                    error.message
                ),
            )
        })?;
        created_files.push(auth_file);
    }

    Ok(created_files)
}

pub(super) fn seed_default_profile(codex_home: &Path, backup_root: &Path) -> AppResult<bool> {
    let root_auth_file = codex_home.join("auth.json");
    if !root_auth_file.is_file() {
        return Ok(false);
    }

    let default_profile_auth_file = backup_root.join("a").join("auth.json");
    atomic_copy_file(&root_auth_file, &default_profile_auth_file).map_err(|error| {
        AppError::new(
            "FS_COPY_FAILED",
            format!(
                "Failed to seed default profile auth {} -> {}: {}",
                root_auth_file.display(),
                default_profile_auth_file.display(),
                error.message
            ),
        )
    })?;

    Ok(true)
}

fn write_codex_shim(shim_path: &Path) -> AppResult<()> {
    if let Some(parent) = shim_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            AppError::new(
                "FS_CREATE_FAILED",
                format!(
                    "Failed to create shim directory {}: {error}",
                    parent.display()
                ),
            )
        })?;
    }

    let shim_contents = format!(
        "@echo off\r\nsetlocal\r\nif not defined CODEX_HOME set \"CODEX_HOME=%USERPROFILE%\\\\.codex\"\r\n\"%CODEX_HOME%\\\\account_backup\\\\windows\\\\{CLI_RUNTIME_FILENAME}\" shim %*\r\nexit /b %ERRORLEVEL%\r\n"
    );
    atomic_write_file(shim_path, shim_contents).map_err(|error| {
        AppError::new(
            "SHIM_WRITE_FAILED",
            format!(
                "Failed to write command shim {}: {}",
                shim_path.display(),
                error.message
            ),
        )
    })
}

fn copy_runtime_cli(source_cli_path: &Path, target_cli_path: &Path) -> AppResult<()> {
    if let Some(parent) = target_cli_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            AppError::new(
                "FS_CREATE_FAILED",
                format!(
                    "Failed to create runtime directory {}: {error}",
                    parent.display()
                ),
            )
        })?;
    }

    if source_cli_path != target_cli_path {
        atomic_copy_file(source_cli_path, target_cli_path).map_err(|error| {
            AppError::new(
                "FS_COPY_FAILED",
                format!(
                    "Failed to copy CLI {} -> {}: {}",
                    source_cli_path.display(),
                    target_cli_path.display(),
                    error.message
                ),
            )
        })?;
    }

    Ok(())
}

pub fn refresh_install_state(codex_home: &Path) -> AppResult<()> {
    let managed_shim_path = managed_shim_path(codex_home);
    let previous_state = load_install_state(Some(codex_home));
    let real_codex_path = previous_state
        .real_codex_path
        .as_deref()
        .and_then(|path| resolve_windows_invokable_path(Path::new(path)))
        .filter(|path| is_acceptable_real_codex_cli_path(path, Some(&managed_shim_path)))
        .or_else(|| discover_real_codex_cli_path(Some(&managed_shim_path)))
        .map(|path| path.to_string_lossy().into_owned());

    let state = InstallState {
        real_codex_path,
        path_added_by_installer: previous_state.path_added_by_installer,
        user_codex_path: previous_state.user_codex_path,
    };
    save_install_state(Some(codex_home), &state);
    Ok(())
}

fn install_from_with_path_hook(
    source_cli_path: &Path,
    codex_home: Option<&Path>,
    ensure_dir_on_user_path_hook: impl Fn(&Path) -> AppResult<bool>,
) -> AppResult<InstallSummary> {
    let codex_home = codex_home.map(PathBuf::from).unwrap_or_else(get_codex_home);
    let backup_root = get_backup_root(Some(&codex_home));
    let runtime_dir = get_runtime_dir(Some(&codex_home));
    let managed_bin_dir = codex_home.join("bin");
    let managed_shim_path = managed_shim_path(&codex_home);
    let runtime_cli_path = runtime_cli_path(&codex_home);
    let real_codex_path = resolve_real_codex_path(&codex_home)?;

    fs::create_dir_all(&backup_root).map_err(|error| {
        AppError::new(
            "FS_CREATE_FAILED",
            format!(
                "Failed to create backup root {}: {error}",
                backup_root.display()
            ),
        )
    })?;

    ensure_default_profiles(&backup_root)?;
    let placeholder_auth_files = ensure_placeholder_auth_files(&backup_root)?;
    let seeded_auth = seed_default_profile(&codex_home, &backup_root)?;
    let mut initialized_default_profile = false;
    if seeded_auth && !has_initialized_active_profile(&backup_root) {
        initialize_default_active_profile(&backup_root)?;
        initialized_default_profile = true;
    }

    fs::create_dir_all(&runtime_dir).map_err(|error| {
        AppError::new(
            "FS_CREATE_FAILED",
            format!(
                "Failed to create runtime directory {}: {error}",
                runtime_dir.display()
            ),
        )
    })?;
    copy_runtime_cli(source_cli_path, &runtime_cli_path)?;
    write_codex_shim(&managed_shim_path)?;

    let added_to_path = ensure_dir_on_user_path_hook(&managed_bin_dir)?;
    let previous_state = load_install_state(Some(&codex_home));
    let state = InstallState {
        real_codex_path: Some(real_codex_path.to_string_lossy().into_owned()),
        path_added_by_installer: added_to_path || previous_state.path_added_by_installer,
        user_codex_path: previous_state.user_codex_path.clone(),
    };
    save_install_state(Some(&codex_home), &state);

    Ok(InstallSummary {
        seeded_auth,
        placeholder_auth_files,
        initialized_default_profile,
        runtime_cli_path,
        managed_shim_path,
        path_added_by_installer: added_to_path || previous_state.path_added_by_installer,
        path_changed: added_to_path,
        real_codex_path,
    })
}

pub fn install_from(
    source_cli_path: &Path,
    codex_home: Option<&Path>,
) -> AppResult<InstallSummary> {
    install_from_with_path_hook(source_cli_path, codex_home, ensure_dir_on_user_path)
}

#[allow(dead_code)]
pub fn install(codex_home: Option<&Path>) -> AppResult<InstallSummary> {
    let source_cli_path = std::env::var_os("CODEX_SWITCH_RELEASE_EXE")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .or_else(|| std::env::current_exe().ok())
        .ok_or_else(|| {
            AppError::new(
                "CLI_PATH_UNAVAILABLE",
                "Failed to resolve current CLI path.",
            )
        })?;
    if source_cli_path.is_file() {
        return install_from(&source_cli_path, codex_home);
    }

    Err(AppError::new(
        "CLI_PATH_UNAVAILABLE",
        "Failed to resolve current CLI path.",
    ))
}

pub fn install_current_exe(codex_home: Option<&Path>) -> AppResult<InstallSummary> {
    let source_cli_path = std::env::current_exe().map_err(|error| {
        AppError::new(
            "CLI_PATH_UNAVAILABLE",
            format!("Failed to resolve current CLI path: {error}"),
        )
    })?;

    install_from(&source_cli_path, codex_home)
}

fn is_directory_empty(path: &Path) -> bool {
    path.is_dir()
        && fs::read_dir(path)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(false)
}

fn uninstall_with_path_hook(
    remove_script: bool,
    codex_home: Option<&Path>,
    remove_dir_from_user_path_hook: impl Fn(&Path) -> AppResult<bool>,
    resolve_desktop_dir: impl FnOnce() -> AppResult<PathBuf>,
) -> AppResult<UninstallSummary> {
    let codex_home = codex_home.map(PathBuf::from).unwrap_or_else(get_codex_home);
    let runtime_dir = get_runtime_dir(Some(&codex_home));
    let install_state_path = get_install_state_file(Some(&codex_home));
    let install_state = load_install_state(Some(&codex_home));
    let bin_dir = codex_home.join("bin");
    let managed_shim_path = managed_shim_path(&codex_home);
    let runtime_cli_path = runtime_cli_path(&codex_home);

    let removed_shim = if managed_shim_path.exists() {
        remove_path(&managed_shim_path)?;
        true
    } else {
        false
    };

    let removed_install_state = if install_state_path.exists() {
        remove_path(&install_state_path)?;
        true
    } else {
        false
    };

    let removed_runtime_cli = if remove_script && runtime_cli_path.exists() {
        remove_path(&runtime_cli_path)?;
        true
    } else {
        false
    };

    if remove_script {
        let legacy_cli_script = runtime_dir.join("codex_switch.py");
        if legacy_cli_script.exists() {
            remove_path(&legacy_cli_script)?;
        }

        let legacy_common_script = runtime_dir.join("common.py");
        if legacy_common_script.exists() {
            remove_path(&legacy_common_script)?;
        }

        let legacy_cache_dir = runtime_dir.join("__pycache__");
        if legacy_cache_dir.exists() {
            remove_path(&legacy_cache_dir)?;
        }
    }

    let removed_path_entry =
        if install_state.path_added_by_installer && is_directory_empty(&bin_dir) {
            remove_dir_from_user_path_hook(&bin_dir)?
        } else {
            false
        };

    if is_directory_empty(&bin_dir) {
        let _ = fs::remove_dir(&bin_dir);
    }
    if is_directory_empty(&runtime_dir) {
        let _ = fs::remove_dir(&runtime_dir);
    }

    let removed_restart_shortcut = match resolve_desktop_dir() {
        Ok(desktop_dir) => {
            let shortcut_path = desktop_dir.join(RESTART_SHORTCUT_FILENAME);
            if shortcut_path.exists() {
                remove_path(&shortcut_path)?;
                true
            } else {
                false
            }
        }
        Err(_) => false,
    };

    Ok(UninstallSummary {
        removed_shim,
        removed_install_state,
        removed_runtime_cli,
        removed_path_entry,
        removed_restart_shortcut,
        managed_shim_path,
        install_state_path,
        runtime_cli_path,
    })
}

pub fn uninstall(remove_script: bool, codex_home: Option<&Path>) -> AppResult<UninstallSummary> {
    uninstall_with_path_hook(
        remove_script,
        codex_home,
        remove_dir_from_user_path,
        resolve_user_desktop_dir,
    )
}

fn resolve_user_desktop_dir() -> AppResult<PathBuf> {
    let mut command = Command::new("powershell");
    command.args([
        "-NoProfile",
        "-Command",
        "[Environment]::GetFolderPath('Desktop')",
    ]);

    let output = hide_console_window(&mut command).output().map_err(|error| {
        AppError::new(
            "DESKTOP_PATH_UNAVAILABLE",
            format!("Failed to resolve the Desktop directory: {error}"),
        )
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(AppError::new(
            "DESKTOP_PATH_UNAVAILABLE",
            if stderr.is_empty() {
                "Failed to resolve the Desktop directory.".to_string()
            } else {
                stderr
            },
        ));
    }

    let desktop = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if desktop.is_empty() {
        return Err(AppError::new(
            "DESKTOP_PATH_UNAVAILABLE",
            "Failed to resolve the Desktop directory.",
        ));
    }

    Ok(PathBuf::from(desktop))
}

fn run_shortcut_command(mut command: Command) -> AppResult<()> {
    let output = command.output().map_err(|error| {
        AppError::new(
            "SHORTCUT_CREATE_FAILED",
            format!("Failed to create desktop shortcut: {error}"),
        )
    })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(AppError::new(
        "SHORTCUT_CREATE_FAILED",
        if stderr.is_empty() {
            "Failed to create desktop shortcut.".to_string()
        } else {
            stderr
        },
    ))
}

fn install_restart_shortcut_with_hooks(
    codex_home: Option<&Path>,
    resolve_desktop_dir: impl FnOnce() -> AppResult<PathBuf>,
    run_command: impl FnOnce(Command) -> AppResult<()>,
) -> AppResult<PathBuf> {
    let codex_home = codex_home.map(PathBuf::from).unwrap_or_else(get_codex_home);
    let target_path = runtime_cli_path(&codex_home);
    if !target_path.is_file() {
        return Err(AppError::new(
            "RUNTIME_CLI_NOT_FOUND",
            format!(
                "Runtime CLI not found at {}. Run `codex_switch_cli install` first.",
                target_path.display()
            ),
        ));
    }

    let desktop_dir = resolve_desktop_dir()?;
    if !desktop_dir.is_dir() {
        return Err(AppError::new(
            "DESKTOP_PATH_UNAVAILABLE",
            format!(
                "Desktop directory is unavailable: {}",
                desktop_dir.display()
            ),
        ));
    }

    let shortcut_path = desktop_dir.join(RESTART_SHORTCUT_FILENAME);
    let mut command = Command::new("powershell");
    command.args(["-NoProfile", "-Command", CREATE_RESTART_SHORTCUT_COMMAND]);
    command.env("CODEX_SWITCH_SHORTCUT_PATH", &shortcut_path);
    command.env("CODEX_SWITCH_SHORTCUT_TARGET", &target_path);
    hide_console_window(&mut command);
    run_command(command)?;

    if !shortcut_path.is_file() {
        return Err(AppError::new(
            "SHORTCUT_CREATE_FAILED",
            format!(
                "Desktop shortcut was not created at {}.",
                shortcut_path.display()
            ),
        ));
    }

    Ok(shortcut_path)
}

pub fn install_restart_shortcut(codex_home: Option<&Path>) -> AppResult<PathBuf> {
    install_restart_shortcut_with_hooks(
        codex_home,
        resolve_user_desktop_dir,
        run_shortcut_command,
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        ensure_dir_on_path_entries, install_from_with_path_hook, install_restart_shortcut_with_hooks,
        read_user_path_value_with_command, refresh_install_state,
        remove_dir_from_path_entries, uninstall_with_path_hook, CLI_RUNTIME_FILENAME,
    };
    use crate::errors::AppError;
    use crate::windows::{
        env_guard,
        process::{hide_console_window, load_install_state, InstallState},
        write_runnable_codex_cmd, EnvVarGuard,
    };

    fn temp_codex_home(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("codex-switch-install-{name}-{unique}"))
    }

    fn assert_same_existing_path(actual: Option<String>, expected: PathBuf) {
        let actual = actual.expect("expected a persisted CLI path");
        assert_eq!(
            fs::canonicalize(PathBuf::from(actual)).unwrap(),
            fs::canonicalize(expected).unwrap(),
        );
    }

    fn shell_command(script: &str) -> Command {
        if cfg!(target_os = "windows") {
            let program = std::env::var_os("COMSPEC")
                .unwrap_or_else(|| std::ffi::OsString::from(r"C:\Windows\System32\cmd.exe"));
            let mut command = Command::new(program);
            command.args(["/C", script]);
            command
        } else {
            let mut command = Command::new("/bin/sh");
            command.args(["-c", script]);
            command
        }
    }

    #[test]
    fn ensure_dir_on_path_entries_moves_target_to_front() {
        let target = PathBuf::from(r"C:\Users\demo\.codex\bin");
        let entries = vec![
            r"C:\Program Files\Codex".to_string(),
            r"c:\users\demo\.codex\bin".to_string(),
        ];

        let (next_entries, changed) = ensure_dir_on_path_entries(&entries, &target);

        assert!(changed);
        assert_eq!(next_entries[0], r"C:\Users\demo\.codex\bin");
        assert_eq!(next_entries.len(), 2);
    }

    #[test]
    fn ensure_dir_on_path_entries_adds_target_when_user_path_is_empty() {
        let target = PathBuf::from(r"C:\Users\demo\.codex\bin");

        let (next_entries, changed) = ensure_dir_on_path_entries(&[], &target);

        assert!(changed);
        assert_eq!(next_entries, vec![r"C:\Users\demo\.codex\bin".to_string()]);
    }

    #[test]
    fn remove_dir_from_path_entries_removes_target_case_insensitively() {
        let target = PathBuf::from(r"C:\Users\demo\.codex\bin");
        let entries = vec![
            r"C:\Program Files\Codex".to_string(),
            r"c:\users\demo\.codex\bin".to_string(),
        ];

        let (next_entries, changed) = remove_dir_from_path_entries(&entries, &target);

        assert!(changed);
        assert_eq!(next_entries, vec![r"C:\Program Files\Codex".to_string()]);
    }

    #[test]
    fn read_user_path_value_with_command_returns_path_read_failed_on_non_zero_exit() {
        let command = shell_command("echo synthetic path read failure 1>&2 && exit 7");

        let error = read_user_path_value_with_command(command).unwrap_err();

        assert_eq!(error.error_code, "PATH_READ_FAILED");
        assert!(error.message.contains("synthetic path read failure"));
    }

    #[test]
    fn read_user_path_value_with_command_treats_successful_empty_stdout_as_empty_path() {
        let command = shell_command("exit 0");

        let value = read_user_path_value_with_command(command).unwrap();

        assert_eq!(value, "");
    }

    #[test]
    fn install_from_copies_cli_and_writes_state() {
        let _guard = env_guard();
        let codex_home = temp_codex_home("install");
        let _env =
            EnvVarGuard::capture(&["PATH", "CODEX_EXE", "CODEX_INSTALL_DIR", "LOCALAPPDATA"]);
        let source_cli = codex_home.join("source").join(CLI_RUNTIME_FILENAME);
        fs::create_dir_all(source_cli.parent().unwrap()).unwrap();
        fs::write(&source_cli, "cli").unwrap();
        fs::write(codex_home.join("auth.json"), "seed-auth").unwrap();

        let npm_dir = codex_home.join("npm");
        fs::create_dir_all(&npm_dir).unwrap();
        write_runnable_codex_cmd(&npm_dir.join("codex.cmd"), "codex-cli 0.133.0");
        std::env::remove_var("CODEX_EXE");
        std::env::remove_var("CODEX_INSTALL_DIR");
        std::env::set_var("LOCALAPPDATA", codex_home.join("local-app-data"));
        std::env::set_var("PATH", &npm_dir);

        let result =
            install_from_with_path_hook(&source_cli, Some(&codex_home), |_| Ok(true)).unwrap();

        assert!(result.runtime_cli_path.is_file());
        assert!(result.managed_shim_path.is_file());
        assert_eq!(
            fs::read_to_string(codex_home.join("account_backup").join(".current_profile")).unwrap(),
            "a\n"
        );
        let state = load_install_state(Some(&codex_home));
        assert_same_existing_path(state.real_codex_path, npm_dir.join("codex.cmd"));
        assert!(state.path_added_by_installer);
        assert_eq!(state.user_codex_path, None);
        assert!(result.path_added_by_installer);
        assert!(result.path_changed);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn install_from_path_read_failure_does_not_write_path_added_state() {
        let _guard = env_guard();
        let codex_home = temp_codex_home("install-path-read-failure");
        let _env =
            EnvVarGuard::capture(&["PATH", "CODEX_EXE", "CODEX_INSTALL_DIR", "LOCALAPPDATA"]);
        let source_cli = codex_home.join("source").join(CLI_RUNTIME_FILENAME);
        fs::create_dir_all(source_cli.parent().unwrap()).unwrap();
        fs::write(&source_cli, "cli").unwrap();
        fs::write(codex_home.join("auth.json"), "seed-auth").unwrap();

        let npm_dir = codex_home.join("npm");
        fs::create_dir_all(&npm_dir).unwrap();
        write_runnable_codex_cmd(&npm_dir.join("codex.cmd"), "codex-cli 0.133.0");
        std::env::remove_var("CODEX_EXE");
        std::env::remove_var("CODEX_INSTALL_DIR");
        std::env::set_var("LOCALAPPDATA", codex_home.join("local-app-data"));
        std::env::set_var("PATH", &npm_dir);

        let result = install_from_with_path_hook(&source_cli, Some(&codex_home), |_| {
            Err(AppError::new(
                "PATH_READ_FAILED",
                "synthetic path read failure",
            ))
        });
        let error = match result {
            Ok(_) => panic!("install should fail when reading user PATH fails"),
            Err(error) => error,
        };

        assert_eq!(error.error_code, "PATH_READ_FAILED");
        assert_eq!(
            load_install_state(Some(&codex_home)),
            InstallState::default()
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn refresh_install_state_replaces_cached_windows_apps_alias() {
        let _guard = env_guard();
        let codex_home = temp_codex_home("refresh-install-state-windowsapps");
        let _env =
            EnvVarGuard::capture(&["PATH", "CODEX_EXE", "CODEX_INSTALL_DIR", "LOCALAPPDATA"]);
        let runtime_dir = codex_home.join("account_backup").join("windows");
        let managed_bin = codex_home.join("bin");
        let alias_dir = codex_home
            .join("AppData")
            .join("Local")
            .join("Microsoft")
            .join("WindowsApps");
        let npm_dir = codex_home.join("npm");
        fs::create_dir_all(&runtime_dir).unwrap();
        fs::create_dir_all(&managed_bin).unwrap();
        fs::create_dir_all(&alias_dir).unwrap();
        fs::create_dir_all(&npm_dir).unwrap();
        fs::write(managed_bin.join("codex.cmd"), "@echo off\r\n").unwrap();
        fs::write(alias_dir.join("codex.exe"), "alias").unwrap();
        write_runnable_codex_cmd(&npm_dir.join("codex.cmd"), "codex-cli 0.133.0");
        fs::write(
            runtime_dir.join("install_state.json"),
            format!(
                "{}\n",
                serde_json::to_string_pretty(&InstallState {
                    real_codex_path: Some(
                        alias_dir.join("codex.exe").to_string_lossy().into_owned()
                    ),
                    path_added_by_installer: false,
                    user_codex_path: None,
                })
                .unwrap()
            ),
        )
        .unwrap();

        std::env::remove_var("CODEX_EXE");
        std::env::remove_var("CODEX_INSTALL_DIR");
        std::env::set_var("LOCALAPPDATA", codex_home.join("local-app-data"));
        std::env::set_var(
            "PATH",
            std::env::join_paths([managed_bin.clone(), npm_dir.clone()]).unwrap(),
        );

        refresh_install_state(&codex_home).unwrap();
        let state = load_install_state(Some(&codex_home));

        assert_same_existing_path(state.real_codex_path, npm_dir.join("codex.cmd"));
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn uninstall_remove_script_removes_runtime_cli() {
        let codex_home = temp_codex_home("uninstall");
        let runtime_dir = codex_home.join("account_backup").join("windows");
        let bin_dir = codex_home.join("bin");
        fs::create_dir_all(&runtime_dir).unwrap();
        fs::create_dir_all(&bin_dir).unwrap();
        fs::write(
            runtime_dir.join("install_state.json"),
            "{\n  \"path_added_by_installer\": false\n}\n",
        )
        .unwrap();
        fs::write(runtime_dir.join(CLI_RUNTIME_FILENAME), "cli").unwrap();
        fs::write(bin_dir.join("codex.cmd"), "shim").unwrap();

        let summary = uninstall_with_path_hook(
            true,
            Some(&codex_home),
            |_| Ok(false),
            || Ok(codex_home.join("missing-desktop")),
        )
        .unwrap();

        assert!(summary.removed_shim);
        assert!(summary.removed_install_state);
        assert!(summary.removed_runtime_cli);
        assert!(!summary.removed_restart_shortcut);
        assert!(!runtime_dir.exists());
        let _ = fs::remove_dir_all(&codex_home);
    }

    fn plant_uninstallable_layout(codex_home: &std::path::Path) {
        let runtime_dir = codex_home.join("account_backup").join("windows");
        let bin_dir = codex_home.join("bin");
        fs::create_dir_all(&runtime_dir).unwrap();
        fs::create_dir_all(&bin_dir).unwrap();
        fs::write(
            runtime_dir.join("install_state.json"),
            "{\n  \"path_added_by_installer\": false\n}\n",
        )
        .unwrap();
        fs::write(runtime_dir.join(CLI_RUNTIME_FILENAME), "cli").unwrap();
        fs::write(bin_dir.join("codex.cmd"), "shim").unwrap();
    }

    #[test]
    fn uninstall_removes_fixed_restart_shortcut_from_injected_desktop() {
        let codex_home = temp_codex_home("uninstall-shortcut-present");
        let desktop_dir = temp_codex_home("uninstall-shortcut-present-desktop");
        plant_uninstallable_layout(&codex_home);
        fs::create_dir_all(&desktop_dir).unwrap();
        let shortcut_path = desktop_dir.join("重启 Codex.lnk");
        fs::write(&shortcut_path, "shortcut").unwrap();
        fs::write(desktop_dir.join("unrelated.txt"), "keep").unwrap();
        fs::write(desktop_dir.join("其他.lnk"), "other").unwrap();

        let summary = uninstall_with_path_hook(
            true,
            Some(&codex_home),
            |_| Ok(false),
            || Ok(desktop_dir.clone()),
        )
        .unwrap();

        assert!(summary.removed_restart_shortcut);
        assert!(!shortcut_path.exists());
        assert!(desktop_dir.join("unrelated.txt").is_file());
        assert!(desktop_dir.join("其他.lnk").is_file());
        assert!(summary.removed_shim);
        assert!(summary.removed_install_state);
        assert!(summary.removed_runtime_cli);
        assert!(!codex_home.join("account_backup").join("windows").exists());

        let _ = fs::remove_dir_all(&codex_home);
        let _ = fs::remove_dir_all(&desktop_dir);
    }

    #[test]
    fn uninstall_succeeds_when_restart_shortcut_is_absent() {
        let codex_home = temp_codex_home("uninstall-shortcut-absent");
        let desktop_dir = temp_codex_home("uninstall-shortcut-absent-desktop");
        plant_uninstallable_layout(&codex_home);
        fs::create_dir_all(&desktop_dir).unwrap();
        fs::write(desktop_dir.join("unrelated.txt"), "keep").unwrap();
        fs::write(desktop_dir.join("其他.lnk"), "other").unwrap();

        let summary = uninstall_with_path_hook(
            true,
            Some(&codex_home),
            |_| Ok(false),
            || Ok(desktop_dir.clone()),
        )
        .unwrap();

        assert!(!summary.removed_restart_shortcut);
        assert!(!desktop_dir.join("重启 Codex.lnk").exists());
        assert!(desktop_dir.join("unrelated.txt").is_file());
        assert!(desktop_dir.join("其他.lnk").is_file());
        assert_eq!(lnk_count(&desktop_dir), 1);
        assert!(summary.removed_shim);
        assert!(summary.removed_install_state);
        assert!(summary.removed_runtime_cli);
        assert!(!codex_home.join("account_backup").join("windows").exists());

        let _ = fs::remove_dir_all(&codex_home);
        let _ = fs::remove_dir_all(&desktop_dir);
    }

    #[test]
    fn uninstall_succeeds_when_desktop_dir_cannot_be_resolved() {
        let codex_home = temp_codex_home("uninstall-desktop-fail");
        plant_uninstallable_layout(&codex_home);

        let summary = uninstall_with_path_hook(
            true,
            Some(&codex_home),
            |_| Ok(false),
            || {
                Err(AppError::new(
                    "DESKTOP_PATH_UNAVAILABLE",
                    "synthetic desktop lookup failure",
                ))
            },
        )
        .unwrap();

        assert!(!summary.removed_restart_shortcut);
        assert!(summary.removed_shim);
        assert!(summary.removed_install_state);
        assert!(summary.removed_runtime_cli);

        let _ = fs::remove_dir_all(&codex_home);
    }

    fn planted_runtime_cli(codex_home: &std::path::Path) -> PathBuf {
        let runtime_cli = codex_home
            .join("account_backup")
            .join("windows")
            .join(CLI_RUNTIME_FILENAME);
        fs::create_dir_all(runtime_cli.parent().unwrap()).unwrap();
        fs::write(&runtime_cli, "cli").unwrap();
        runtime_cli
    }

    fn run_hook_command(mut command: Command) -> Result<(), AppError> {
        let output = hide_console_window(&mut command).output().map_err(|error| {
            AppError::new(
                "SHORTCUT_CREATE_FAILED",
                format!("Failed to create desktop shortcut: {error}"),
            )
        })?;
        if output.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(AppError::new(
            "SHORTCUT_CREATE_FAILED",
            if stderr.is_empty() {
                "Failed to create desktop shortcut.".to_string()
            } else {
                stderr
            },
        ))
    }

    fn read_shortcut(path: &std::path::Path) -> (String, String, String) {
        // Read back through `Shell.Application` for the same locale-independence
        // reason as CREATE_RESTART_SHORTCUT_COMMAND above: WScript.Shell would
        // mangle the CJK shortcut name on non-CJK system locales.
        let mut command = Command::new("powershell");
        command.args([
            "-NoProfile",
            "-Command",
            "$lnkPath = $env:CODEX_SWITCH_SHORTCUT_PATH; $lnk = (New-Object -ComObject Shell.Application).NameSpace([System.IO.Path]::GetDirectoryName($lnkPath)).ParseName([System.IO.Path]::GetFileName($lnkPath)).GetLink; $iconPath = ''; $iconIndex = $lnk.GetIconLocation([ref]$iconPath); Write-Output ('T=' + $lnk.Path); Write-Output ('A=' + $lnk.Arguments); Write-Output ('I=' + $iconPath + ',' + $iconIndex)",
        ]);
        command.env("CODEX_SWITCH_SHORTCUT_PATH", path);
        let output = hide_console_window(&mut command)
            .output()
            .expect("read shortcut properties");
        assert!(
            output.status.success(),
            "failed to read shortcut: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut target = String::new();
        let mut arguments = String::new();
        let mut icon = String::new();
        for line in stdout.lines() {
            if let Some(value) = line.strip_prefix("T=") {
                target = value.to_string();
            } else if let Some(value) = line.strip_prefix("A=") {
                arguments = value.to_string();
            } else if let Some(value) = line.strip_prefix("I=") {
                icon = value.to_string();
            }
        }
        (target, arguments, icon)
    }

    fn lnk_count(dir: &std::path::Path) -> usize {
        fs::read_dir(dir)
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .ok()
                    .and_then(|value| value.path().extension().map(|ext| ext == "lnk"))
                    .unwrap_or(false)
            })
            .count()
    }

    #[test]
    fn install_restart_shortcut_writes_target_path_and_restart_argument() {
        let _guard = env_guard();
        let codex_home = temp_codex_home("shortcut-create");
        let desktop_dir = temp_codex_home("shortcut-create-desktop");
        fs::create_dir_all(&desktop_dir).unwrap();
        let runtime_cli = planted_runtime_cli(&codex_home);

        let shortcut_path = install_restart_shortcut_with_hooks(
            Some(&codex_home),
            || Ok(desktop_dir.clone()),
            run_hook_command,
        )
        .unwrap();

        assert_eq!(
            shortcut_path.file_name().and_then(|name| name.to_str()),
            Some("重启 Codex.lnk")
        );
        assert_eq!(shortcut_path, desktop_dir.join("重启 Codex.lnk"));
        assert!(shortcut_path.is_file());
        assert_eq!(lnk_count(&desktop_dir), 1);

        let (target, arguments, icon) = read_shortcut(&shortcut_path);
        // The persisted target is normalized to the final long path form by
        // Shell.Application's Save(), while the planted runtime CLI keeps the
        // environment's spelling (short 8.3 TEMP prefix on CI runners, long
        // locally); compare canonicalized forms instead of raw strings.
        assert_same_existing_path(Some(target), runtime_cli);
        assert_eq!(arguments, "restart");
        assert!(
            icon.contains(CLI_RUNTIME_FILENAME) && icon.contains(",0"),
            "shortcut icon should use the runtime CLI exe: {icon}"
        );

        let _ = fs::remove_dir_all(&codex_home);
        let _ = fs::remove_dir_all(&desktop_dir);
    }

    #[test]
    fn install_restart_shortcut_overwrites_existing_shortcut_in_place() {
        let _guard = env_guard();
        let codex_home = temp_codex_home("shortcut-overwrite");
        let desktop_dir = temp_codex_home("shortcut-overwrite-desktop");
        fs::create_dir_all(&desktop_dir).unwrap();
        let runtime_cli = planted_runtime_cli(&codex_home);
        let shortcut_path = desktop_dir.join("重启 Codex.lnk");
        fs::write(&shortcut_path, "stale-shortcut").unwrap();
        fs::write(desktop_dir.join("unrelated.txt"), "keep").unwrap();

        install_restart_shortcut_with_hooks(
            Some(&codex_home),
            || Ok(desktop_dir.clone()),
            run_hook_command,
        )
        .unwrap();
        install_restart_shortcut_with_hooks(
            Some(&codex_home),
            || Ok(desktop_dir.clone()),
            run_hook_command,
        )
        .unwrap();

        assert_eq!(lnk_count(&desktop_dir), 1);
        assert!(desktop_dir.join("unrelated.txt").is_file());
        assert!(shortcut_path.is_file());
        let (target, arguments, _) = read_shortcut(&shortcut_path);
        // Same canonicalized-form comparison as above: the .lnk target is
        // stored normalized while the expected path may use a short spelling.
        assert_same_existing_path(Some(target), runtime_cli);
        assert_eq!(arguments, "restart");

        let _ = fs::remove_dir_all(&codex_home);
        let _ = fs::remove_dir_all(&desktop_dir);
    }

    #[test]
    fn install_restart_shortcut_fails_closed_when_runtime_cli_is_missing() {
        let codex_home = temp_codex_home("shortcut-missing-cli");
        let desktop_dir = temp_codex_home("shortcut-missing-cli-desktop");
        fs::create_dir_all(&desktop_dir).unwrap();

        let error = install_restart_shortcut_with_hooks(
            Some(&codex_home),
            || Ok(desktop_dir.clone()),
            |_| panic!("shortcut command must not run when runtime CLI is missing"),
        )
        .expect_err("missing runtime CLI must fail closed");

        assert_eq!(error.error_code, "RUNTIME_CLI_NOT_FOUND");
        assert!(
            error.message.contains("install"),
            "error should tell the user to run install first: {}",
            error.message
        );
        assert_eq!(lnk_count(&desktop_dir), 0);
        assert_eq!(fs::read_dir(&desktop_dir).unwrap().count(), 0);

        let _ = fs::remove_dir_all(&codex_home);
        let _ = fs::remove_dir_all(&desktop_dir);
    }

    #[test]
    fn install_restart_shortcut_fails_closed_when_desktop_dir_cannot_be_resolved() {
        let codex_home = temp_codex_home("shortcut-desktop-fail");
        planted_runtime_cli(&codex_home);

        let error = install_restart_shortcut_with_hooks(
            Some(&codex_home),
            || {
                Err(AppError::new(
                    "DESKTOP_PATH_UNAVAILABLE",
                    "synthetic desktop lookup failure",
                ))
            },
            |_| panic!("shortcut command must not run when desktop resolution fails"),
        )
        .expect_err("desktop lookup failure must fail closed");

        assert_eq!(error.error_code, "DESKTOP_PATH_UNAVAILABLE");
        assert!(error.message.contains("synthetic desktop lookup failure"));
        assert!(!codex_home.join("重启 Codex.lnk").exists());
        assert!(!codex_home
            .join("account_backup")
            .join("windows")
            .join("重启 Codex.lnk")
            .exists());

        let _ = fs::remove_dir_all(&codex_home);
    }
}
