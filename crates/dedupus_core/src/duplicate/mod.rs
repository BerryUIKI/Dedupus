use crate::common::{
    CancellationToken, DiscoveryThrottle, HashAlgorithm, ProgressCounter, ProgressEvent,
    ProgressReporter, ScannedFile,
};
use anyhow::Result;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const PREFIX_HASH_SIZE: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateFinderConfig {
    pub included_directories: Vec<PathBuf>,
    pub excluded_directories: Vec<PathBuf>,
    pub min_file_size: u64,
    pub max_file_size: Option<u64>,
    pub hash_algorithm: HashAlgorithm,
    pub use_cache: bool,
}

impl Default for DuplicateFinderConfig {
    fn default() -> Self {
        Self {
            included_directories: Vec::new(),
            excluded_directories: Vec::new(),
            min_file_size: 1, // Skip empty files in duplicate scanner by default
            max_file_size: None,
            hash_algorithm: HashAlgorithm::Blake3,
            use_cache: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateGroup {
    pub file_size: u64,
    pub hash: String,
    pub files: Vec<ScannedFile>,
}

pub struct DuplicateFinder {
    config: DuplicateFinderConfig,
    cancellation_token: CancellationToken,
}

impl DuplicateFinder {
    pub fn new(config: DuplicateFinderConfig, cancellation_token: CancellationToken) -> Self {
        Self {
            config,
            cancellation_token,
        }
    }

    /// Execute the full multi-stage duplicate search pipeline.
    pub fn find_duplicates<F>(&self, mut on_progress: F) -> Result<Vec<DuplicateGroup>>
    where
        F: FnMut(ProgressEvent) + Send,
    {
        if self.cancellation_token.is_cancelled() {
            return Ok(Vec::new());
        }

        // --- STAGE 1: Directory traversal & size collection ---
        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Starting directory scan".to_string(),
        });

        let mut files_by_size: HashMap<u64, Vec<ScannedFile>> = HashMap::new();
        let mut total_discovered = 0;
        let mut discovery = DiscoveryThrottle::new();

        for dir in &self.config.included_directories {
            if self.cancellation_token.is_cancelled() {
                return Ok(Vec::new());
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
                    return Ok(Vec::new());
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
                if size < self.config.min_file_size {
                    continue;
                }
                if let Some(max) = self.config.max_file_size {
                    if size > max {
                        continue;
                    }
                }

                let modified_secs = metadata
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs());

                files_by_size.entry(size).or_default().push(ScannedFile {
                    path: entry.path().to_path_buf(),
                    size,
                    modified_secs,
                });

                total_discovered += 1;
                if discovery.should_emit() {
                    on_progress(ProgressEvent::FilesDiscovered {
                        count: total_discovered,
                    });
                }
            }
        }

        on_progress(ProgressEvent::FilesDiscovered {
            count: total_discovered,
        });

        // Retain only groups with 2 or more files with the exact same size
        files_by_size.retain(|_, files| files.len() >= 2);

        if self.cancellation_token.is_cancelled() || files_by_size.is_empty() {
            return Ok(Vec::new());
        }

        // --- STAGE 2: Prefix hash filtering (4KB) ---
        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Computing prefix hashes".to_string(),
        });

        let candidate_files: Vec<ScannedFile> = files_by_size.into_values().flatten().collect();

        on_progress(ProgressEvent::HashingStarted {
            total_files: candidate_files.len(),
        });

        let reporter = ProgressReporter::new(on_progress);
        let prefix_counter = ProgressCounter::new(candidate_files.len());

        // Compute prefix hash in parallel
        let prefix_hashed: Vec<(u64, [u8; 16], ScannedFile)> = candidate_files
            .into_par_iter()
            .filter_map(|file| {
                if self.cancellation_token.is_cancelled() {
                    return None;
                }
                if let Some((processed, total)) = prefix_counter.tick() {
                    reporter.report(ProgressEvent::HashingProgress { processed, total });
                }
                let prefix = compute_prefix_hash(&file.path).ok()?;
                Some((file.size, prefix, file))
            })
            .collect();

        let mut on_progress = reporter.into_inner();

        // Group by (size, prefix_hash)
        let mut by_prefix: HashMap<(u64, [u8; 16]), Vec<ScannedFile>> = HashMap::new();
        for (size, prefix, file) in prefix_hashed {
            by_prefix.entry((size, prefix)).or_default().push(file);
        }
        by_prefix.retain(|_, files| files.len() >= 2);

        if self.cancellation_token.is_cancelled() || by_prefix.is_empty() {
            return Ok(Vec::new());
        }

        // --- STAGE 3: Full hashing ---
        let full_hash_candidates: Vec<ScannedFile> = by_prefix.into_values().flatten().collect();
        let total_to_hash = full_hash_candidates.len();

        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Hashing matching files".to_string(),
        });
        on_progress(ProgressEvent::HashingStarted {
            total_files: total_to_hash,
        });

        let reporter = ProgressReporter::new(on_progress);
        let hash_counter = ProgressCounter::new(total_to_hash);

        let cache = if self.config.use_cache {
            Some(crate::cache::ScanCache::load_or_create("dup_cache"))
        } else {
            None
        };

        let algo = self.config.hash_algorithm;
        let full_hashes: Vec<(u64, String, ScannedFile)> = full_hash_candidates
            .into_par_iter()
            .filter_map(|file| {
                if self.cancellation_token.is_cancelled() {
                    return None;
                }
                if let Some((processed, total)) = hash_counter.tick() {
                    reporter.report(ProgressEvent::HashingProgress { processed, total });
                }

                // Check cache if enabled
                if let Some(c) = &cache {
                    if let Some(h) =
                        c.get_file_hash(&file.path, file.size, file.modified_secs.unwrap_or(0))
                    {
                        return Some((file.size, h.to_string(), file));
                    }
                }

                let hash = compute_full_hash(&file.path, algo).ok()?;
                Some((file.size, hash, file))
            })
            .collect();

        let mut on_progress = reporter.into_inner();

        // Update and save cache with newly computed hashes
        if let Some(mut c) = cache {
            for (size, hash, file) in &full_hashes {
                c.insert_file_hash(
                    file.path.clone(),
                    *size,
                    file.modified_secs.unwrap_or(0),
                    hash.clone(),
                );
            }
            let _ = c.save();
        }

        // Group by (size, full_hash)
        let mut duplicates_map: HashMap<(u64, String), Vec<ScannedFile>> = HashMap::new();
        for (size, hash, file) in full_hashes {
            duplicates_map.entry((size, hash)).or_default().push(file);
        }
        duplicates_map.retain(|_, files| files.len() >= 2);

        let mut duplicates: Vec<DuplicateGroup> = duplicates_map
            .into_iter()
            .map(|((file_size, hash), files)| DuplicateGroup {
                file_size,
                hash,
                files,
            })
            .collect();

        // Sort descending by wasted space (file_size * (files.len() - 1))
        duplicates.sort_by(|a, b| {
            let wasted_a = a.file_size * (a.files.len() as u64 - 1);
            let wasted_b = b.file_size * (b.files.len() as u64 - 1);
            wasted_b.cmp(&wasted_a)
        });

        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Duplicate search finished".to_string(),
        });

        Ok(duplicates)
    }
}

fn compute_prefix_hash(path: &Path) -> Result<[u8; 16]> {
    let mut file = File::open(path)?;
    let mut buffer = [0u8; PREFIX_HASH_SIZE];
    let bytes_read = file.read(&mut buffer)?;
    let hash = xxhash_rust::xxh3::xxh3_128(&buffer[..bytes_read]);
    Ok(hash.to_le_bytes())
}

fn compute_full_hash(path: &Path, algorithm: HashAlgorithm) -> Result<String> {
    let file = File::open(path)?;
    let len = file.metadata()?.len();

    // Use memory-mapped I/O for files >= 16KB up to 1GB for optimal speed
    if (16 * 1024..=1024 * 1024 * 1024).contains(&len) {
        if let Ok(mmap) = unsafe { memmap2::Mmap::map(&file) } {
            return Ok(match algorithm {
                HashAlgorithm::Blake3 => blake3::hash(&mmap).to_hex().to_string(),
                HashAlgorithm::Xxh3_64 => format!("{:016x}", xxhash_rust::xxh3::xxh3_64(&mmap)),
                HashAlgorithm::Xxh3_128 => format!("{:032x}", xxhash_rust::xxh3::xxh3_128(&mmap)),
            });
        }
    }

    // Streaming fallback for very large files or when mmap fails
    let mut reader = std::io::BufReader::with_capacity(64 * 1024, file);
    match algorithm {
        HashAlgorithm::Blake3 => {
            let mut hasher = blake3::Hasher::new();
            let mut buf = [0u8; 64 * 1024];
            loop {
                let n = reader.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            Ok(hasher.finalize().to_hex().to_string())
        }
        HashAlgorithm::Xxh3_64 | HashAlgorithm::Xxh3_128 => {
            let mut data = Vec::new();
            reader.read_to_end(&mut data)?;
            if algorithm == HashAlgorithm::Xxh3_64 {
                Ok(format!("{:016x}", xxhash_rust::xxh3::xxh3_64(&data)))
            } else {
                Ok(format!("{:032x}", xxhash_rust::xxh3::xxh3_128(&data)))
            }
        }
    }
}
