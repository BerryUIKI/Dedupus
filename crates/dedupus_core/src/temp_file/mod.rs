use crate::common::{CancellationToken, ProgressEvent};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Extensions commonly used for temporary, backup and partial-download files.
pub const DEFAULT_TEMP_EXTENSIONS: &[&str] = &[
    "tmp", "temp", "bak", "old", "orig", "backup", "swp", "swo", "swn", "part", "partial",
    "download", "crdownload", "cpart", "cache", "rubbish", "trash",
];

/// Well-known OS junk file names (compared case-insensitively).
pub const JUNK_FILE_NAMES: &[&str] = &[".ds_store", "thumbs.db", "desktop.ini", "ehthumbs.db"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TempFile {
    pub path: PathBuf,
    pub size: u64,
    pub modified_secs: Option<u64>,
    /// Why the file was classified as temporary (extension, lock file, backup copy).
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TempFileFinderConfig {
    pub included_directories: Vec<PathBuf>,
    pub excluded_directories: Vec<PathBuf>,
    pub min_file_size: u64,
    /// Extra extensions to treat as temporary (lowercase, without the dot).
    pub extra_extensions: Vec<String>,
}

pub struct TempFileFinder {
    config: TempFileFinderConfig,
    cancellation_token: CancellationToken,
}

impl TempFileFinder {
    pub fn new(config: TempFileFinderConfig, cancellation_token: CancellationToken) -> Self {
        Self {
            config,
            cancellation_token,
        }
    }

    /// Finds temporary/backup files, sorted by size descending.
    pub fn find_temp_files<F>(&self, mut on_progress: F) -> Result<Vec<TempFile>>
    where
        F: FnMut(ProgressEvent),
    {
        let mut found: Vec<TempFile> = Vec::new();
        let mut total_scanned = 0usize;

        let mut extensions: Vec<String> = DEFAULT_TEMP_EXTENSIONS
            .iter()
            .map(|e| e.to_string())
            .collect();
        extensions.extend(self.config.extra_extensions.iter().cloned());

        for dir in &self.config.included_directories {
            if self.cancellation_token.is_cancelled() {
                break;
            }

            on_progress(ProgressEvent::ScanningDirectory { path: dir.clone() });

            for entry in WalkDir::new(dir).into_iter().filter_entry(|e| {
                let path = e.path();
                !self
                    .config
                    .excluded_directories
                    .iter()
                    .any(|ex| path.starts_with(ex))
            }) {
                if self.cancellation_token.is_cancelled() {
                    break;
                }

                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                if !entry.file_type().is_file() {
                    continue;
                }

                let path = entry.path();
                total_scanned += 1;
                if total_scanned.is_multiple_of(500) {
                    on_progress(ProgressEvent::FilesDiscovered {
                        count: total_scanned,
                    });
                }

                let reason = match classify_temp_file(path, &extensions) {
                    Some(r) => r,
                    None => continue,
                };

                let metadata = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                if metadata.len() < self.config.min_file_size {
                    continue;
                }

                let modified_secs = metadata
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs());

                found.push(TempFile {
                    path: path.to_path_buf(),
                    size: metadata.len(),
                    modified_secs,
                    reason,
                });
            }
        }

        on_progress(ProgressEvent::FilesDiscovered {
            count: total_scanned,
        });

        found.sort_by_key(|f| std::cmp::Reverse(f.size));

        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Temporary file search finished".to_string(),
        });

        Ok(found)
    }
}

/// Returns a human-readable reason when the file looks temporary.
fn classify_temp_file(path: &Path, extensions: &[String]) -> Option<String> {
    let file_name = path.file_name()?.to_string_lossy();
    let lower_name = file_name.to_ascii_lowercase();

    // Office/Word lock files: ~$document.docx
    if lower_name.starts_with("~$") {
        return Some("Office lock file".to_string());
    }

    // Editor/Unix backup copies: report.txt~
    if file_name.ends_with('~') {
        return Some("Backup copy".to_string());
    }

    // OS junk files: Thumbs.db, .DS_Store, desktop.ini
    if JUNK_FILE_NAMES.contains(&lower_name.as_str()) {
        return Some("OS junk file".to_string());
    }

    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let ext_lower = ext.to_ascii_lowercase();
        if extensions.iter().any(|e| e == &ext_lower) {
            return Some(format!("Extension .{}", ext_lower));
        }
    }

    None
}
