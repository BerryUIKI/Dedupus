use crate::common::{CancellationToken, DiscoveryThrottle, ProgressEvent};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmptyFolderFinderConfig {
    pub included_directories: Vec<PathBuf>,
    pub excluded_directories: Vec<PathBuf>,
}

pub struct EmptyFolderFinder {
    config: EmptyFolderFinderConfig,
    cancellation_token: CancellationToken,
}

impl EmptyFolderFinder {
    pub fn new(config: EmptyFolderFinderConfig, cancellation_token: CancellationToken) -> Self {
        Self {
            config,
            cancellation_token,
        }
    }

    /// Finds empty directories (bottom-up scan).
    pub fn find_empty_folders<F>(&self, mut on_progress: F) -> Result<Vec<PathBuf>>
    where
        F: FnMut(ProgressEvent),
    {
        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Discovering empty folders".to_string(),
        });

        let mut empty_folders = Vec::new();
        let mut total_scanned = 0usize;
        let mut discovery = DiscoveryThrottle::new();

        for dir in &self.config.included_directories {
            if self.cancellation_token.is_cancelled() {
                break;
            }

            on_progress(ProgressEvent::ScanningDirectory { path: dir.clone() });

            // contents_first(true) yields children before parent (bottom-up)
            for entry in WalkDir::new(dir)
                .contents_first(true)
                .into_iter()
                .filter_entry(|e| {
                    let path = e.path();
                    !self
                        .config
                        .excluded_directories
                        .iter()
                        .any(|ex| path.starts_with(ex))
                })
            {
                if self.cancellation_token.is_cancelled() {
                    break;
                }

                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                if !entry.file_type().is_dir() {
                    continue;
                }

                total_scanned += 1;
                if discovery.should_emit() {
                    on_progress(ProgressEvent::FilesDiscovered {
                        count: total_scanned,
                    });
                }

                let path = entry.path();
                // Avoid marking root scan directory itself if user just selected an empty folder
                if path == dir {
                    continue;
                }

                if is_folder_empty(path) {
                    empty_folders.push(path.to_path_buf());
                }
            }
        }

        on_progress(ProgressEvent::FilesDiscovered {
            count: total_scanned,
        });
        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Empty folder search finished".to_string(),
        });

        Ok(empty_folders)
    }
}

fn is_folder_empty(path: &std::path::Path) -> bool {
    if let Ok(mut read_dir) = fs::read_dir(path) {
        read_dir.next().is_none()
    } else {
        false
    }
}
