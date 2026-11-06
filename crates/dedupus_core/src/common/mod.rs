use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use walkdir::{DirEntry, WalkDir};

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
        let mut callback = self.callback.lock().expect("progress callback panicked");
        callback(event);
    }

    pub fn into_inner(self) -> F {
        self.callback
            .into_inner()
            .expect("progress callback panicked")
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
    last_emit: Option<Instant>,
}

impl Default for DiscoveryThrottle {
    fn default() -> Self {
        Self::new()
    }
}

impl DiscoveryThrottle {
    pub fn new() -> Self {
        Self { last_emit: None }
    }

    pub fn should_emit(&mut self) -> bool {
        let now = Instant::now();
        match self.last_emit {
            Some(last) if now.duration_since(last) < Duration::from_millis(120) => false,
            _ => {
                self.last_emit = Some(now);
                true
            }
        }
    }
}

/// Walks every directory, skipping excluded subtrees and stopping when the
/// token is cancelled. `on_entry` is called for every entry; the caller decides
/// which entries to keep. Returns `false` when the walk was cancelled.
pub fn walk_files<F>(
    directories: &[PathBuf],
    excluded_directories: &[PathBuf],
    token: &CancellationToken,
    on_progress: &mut impl FnMut(ProgressEvent),
    mut on_entry: F,
) -> bool
where
    F: FnMut(&DirEntry),
{
    let mut total_scanned = 0usize;
    let mut discovery = DiscoveryThrottle::new();

    for dir in directories {
        if token.is_cancelled() {
            return false;
        }

        on_progress(ProgressEvent::ScanningDirectory { path: dir.clone() });

        for entry in WalkDir::new(dir).into_iter().filter_entry(|e| {
            !excluded_directories
                .iter()
                .any(|ex| e.path().starts_with(ex))
        }) {
            if token.is_cancelled() {
                return false;
            }

            let Ok(entry) = entry else { continue };

            if entry.file_type().is_file() {
                total_scanned += 1;
                if discovery.should_emit() {
                    on_progress(ProgressEvent::FilesDiscovered {
                        count: total_scanned,
                    });
                }
            }

            on_entry(&entry);
        }
    }

    on_progress(ProgressEvent::FilesDiscovered {
        count: total_scanned,
    });
    true
}

/// Disjoint-set forest used to cluster near-matching items.
pub struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    pub fn new(len: usize) -> Self {
        Self {
            parent: (0..len).collect(),
        }
    }

    pub fn find(&mut self, mut index: usize) -> usize {
        while self.parent[index] != index {
            self.parent[index] = self.parent[self.parent[index]];
            index = self.parent[index];
        }
        index
    }

    pub fn union(&mut self, a: usize, b: usize) {
        let root_a = self.find(a);
        let root_b = self.find(b);
        if root_a != root_b {
            self.parent[root_a] = root_b;
        }
    }
}
