use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::errors::{AppError, AppResult};

use super::paths::{
    autosave_timestamp, get_auto_save_root, get_current_profile_file, is_profile_dir,
    list_profile_dirs, utc_timestamp, ACTIVE_MARKER_FILE, PROFILE_METADATA_FILENAME,
};

const PROFILE_MANAGED_FILES: [&str; 2] = ["auth.json", PROFILE_METADATA_FILENAME];

fn should_ignore_entry(name: &str) -> bool {
    matches!(name, ".DS_Store" | ACTIVE_MARKER_FILE)
}

fn is_profile_managed_entry(name: &str) -> bool {
    PROFILE_MANAGED_FILES.contains(&name)
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

fn validate_profile_managed_file(path: &Path, name: &str) -> AppResult<bool> {
    if !path.exists() && !path.is_symlink() {
        return Ok(false);
    }

    let metadata = fs::symlink_metadata(path).map_err(|error| {
        AppError::new(
            "PROFILE_STATE_INVALID",
            format!(
                "Failed to inspect profile-managed entry {}: {error}",
                path.display()
            ),
        )
    })?;

    if metadata.file_type().is_symlink()
        || is_windows_reparse_point(&metadata)
        || !metadata.is_file()
    {
        return Err(AppError::new(
            "PROFILE_STATE_INVALID",
            format!(
                "Profile-managed entry {name} must be a regular file: {}",
                path.display()
            ),
        ));
    }

    Ok(true)
}

pub fn read_text_stripped(path: &Path) -> String {
    fs::read_to_string(path)
        .map(|content| content.trim().to_string())
        .unwrap_or_default()
}

pub fn remove_path(path: &Path) -> AppResult<()> {
    if !path.exists() && !path.is_symlink() {
        return Ok(());
    }

    if path.is_dir() && !path.is_symlink() {
        fs::remove_dir_all(path).map_err(|error| {
            AppError::new(
                "FS_REMOVE_FAILED",
                format!("Failed to remove directory {}: {error}", path.display()),
            )
        })
    } else {
        fs::remove_file(path).map_err(|error| {
            AppError::new(
                "FS_REMOVE_FAILED",
                format!("Failed to remove file {}: {error}", path.display()),
            )
        })
    }
}

pub fn copy_entry(src: &Path, dst: &Path) -> AppResult<()> {
    if src.is_dir() {
        replace_tree(src, dst)
    } else {
        remove_path(dst)?;
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                AppError::new(
                    "FS_CREATE_FAILED",
                    format!(
                        "Failed to create parent directory {}: {error}",
                        parent.display()
                    ),
                )
            })?;
        }

        fs::copy(src, dst).map_err(|error| {
            AppError::new(
                "FS_COPY_FAILED",
                format!(
                    "Failed to copy {} -> {}: {error}",
                    src.display(),
                    dst.display()
                ),
            )
        })?;
        Ok(())
    }
}

pub fn replace_tree(src: &Path, dst: &Path) -> AppResult<()> {
    remove_path(dst)?;
    fs::create_dir_all(dst).map_err(|error| {
        AppError::new(
            "FS_CREATE_FAILED",
            format!("Failed to create directory {}: {error}", dst.display()),
        )
    })?;

    for entry in fs::read_dir(src).map_err(|error| {
        AppError::new(
            "FS_READ_FAILED",
            format!("Failed to read directory {}: {error}", src.display()),
        )
    })? {
        let entry = entry.map_err(|error| {
            AppError::new(
                "FS_READ_FAILED",
                format!("Failed to read directory entry {}: {error}", src.display()),
            )
        })?;
        let source_path = entry.path();
        let target_path = dst.join(entry.file_name());
        copy_entry(&source_path, &target_path)?;
    }

    Ok(())
}

pub fn overlay_directory_contents(source_dir: &Path, target_dir: &Path) -> AppResult<()> {
    fs::create_dir_all(target_dir).map_err(|error| {
        AppError::new(
            "FS_CREATE_FAILED",
            format!(
                "Failed to create directory {}: {error}",
                target_dir.display()
            ),
        )
    })?;

    let mut managed_entries = Vec::new();

    for entry in fs::read_dir(source_dir).map_err(|error| {
        AppError::new(
            "FS_READ_FAILED",
            format!("Failed to read directory {}: {error}", source_dir.display()),
        )
    })? {
        let entry = entry.map_err(|error| {
            AppError::new(
                "FS_READ_FAILED",
                format!(
                    "Failed to read directory entry {}: {error}",
                    source_dir.display()
                ),
            )
        })?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if should_ignore_entry(name) {
            continue;
        }
        if !is_profile_managed_entry(name) {
            continue;
        }

        let source_path = entry.path();
        if validate_profile_managed_file(&source_path, name)? {
            managed_entries.push((name.to_string(), source_path));
        }
    }

    for (name, source_path) in managed_entries {
        copy_entry(&source_path, &target_dir.join(name))?;
    }

    Ok(())
}

pub fn backup_root_state_to_profile(
    profile: &str,
    codex_home: &Path,
    backup_root: &Path,
) -> AppResult<()> {
    let profile_dir = backup_root.join(profile);
    if !is_profile_dir(&profile_dir) {
        return Ok(());
    }

    let managed_names = BTreeSet::from(PROFILE_MANAGED_FILES.map(str::to_string));
    let mut managed_sources = Vec::new();

    for name in managed_names {
        let src = codex_home.join(&name);
        let exists = validate_profile_managed_file(&src, &name)?;
        managed_sources.push((name, src, exists));
    }

    for (name, src, exists) in managed_sources {
        let dst = profile_dir.join(&name);
        if exists {
            copy_entry(&src, &dst)?;
        } else {
            remove_path(&dst)?;
        }
    }

    Ok(())
}

pub fn autosave_auth(codex_home: &Path) -> AppResult<()> {
    let auth_file = codex_home.join("auth.json");
    if !auth_file.is_file() {
        return Ok(());
    }

    let auto_save_root = get_auto_save_root(Some(codex_home));
    let snapshot_dir = create_autosave_snapshot_dir(&auto_save_root)?;
    copy_entry(&auth_file, &snapshot_dir.join("auth.json"))
}

fn create_autosave_snapshot_dir(auto_save_root: &Path) -> AppResult<std::path::PathBuf> {
    create_autosave_snapshot_dir_with_base(auto_save_root, &autosave_timestamp())
}

fn create_autosave_snapshot_dir_with_base(
    auto_save_root: &Path,
    base: &str,
) -> AppResult<std::path::PathBuf> {
    fs::create_dir_all(auto_save_root).map_err(|error| {
        AppError::new(
            "FS_CREATE_FAILED",
            format!(
                "Failed to create autosave root {}: {error}",
                auto_save_root.display()
            ),
        )
    })?;

    for attempt in 0..32 {
        let candidate = if attempt == 0 {
            auto_save_root.join(base)
        } else {
            let suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0);
            auto_save_root.join(format!("{base}-{suffix}-{attempt}"))
        };
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(AppError::new(
                    "FS_CREATE_FAILED",
                    format!(
                        "Failed to create autosave directory {}: {error}",
                        candidate.display()
                    ),
                ));
            }
        }
    }

    Err(AppError::new(
        "FS_CREATE_FAILED",
        format!(
            "Failed to create a unique autosave directory under {}",
            auto_save_root.display()
        ),
    ))
}

pub fn set_active_marker(profile: &str, backup_root: &Path) -> AppResult<()> {
    for profile_dir in list_profile_dirs(backup_root) {
        remove_path(&profile_dir.join(ACTIVE_MARKER_FILE))?;
    }

    let marker = backup_root.join(profile).join(ACTIVE_MARKER_FILE);
    fs::write(&marker, format!("activated_at={}\n", utc_timestamp())).map_err(|error| {
        AppError::new(
            "FS_WRITE_FAILED",
            format!(
                "Failed to write active marker {}: {error}",
                marker.display()
            ),
        )
    })?;

    let current_profile_file = get_current_profile_file(backup_root.parent());
    fs::write(&current_profile_file, format!("{profile}\n")).map_err(|error| {
        AppError::new(
            "FS_WRITE_FAILED",
            format!(
                "Failed to write current profile marker {}: {error}",
                current_profile_file.display()
            ),
        )
    })
}

/// Clear every active-profile marker and the `.current_profile` pointer,
/// leaving no profile flagged as current.
pub fn clear_active_markers(backup_root: &Path) -> AppResult<()> {
    for profile_dir in list_profile_dirs(backup_root) {
        remove_path(&profile_dir.join(ACTIVE_MARKER_FILE))?;
    }
    remove_path(&get_current_profile_file(backup_root.parent()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("codex-switch-fs-ops-{name}-{unique}"));
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
    fn autosave_auth_keeps_existing_snapshots() {
        let codex_home = temp_root("autosave-keep-existing");
        write(&codex_home.join("auth.json"), "{}");
        let auto_save_root = get_auto_save_root(Some(&codex_home));
        for index in 1..=6 {
            let name = format!("2026010{index}-000000");
            write(&auto_save_root.join(name).join("auth.json"), "{}");
        }

        autosave_auth(&codex_home).unwrap();

        let snapshots = fs::read_dir(&auto_save_root)
            .unwrap()
            .filter(|entry| entry.as_ref().unwrap().path().is_dir())
            .collect::<Vec<_>>();
        assert_eq!(snapshots.len(), 7);
        for index in 1..=6 {
            let name = format!("2026010{index}-000000");
            assert!(auto_save_root.join(name).join("auth.json").is_file());
        }

        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn autosave_auth_uses_unique_directory_when_timestamp_exists() {
        let codex_home = temp_root("autosave-unique");
        let auto_save_root = get_auto_save_root(Some(&codex_home));
        let existing = auto_save_root.join("20260101-000000");
        write(&existing.join("auth.json"), "old");

        let created =
            create_autosave_snapshot_dir_with_base(&auto_save_root, "20260101-000000").unwrap();

        let snapshots = fs::read_dir(&auto_save_root)
            .unwrap()
            .filter(|entry| entry.as_ref().unwrap().path().is_dir())
            .collect::<Vec<_>>();
        assert_eq!(snapshots.len(), 2);
        assert_ne!(created, existing);
        assert_eq!(
            fs::read_to_string(existing.join("auth.json")).unwrap(),
            "old"
        );

        let _ = fs::remove_dir_all(&codex_home);
    }
}
