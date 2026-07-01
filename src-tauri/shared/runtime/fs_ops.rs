use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::errors::{AppError, AppResult};

use super::paths::{
    autosave_timestamp, get_auto_save_root, get_current_profile_file, is_profile_dir,
    list_profile_dirs, utc_timestamp, ACTIVE_MARKER_FILE, PROFILE_METADATA_FILENAME,
};

const PROFILE_MANAGED_FILES: [&str; 2] = ["auth.json", PROFILE_METADATA_FILENAME];
const PUBLISH_RETRY_COUNT: usize = 6;
const PUBLISH_RETRY_DELAY: Duration = Duration::from_millis(25);
static UNIQUE_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

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

fn unique_sibling_path(path: &Path, suffix: &str) -> std::path::PathBuf {
    let base_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("codex-switch-file");
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    let counter = UNIQUE_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_name = format!(
        ".{base_name}.{}.{}.{}.{}",
        std::process::id(),
        unique,
        counter,
        suffix
    );
    path.with_file_name(file_name)
}

fn ensure_parent_dir(path: &Path) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        if parent.as_os_str().is_empty() {
            return Ok(());
        }
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
    Ok(())
}

fn remove_staged_file(path: &Path) {
    let _ = fs::remove_file(path);
}

#[cfg(windows)]
fn replace_file_with_temp(temp_path: &Path, target_path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    fn wide_prefix(prefix: &str) -> Vec<u16> {
        prefix.encode_utf16().collect()
    }

    fn starts_with_wide(value: &[u16], prefix: &str) -> bool {
        value.starts_with(&wide_prefix(prefix))
    }

    fn wide(path: &Path) -> Vec<u16> {
        let mut encoded = path
            .as_os_str()
            .encode_wide()
            .map(|value| {
                if value == b'/' as u16 {
                    b'\\' as u16
                } else {
                    value
                }
            })
            .collect::<Vec<_>>();

        if path.is_absolute()
            && !starts_with_wide(&encoded, r"\\?\")
            && !starts_with_wide(&encoded, r"\\.\")
        {
            if encoded.starts_with(&[b'\\' as u16, b'\\' as u16]) {
                let mut prefixed = wide_prefix(r"\\?\UNC\");
                prefixed.extend_from_slice(&encoded[2..]);
                encoded = prefixed;
            } else {
                let mut prefixed = wide_prefix(r"\\?\");
                prefixed.extend_from_slice(&encoded);
                encoded = prefixed;
            }
        }

        encoded.push(0);
        encoded
    }

    let temp = wide(temp_path);
    let target = wide(target_path);
    let flags = MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH;
    let replaced = unsafe { MoveFileExW(temp.as_ptr(), target.as_ptr(), flags) };
    if replaced == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file_with_temp(temp_path: &Path, target_path: &Path) -> std::io::Result<()> {
    fs::rename(temp_path, target_path)
}

fn publish_temp_file(temp_path: &Path, target_path: &Path) -> AppResult<()> {
    if let Ok(metadata) = fs::symlink_metadata(target_path) {
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            remove_staged_file(temp_path);
            return Err(AppError::new(
                "FS_REPLACE_FAILED",
                format!(
                    "Refusing to replace directory {} with file {}",
                    target_path.display(),
                    temp_path.display()
                ),
            ));
        }
    }

    let mut last_error = None;
    for attempt in 0..PUBLISH_RETRY_COUNT {
        match replace_file_with_temp(temp_path, target_path) {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                if attempt + 1 < PUBLISH_RETRY_COUNT {
                    std::thread::sleep(PUBLISH_RETRY_DELAY);
                }
            }
        }
    }

    let error = last_error.unwrap_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::Other, "publish failed without error")
    });
    remove_staged_file(temp_path);
    Err(AppError::new(
        "FS_REPLACE_FAILED",
        format!(
            "Failed to publish {} -> {}: {error}",
            temp_path.display(),
            target_path.display()
        ),
    ))
}

pub fn atomic_write_file(path: &Path, contents: impl AsRef<[u8]>) -> AppResult<()> {
    ensure_parent_dir(path)?;
    let temp_path = unique_sibling_path(path, "tmp");
    fs::write(&temp_path, contents).map_err(|error| {
        remove_staged_file(&temp_path);
        AppError::new(
            "FS_WRITE_FAILED",
            format!("Failed to write temp file {}: {error}", temp_path.display()),
        )
    })?;

    publish_temp_file(&temp_path, path)
}

pub fn atomic_copy_file(src: &Path, dst: &Path) -> AppResult<()> {
    ensure_parent_dir(dst)?;
    let temp_path = unique_sibling_path(dst, "tmp");
    fs::copy(src, &temp_path).map_err(|error| {
        remove_staged_file(&temp_path);
        AppError::new(
            "FS_COPY_FAILED",
            format!(
                "Failed to copy {} -> {}: {error}",
                src.display(),
                dst.display()
            ),
        )
    })?;

    publish_temp_file(&temp_path, dst)
}

pub fn copy_entry(src: &Path, dst: &Path) -> AppResult<()> {
    if src.is_dir() {
        replace_tree(src, dst)
    } else {
        atomic_copy_file(src, dst).map_err(|error| {
            if error.error_code == "FS_COPY_FAILED" {
                return error;
            }
            AppError::new(
                "FS_COPY_FAILED",
                format!(
                    "Failed to copy {} -> {}: {}",
                    src.display(),
                    dst.display(),
                    error.message
                ),
            )
        })
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
    atomic_write_file(&marker, format!("activated_at={}\n", utc_timestamp())).map_err(|error| {
        AppError::new(
            "FS_WRITE_FAILED",
            format!(
                "Failed to write active marker {}: {}",
                marker.display(),
                error.message
            ),
        )
    })?;

    let current_profile_file = get_current_profile_file(backup_root.parent());
    atomic_write_file(&current_profile_file, format!("{profile}\n")).map_err(|error| {
        AppError::new(
            "FS_WRITE_FAILED",
            format!(
                "Failed to write current profile marker {}: {}",
                current_profile_file.display(),
                error.message
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

    fn staged_files_for(target: &Path) -> Vec<PathBuf> {
        let Some(parent) = target.parent() else {
            return Vec::new();
        };
        let Some(name) = target.file_name().and_then(|value| value.to_str()) else {
            return Vec::new();
        };
        fs::read_dir(parent)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value.starts_with(&format!(".{name}.")))
            })
            .collect()
    }

    #[test]
    fn copy_entry_keeps_existing_file_when_source_copy_fails() {
        let root = temp_root("copy-entry-preserves-target-on-copy-failure");
        let missing_source = root.join("missing-auth.json");
        let target = root.join("profile").join("auth.json");
        write(&target, "old auth");

        let error = copy_entry(&missing_source, &target).unwrap_err();

        assert_eq!(error.error_code, "FS_COPY_FAILED");
        assert_eq!(fs::read_to_string(&target).unwrap(), "old auth");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn copy_entry_replaces_existing_file_after_successful_stage() {
        let root = temp_root("copy-entry-replaces-existing-file");
        let source = root.join("source-auth.json");
        let target = root.join("profile").join("auth.json");
        write(&source, "new auth");
        write(&target, "old auth");

        copy_entry(&source, &target).unwrap();

        assert_eq!(fs::read_to_string(&target).unwrap(), "new auth");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn atomic_write_file_replaces_existing_file() {
        let root = temp_root("atomic-write-replaces-existing-file");
        let target = root.join("profile.json");
        write(&target, "old metadata");

        atomic_write_file(&target, "new metadata").unwrap();

        assert_eq!(fs::read_to_string(&target).unwrap(), "new metadata");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn atomic_write_file_refuses_to_replace_directory() {
        let root = temp_root("atomic-write-refuses-directory-target");
        let target = root.join("auth.json");
        fs::create_dir_all(&target).unwrap();

        let error = atomic_write_file(&target, "new auth").unwrap_err();

        assert_eq!(error.error_code, "FS_REPLACE_FAILED");
        assert!(target.is_dir());

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    fn atomic_write_file_keeps_existing_file_when_target_blocks_replace() {
        use std::fs::OpenOptions;
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;

        let root = temp_root("atomic-write-preserves-locked-target");
        let target = root.join("auth.json");
        write(&target, "old auth");
        let _locked = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&target)
            .unwrap();

        let error = atomic_write_file(&target, "new auth").unwrap_err();

        assert_eq!(error.error_code, "FS_REPLACE_FAILED");
        assert_eq!(fs::read_to_string(&target).unwrap(), "old auth");
        assert!(staged_files_for(&target).is_empty());

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    fn atomic_copy_file_keeps_existing_file_when_target_blocks_replace() {
        use std::fs::OpenOptions;
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;

        let root = temp_root("atomic-copy-preserves-locked-target");
        let source = root.join("new-auth.json");
        let target = root.join("auth.json");
        write(&source, "new auth");
        write(&target, "old auth");
        let _locked = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&target)
            .unwrap();

        let error = atomic_copy_file(&source, &target).unwrap_err();

        assert_eq!(error.error_code, "FS_REPLACE_FAILED");
        assert_eq!(fs::read_to_string(&target).unwrap(), "old auth");
        assert!(staged_files_for(&target).is_empty());

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    fn atomic_write_file_handles_long_absolute_paths() {
        let root = temp_root("atomic-write-long-path");
        let mut dir = root.clone();
        for index in 0..12 {
            dir = dir.join(format!("segment-{index:02}-abcdefghijklmnopqrstuvwxyz"));
        }
        let target = dir.join("profile.json");
        assert!(target.to_string_lossy().len() > 260);

        atomic_write_file(&target, "metadata").unwrap();

        assert_eq!(fs::read_to_string(&target).unwrap(), "metadata");

        let _ = fs::remove_dir_all(&root);
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
