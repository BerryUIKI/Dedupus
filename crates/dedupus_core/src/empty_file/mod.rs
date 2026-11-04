use crate::common::{CancellationToken, DiscoveryThrottle, ProgressEvent, ScannedFile};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmptyFileFinderConfig {
    pub included_directories: Vec<PathBuf>,
    pub excluded_directories: Vec<PathBuf>,
}

pub struct EmptyFileFinder {
    config: EmptyFileFinderConfig,
    cancellation_token: CancellationToken,
}

impl EmptyFileFinder {
    pub fn new(config: EmptyFileFinderConfig, cancellation_token: CancellationToken) -> Self {
        Self {
            config,
            cancellation_token,
        }
    }

    pub fn find_empty_files<F>(&self, mut on_progress: F) -> Result<Vec<ScannedFile>>
    where
        F: FnMut(ProgressEvent),
    {
        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Discovering empty files".to_string(),
        });

        let mut empty_files = Vec::new();
        let mut total_scanned = 0usize;
        let mut discovery = DiscoveryThrottle::new();

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

                total_scanned += 1;
                if discovery.should_emit() {
                    on_progress(ProgressEvent::FilesDiscovered {
                        count: total_scanned,
                    });
                }

                if let Ok(metadata) = entry.metadata() {
                    if metadata.len() == 0 {
                        let modified_secs = metadata
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs());

                        empty_files.push(ScannedFile {
                            path: entry.path().to_path_buf(),
                            size: 0,
                            modified_secs,
                        });
                    }
                }
            }
        }

        on_progress(ProgressEvent::FilesDiscovered {
            count: total_scanned,
        });
        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Empty file search finished".to_string(),
        });

        Ok(empty_files)
    }
}
