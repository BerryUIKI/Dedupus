use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CleanReport {
    pub successful: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
    pub bytes_freed: u64,
}

/// Safely moves the given files or directories to the system Recycle Bin / Trash.
pub fn move_to_trash(paths: &[PathBuf]) -> CleanReport {
    let mut report = CleanReport::default();

    for path in paths {
        let size = path.metadata().map(|m| m.len()).unwrap_or(0);
        match trash::delete(path) {
            Ok(_) => {
                report.successful.push(path.clone());
                report.bytes_freed += size;
            }
            Err(e) => {
                report.failed.push((path.clone(), e.to_string()));
            }
        }
    }

    report
}

/// Permanently deletes the given files or directories from disk.
pub fn delete_permanently(paths: &[PathBuf]) -> CleanReport {
    let mut report = CleanReport::default();

    for path in paths {
        let metadata = match path.metadata() {
            Ok(m) => m,
            Err(e) => {
                report.failed.push((path.clone(), e.to_string()));
                continue;
            }
        };

        let size = metadata.len();
        let res = if metadata.is_dir() {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        };

        match res {
            Ok(_) => {
                report.successful.push(path.clone());
                report.bytes_freed += size;
            }
            Err(e) => {
                report.failed.push((path.clone(), e.to_string()));
            }
        }
    }

    report
}

/// Replaces duplicate files with hard links pointing to the target file.
/// This preserves the directory hierarchy and access paths while freeing 100% of duplicate disk space.
pub fn replace_with_hardlinks(target: &Path, duplicates: &[PathBuf]) -> CleanReport {
    let mut report = CleanReport::default();

    if !target.exists() || !target.is_file() {
        report.failed.push((
            target.to_path_buf(),
            "Target file for hard linking does not exist or is not a file".to_string(),
        ));
        return report;
    }

    for dup in duplicates {
        if dup == target {
            continue;
        }

        let size = dup.metadata().map(|m| m.len()).unwrap_or(0);

        // Delete duplicate file first, then create hard link
        match fs::remove_file(dup) {
            Ok(_) => match fs::hard_link(target, dup) {
                Ok(_) => {
                    report.successful.push(dup.clone());
                    report.bytes_freed += size;
                }
                Err(e) => {
                    report
                        .failed
                        .push((dup.clone(), format!("Failed to create hard link: {}", e)));
                }
            },
            Err(e) => {
                report.failed.push((
                    dup.clone(),
                    format!("Failed to remove original file before hardlinking: {}", e),
                ));
            }
        }
    }

    report
}
