use crate::common::{walk_files, CancellationToken, ProgressEvent, ScannedFile};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
                if metadata.len() != 0 {
                    return;
                }

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
            },
        );

        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Empty file search finished".to_string(),
        });

        Ok(empty_files)
    }
}
