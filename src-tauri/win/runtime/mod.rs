#![cfg_attr(not(target_os = "windows"), allow(dead_code, unused_imports))]

pub mod bootstrap;
pub mod install;
pub mod process;
pub mod profile_actions;
pub mod profiles_index;
pub mod refresh_runtime;
pub mod switch;

pub use crate::shared::{
    config, fs_ops, login_runtime, metadata, openai_usage, paths, profiles, runtime_isolation,
    session_files, session_usage,
};

pub mod actions {
    pub use super::profile_actions::{
        add_profile, clear_profile_account, delete_profile, login_current_profile, login_profile,
        open_codex_app, rename_profile, restart_codex_app, update_profile_base_url,
    };
    #[cfg(not(test))]
    pub use super::profile_actions::{
        open_contact, open_profile_folder, open_releases, open_url, open_xiaohongshu,
    };
    pub use super::refresh_runtime::refresh_profile;
}

/// Exposed so the shared `codex_cli_path` Tauri command site can grab
/// the Windows resolver without depending on the platform module
/// directly.
pub fn codex_cli_resolver() -> &'static dyn crate::shared::codex_cli_path::CodexPathResolver {
    &process::WINDOWS_CODEX_PATH_RESOLVER
}

#[cfg(test)]
pub(crate) fn env_lock() -> &'static std::sync::Mutex<()> {
    use std::sync::{Mutex, OnceLock};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    ENV_LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(test)]
pub(crate) fn env_guard() -> std::sync::MutexGuard<'static, ()> {
    env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
pub(crate) fn write_runnable_codex_cmd(path: &std::path::Path, version: &str) {
    if cfg!(target_os = "windows") {
        std::fs::write(path, format!("@echo off\r\necho {version}\r\n")).unwrap();
    } else {
        std::fs::write(path, format!("#!/bin/sh\necho {version}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = std::fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(path, permissions).unwrap();
        }
    }
}

#[cfg(test)]
pub(crate) struct EnvVarGuard {
    saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

#[cfg(test)]
impl EnvVarGuard {
    pub(crate) fn capture(names: &[&'static str]) -> Self {
        Self {
            saved: names
                .iter()
                .map(|name| (*name, std::env::var_os(name)))
                .collect(),
        }
    }
}

#[cfg(test)]
impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        for (name, value) in self.saved.iter().rev() {
            if let Some(value) = value {
                std::env::set_var(name, value);
            } else {
                std::env::remove_var(name);
            }
        }
    }
}
