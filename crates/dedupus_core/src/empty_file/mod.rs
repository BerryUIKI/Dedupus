use crate::common::{CancellationToken, ScannedFile};
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

    pub fn find_empty_files(&self) -> Result<Vec<ScannedFile>> {
        let mut empty_files = Vec::new();

        for dir in &self.config.included_directories {
            if self.cancellation_token.is_cancelled() {
                break;
            }

            for entry in WalkDir::new(dir).into_iter().filter_entry(|e| {
                let path = e.path();
                !self
                    .config
                    .excluded_directories
                    .iter()
                    .any(|ex| path.starts_with(ex))
            }) {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                if !entry.file_type().is_file() {
                    continue;
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

        Ok(empty_files)
    }
}
