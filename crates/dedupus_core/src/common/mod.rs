use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

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

/// Serializes progress callbacks that are invoked from parallel workers.
///
/// A single `ProgressReporter` is shared (by reference) across rayon closures;
/// the mutex keeps the `FnMut` callback usable while workers report concurrently.
pub struct ProgressReporter<F: FnMut(ProgressEvent)> {
    callback: Mutex<F>,
}

impl<F: FnMut(ProgressEvent)> ProgressReporter<F> {
    pub fn new(callback: F) -> Self {
        Self {
            callback: Mutex::new(callback),
        }
    }

    pub fn report(&self, event: ProgressEvent) {
        let mut callback = self.callback.lock().unwrap_or_else(|e| e.into_inner());
        callback(event);
    }

    pub fn into_inner(self) -> F {
        self.callback
            .into_inner()
            .unwrap_or_else(|e| e.into_inner())
    }
}

/// Counts processed items and throttles progress emission to roughly
/// 100 updates per stage, so parallel hashing does not flood the UI.
pub struct ProgressCounter {
    total: usize,
    step: usize,
    processed: AtomicUsize,
}

impl ProgressCounter {
    pub fn new(total: usize) -> Self {
        Self {
            total,
            step: (total / 100).max(1),
            processed: AtomicUsize::new(0),
        }
    }

    /// Records one processed item and returns `(processed, total)` when an
    /// update should be emitted.
    pub fn tick(&self) -> Option<(usize, usize)> {
        let processed = self.processed.fetch_add(1, Ordering::Relaxed) + 1;
        if processed == self.total || processed.is_multiple_of(self.step) {
            Some((processed, self.total))
        } else {
            None
        }
    }
}

/// Time-based throttle for discovery progress: keeps the file counter live
/// without emitting an IPC/CLI update for every single walked entry.
pub struct DiscoveryThrottle {
    last_emit: std::time::Instant,
}

impl Default for DiscoveryThrottle {
    fn default() -> Self {
        Self::new()
    }
}

impl DiscoveryThrottle {
    pub fn new() -> Self {
        Self {
            // Emit the first update immediately.
            last_emit: std::time::Instant::now() - std::time::Duration::from_millis(250),
        }
    }

    pub fn should_emit(&mut self) -> bool {
        if self.last_emit.elapsed() >= std::time::Duration::from_millis(120) {
            self.last_emit = std::time::Instant::now();
            true
        } else {
            false
        }
    }
}
