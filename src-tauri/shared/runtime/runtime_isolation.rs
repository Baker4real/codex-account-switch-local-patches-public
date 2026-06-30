//! Shared CODEX_HOME isolation primitives.
//!
//! The app spawns the real `codex` binary in two situations that need a
//! sandboxed CODEX_HOME instead of the user's live `~/.codex`:
//!
//!   * **Refresh** (`refresh_runtime`) — runs `codex exec` to provoke a
//!     session-file write so we can sample the latest rate limits without
//!     mutating the user's session history or AGENTS.md / skills config.
//!   * **Login** (`login_runtime`) — runs `codex login` for an arbitrary
//!     profile (not necessarily the active one) so the OAuth flow writes
//!     to a sandboxed `auth.json` we can copy into the target profile
//!     folder.
//!
//! Both flows want the *same* baseline: copy the small bookkeeping files
//! that let codex skip first-run setup (models cache, version pin,
//! global state, cap session id), avoid leaking AGENTS / skills / rules,
//! and never carry over the user's live `auth.json`. This module owns
//! that shared list so we don't drift between mac and win, or between
//! refresh and login.

use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::errors::{AppError, AppResult};

use super::fs_ops::{copy_entry, remove_path};

/// Files copied verbatim from the user's `~/.codex` into a sandboxed
/// runtime CODEX_HOME so codex CLI doesn't redo first-run setup work.
pub const RUNTIME_SHARED_FILES: [&str; 4] = [
    "models_cache.json",
    "version.json",
    ".codex-global-state.json",
    "cap_sid",
];

/// Directories lazily seeded (only when missing in the runtime home).
/// These are large and codex re-uses them safely across CODEX_HOMEs.
pub const RUNTIME_SHARED_DIRS: [&str; 3] = ["plugins", "cache", "sqlite"];

/// Files explicitly removed from the runtime home before each run so we
/// don't leak AGENTS / skills overrides into a sandboxed CLI invocation.
pub const RUNTIME_REMOVED_FILES: [&str; 1] = ["AGENTS.md"];

/// Directories explicitly removed from the runtime home for the same
/// reason as `RUNTIME_REMOVED_FILES`.
pub const RUNTIME_REMOVED_DIRS: [&str; 4] = ["rules", "skills", "vendor_imports", "memories"];

/// `auth.json` is intentionally NOT in the shared list — refresh overlays
/// the per-profile copy, login expects codex to write a fresh one. Keep
/// this constant for callers that need to know the auth file name.
pub const RUNTIME_AUTH_FILENAME: &str = "auth.json";

/// `profile.json` mirrors `auth.json`: refresh overlays the profile's
/// copy so codex can read existing session metadata; login starts blank.
pub const RUNTIME_PROFILE_METADATA_FILENAME: &str = "profile.json";

/// Create `runtime_home` (and parents) if missing. Codex CLI itself will
/// `mkdir -p` again, but we want any FS error to surface here with a
/// project-specific code instead of buried inside a child process.
fn ensure_runtime_home(runtime_home: &Path) -> AppResult<()> {
    fs::create_dir_all(runtime_home).map_err(|error| {
        AppError::new(
            "RUNTIME_HOME_CREATE_FAILED",
            format!(
                "Failed to create runtime CODEX_HOME {}: {error}",
                runtime_home.display()
            ),
        )
    })
}

fn is_windows_reparse_point(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

    #[cfg(not(windows))]
    {
        let _ = metadata;
        false
    }
}

fn is_symlink_or_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink() || is_windows_reparse_point(metadata)
}

fn validate_runtime_home(runtime_home: &Path) -> AppResult<()> {
    for ancestor in runtime_home.ancestors() {
        if ancestor.as_os_str().is_empty() || (!ancestor.exists() && !ancestor.is_symlink()) {
            continue;
        }
        let metadata = fs::symlink_metadata(ancestor).map_err(|error| {
            AppError::new(
                "RUNTIME_HOME_INVALID",
                format!(
                    "Failed to inspect runtime CODEX_HOME path {}: {error}",
                    ancestor.display()
                ),
            )
        })?;
        if is_symlink_or_reparse(&metadata) {
            return Err(AppError::new(
                "RUNTIME_HOME_INVALID",
                format!(
                    "Runtime CODEX_HOME path must not contain reparse/symlink entries: {}",
                    ancestor.display()
                ),
            ));
        }
    }

    let metadata = fs::symlink_metadata(runtime_home).map_err(|error| {
        AppError::new(
            "RUNTIME_HOME_INVALID",
            format!(
                "Failed to inspect runtime CODEX_HOME {}: {error}",
                runtime_home.display()
            ),
        )
    })?;
    if !metadata.is_dir() {
        return Err(AppError::new(
            "RUNTIME_HOME_INVALID",
            format!(
                "Runtime CODEX_HOME must be a plain directory: {}",
                runtime_home.display()
            ),
        ));
    }
    Ok(())
}

fn runtime_entry_path(runtime_home: &Path, entry_name: &str) -> AppResult<PathBuf> {
    let relative = Path::new(entry_name);
    let components = relative.components().collect::<Vec<_>>();
    if components.is_empty()
        || components
            .iter()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(AppError::new(
            "RUNTIME_PRUNE_UNSAFE_PATH",
            format!("Runtime prune entry must be relative: {entry_name}"),
        ));
    }

    validate_runtime_home(runtime_home)?;
    let mut current = runtime_home.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(part) = component else {
            unreachable!();
        };
        current.push(part);
        if !current.exists() && !current.is_symlink() {
            continue;
        }
        let metadata = fs::symlink_metadata(&current).map_err(|error| {
            AppError::new(
                "RUNTIME_PRUNE_UNSAFE_PATH",
                format!(
                    "Failed to inspect runtime entry {}: {error}",
                    current.display()
                ),
            )
        })?;
        if is_symlink_or_reparse(&metadata) {
            return Err(AppError::new(
                "RUNTIME_PRUNE_UNSAFE_PATH",
                format!(
                    "Refusing to prune reparse/symlink runtime entry: {}",
                    current.display()
                ),
            ));
        }
        if index + 1 < components.len() && !metadata.is_dir() {
            return Err(AppError::new(
                "RUNTIME_PRUNE_UNSAFE_PATH",
                format!(
                    "Runtime prune parent must be a directory: {}",
                    current.display()
                ),
            ));
        }
    }

    Ok(current)
}

fn remove_runtime_entry(runtime_home: &Path, entry_name: &str) -> AppResult<()> {
    let path = runtime_entry_path(runtime_home, entry_name)?;
    remove_path(&path)
}

/// Seed the shared bookkeeping files / dirs from the user's live
/// CODEX_HOME into a sandboxed runtime home. Idempotent: re-running
/// against an already-seeded runtime updates the shared files to
/// whatever is currently in `codex_home` and lazily fills any missing
/// shared dirs.
pub fn seed_runtime_shared_assets(codex_home: &Path, runtime_home: &Path) -> AppResult<()> {
    ensure_runtime_home(runtime_home)?;
    validate_runtime_home(runtime_home)?;

    for entry_name in RUNTIME_SHARED_FILES {
        let src = codex_home.join(entry_name);
        let dst = runtime_home.join(entry_name);
        if src.exists() {
            copy_entry(&src, &dst)?;
        } else {
            remove_path(&dst)?;
        }
    }

    for entry_name in RUNTIME_SHARED_DIRS {
        let src = codex_home.join(entry_name);
        let dst = runtime_home.join(entry_name);
        if src.exists() && !dst.exists() {
            copy_entry(&src, &dst)?;
        }
    }

    Ok(())
}

/// Strip out AGENTS / skills / rules / vendor / memories so a sandboxed
/// codex run behaves like a clean install regardless of what the user
/// has under `~/.codex`. Without this, e.g. an AGENTS.md that hijacks
/// behavior could distort a refresh / login flow.
pub fn prune_runtime_extra_features(runtime_home: &Path) -> AppResult<()> {
    for entry_name in RUNTIME_REMOVED_FILES {
        remove_runtime_entry(runtime_home, entry_name)?;
    }

    for entry_name in RUNTIME_REMOVED_DIRS {
        remove_runtime_entry(runtime_home, entry_name)?;
    }

    Ok(())
}

/// Clear any leftover auth state from a previous sandboxed run. Used by
/// the login flow before invoking `codex login` so the OAuth handler
/// writes into a guaranteed-empty `auth.json` instead of merging on top
/// of stale tokens from an aborted prior attempt.
pub fn clear_runtime_auth_state(runtime_home: &Path) -> AppResult<()> {
    remove_runtime_entry(runtime_home, RUNTIME_AUTH_FILENAME)?;
    remove_runtime_entry(runtime_home, RUNTIME_PROFILE_METADATA_FILENAME)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("codex-switch-runtime-isolation-{name}-{unique}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn seed_copies_shared_files_and_dirs_lazily() {
        let codex_home = temp_root("seed-copy-src");
        let runtime_home = temp_root("seed-copy-dst");
        write(&codex_home.join("models_cache.json"), "{}");
        write(&codex_home.join("version.json"), "{}");
        write(&codex_home.join("cache/state.json"), "{}");
        write(&codex_home.join("sqlite/state.db"), "sqlite");
        write(&codex_home.join("plugins/foo.js"), "// plugin");

        seed_runtime_shared_assets(&codex_home, &runtime_home).unwrap();

        assert!(runtime_home.join("models_cache.json").is_file());
        assert!(runtime_home.join("version.json").is_file());
        assert!(runtime_home.join("cache/state.json").is_file());
        assert!(runtime_home.join("sqlite/state.db").is_file());
        assert!(runtime_home.join("plugins/foo.js").is_file());
    }

    #[test]
    fn seed_removes_shared_file_in_runtime_when_absent_in_codex_home() {
        let codex_home = temp_root("seed-remove-src");
        let runtime_home = temp_root("seed-remove-dst");
        write(&runtime_home.join("models_cache.json"), "stale");

        seed_runtime_shared_assets(&codex_home, &runtime_home).unwrap();

        assert!(!runtime_home.join("models_cache.json").exists());
    }

    #[test]
    fn prune_removes_extras_even_when_already_absent() {
        let runtime_home = temp_root("prune");
        write(&runtime_home.join("AGENTS.md"), "# stale");
        write(&runtime_home.join("skills/test.toml"), "[task]");
        write(
            &runtime_home.join("plugins/cache/plugin/SKILL.md"),
            "# stale",
        );
        write(
            &runtime_home.join(".tmp/plugins/cache/plugin/SKILL.md"),
            "# stale",
        );
        write(
            &runtime_home.join(".tmp/bundled-marketplaces/staging/SKILL.md"),
            "# stale",
        );
        write(&runtime_home.join(".tmp/keep.txt"), "keep");

        prune_runtime_extra_features(&runtime_home).unwrap();

        assert!(!runtime_home.join("AGENTS.md").exists());
        assert!(!runtime_home.join("skills").exists());
        assert!(runtime_home.join("plugins/cache/plugin/SKILL.md").is_file());
        assert!(runtime_home
            .join(".tmp/plugins/cache/plugin/SKILL.md")
            .is_file());
        assert!(runtime_home
            .join(".tmp/bundled-marketplaces/staging/SKILL.md")
            .is_file());
        assert!(runtime_home.join(".tmp/keep.txt").is_file());

        // Idempotent.
        prune_runtime_extra_features(&runtime_home).unwrap();
    }

    #[test]
    fn runtime_prune_rejects_parent_components() {
        let runtime_home = temp_root("prune-parent");

        let error = remove_runtime_entry(&runtime_home, "../outside").unwrap_err();

        assert_eq!(error.error_code, "RUNTIME_PRUNE_UNSAFE_PATH");
    }

    #[test]
    fn clear_auth_state_removes_auth_and_profile_only() {
        let runtime_home = temp_root("clear-auth");
        write(&runtime_home.join("auth.json"), "{}");
        write(&runtime_home.join("profile.json"), "{}");
        write(&runtime_home.join("models_cache.json"), "{}");

        clear_runtime_auth_state(&runtime_home).unwrap();

        assert!(!runtime_home.join("auth.json").exists());
        assert!(!runtime_home.join("profile.json").exists());
        // Shared bookkeeping must be left intact.
        assert!(runtime_home.join("models_cache.json").is_file());
    }
}
