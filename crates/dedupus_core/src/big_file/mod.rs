use crate::common::{CancellationToken, DiscoveryThrottle, ProgressEvent, ScannedFile};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BigFileFinderConfig {
    pub included_directories: Vec<PathBuf>,
    pub excluded_directories: Vec<PathBuf>,
    pub min_file_size: u64,
    pub max_results: usize,
}

impl Default for BigFileFinderConfig {
    fn default() -> Self {
        Self {
            included_directories: Vec::new(),
            excluded_directories: Vec::new(),
            min_file_size: 100 * 1024 * 1024, // 100 MB
            max_results: 100,
        }
    }
}

pub struct BigFileFinder {
    config: BigFileFinderConfig,
    cancellation_token: CancellationToken,
}

impl BigFileFinder {
    pub fn new(config: BigFileFinderConfig, cancellation_token: CancellationToken) -> Self {
        Self {
            config,
            cancellation_token,
        }
    }

    /// Collects files at or above `min_file_size`, sorted by size descending
    /// and truncated to `max_results`.
    pub fn find_big_files<F>(&self, mut on_progress: F) -> Result<Vec<ScannedFile>>
    where
        F: FnMut(ProgressEvent),
    {
        let mut found: Vec<ScannedFile> = Vec::new();
        let mut total_scanned = 0usize;
        let mut discovery = DiscoveryThrottle::new();

        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Finding big files".to_string(),
        });

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

                let metadata = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                let size = metadata.len();
                total_scanned += 1;

                if discovery.should_emit() {
                    on_progress(ProgressEvent::FilesDiscovered {
                        count: total_scanned,
                    });
                }

                if size < self.config.min_file_size {
                    continue;
                }

                let modified_secs = metadata
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs());

                found.push(ScannedFile {
                    path: entry.path().to_path_buf(),
                    size,
                    modified_secs,
                });
            }
        }

        on_progress(ProgressEvent::FilesDiscovered {
            count: total_scanned,
        });

        found.sort_by_key(|f| std::cmp::Reverse(f.size));

        if found.len() > self.config.max_results {
            found.truncate(self.config.max_results);
        }

        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Big file search finished".to_string(),
        });

        Ok(found)
    }
}
