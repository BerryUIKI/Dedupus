use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Supported hashing algorithms for duplicate detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum HashAlgorithm {
    #[default]
    Blake3,
    Xxh3_64,
    Xxh3_128,
}

/// Token used to cooperatively cancel an in-flight scanning operation.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

/// Scanning progress event sent to progress callbacks or channels.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProgressEvent {
    ScanningDirectory { path: PathBuf },
    FilesDiscovered { count: usize },
    HashingStarted { total_files: usize },
    HashingProgress { processed: usize, total: usize },
    StageCompleted { stage_name: String },
}

/// A scanned file entry with core metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScannedFile {
    pub path: PathBuf,
    pub size: u64,
    pub modified_secs: Option<u64>,
}
