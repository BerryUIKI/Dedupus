use crate::common::{walk_files, CancellationToken, ProgressEvent, ScannedFile};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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

        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Finding big files".to_string(),
        });

        let _ = walk_files(
            &self.config.included_directories,
            &self.config.excluded_directories,
            &self.cancellation_token,
            &mut on_progress,
            |entry| {
                if !entry.file_type().is_file() {
                    return;
                }

                let Ok(metadata) = entry.metadata() else {
                    return;
                };

                let size = metadata.len();
                if size < self.config.min_file_size {
                    return;
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
            },
        );

        found.sort_by_key(|f| std::cmp::Reverse(f.size));
        found.truncate(self.config.max_results);

        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Big file search finished".to_string(),
        });

        Ok(found)
    }
}
