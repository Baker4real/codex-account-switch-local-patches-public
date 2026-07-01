use std::fs;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

pub fn file_modified_ms(path: &Path) -> Option<u64> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|value| u64::try_from(value.as_millis()).ok())
}

pub fn collect_jsonl_files(root: &Path, files: &mut Vec<PathBuf>) {
    let mut visited = std::collections::HashSet::new();
    collect_jsonl_files_inner(root, files, &mut visited);
}

fn is_reparse_or_symlink(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }

    #[cfg(windows)]
    {
        return metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }

    #[cfg(not(windows))]
    {
        false
    }
}

fn collect_jsonl_files_inner(
    root: &Path,
    files: &mut Vec<PathBuf>,
    visited: &mut std::collections::HashSet<PathBuf>,
) {
    let Ok(metadata) = fs::symlink_metadata(root) else {
        return;
    };
    if is_reparse_or_symlink(&metadata) || !metadata.is_dir() {
        return;
    }
    let canonical = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    if !visited.insert(canonical) {
        return;
    }

    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if is_reparse_or_symlink(&metadata) {
            continue;
        }

        if metadata.is_dir() {
            collect_jsonl_files_inner(&path, files, visited);
            continue;
        }

        if metadata.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"))
        {
            files.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("codex-switch-session-files-{name}-{unique}"))
    }

    #[cfg(windows)]
    fn create_reparse_dir(link: &Path, target: &Path) {
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(
            status.success(),
            "failed to create junction at {}",
            link.display()
        );
    }

    #[cfg(unix)]
    fn create_reparse_dir(link: &Path, target: &Path) {
        std::os::unix::fs::symlink(target, link).unwrap();
    }

    #[test]
    fn collect_jsonl_files_skips_reparse_or_symlink_directories() {
        let workspace = temp_dir("skip-reparse");
        let sessions = workspace.join("sessions");
        let real_dir = sessions.join("real");
        let outside_dir = workspace.join("outside");
        let link_dir = sessions.join("linked-outside");
        fs::create_dir_all(&real_dir).unwrap();
        fs::create_dir_all(&outside_dir).unwrap();
        let real_file = real_dir.join("real.jsonl");
        let outside_file = outside_dir.join("leaked.jsonl");
        fs::write(&real_file, "{}\n").unwrap();
        fs::write(&outside_file, "{}\n").unwrap();
        create_reparse_dir(&link_dir, &outside_dir);

        let mut files = Vec::new();
        collect_jsonl_files(&sessions, &mut files);

        assert!(files.iter().any(|path| path == &real_file));
        assert!(!files.iter().any(|path| path == &outside_file));
        #[cfg(windows)]
        let _ = fs::remove_dir(&link_dir);
        #[cfg(unix)]
        let _ = fs::remove_file(&link_dir);
        let _ = fs::remove_dir_all(&workspace);
    }
}
