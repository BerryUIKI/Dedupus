use crate::common::{
    CancellationToken, DiscoveryThrottle, ProgressCounter, ProgressEvent, ProgressReporter,
};
use anyhow::Result;
use image::imageops::FilterType;
use image::{GenericImageView, ImageReader};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const SUPPORTED_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp", "gif"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ImageHashAlgorithm {
    #[default]
    Gradient, // dHash (Difference/Gradient Hash)
    Average, // aHash (Average Hash)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarImageConfig {
    pub included_directories: Vec<PathBuf>,
    pub excluded_directories: Vec<PathBuf>,
    pub max_hamming_distance: u8, // 0 = identical perceptual, 1-3 = very high, 4-8 = high
    pub hash_algorithm: ImageHashAlgorithm,
    pub min_file_size: u64,
    pub use_cache: bool,
}

impl Default for SimilarImageConfig {
    fn default() -> Self {
        Self {
            included_directories: Vec::new(),
            excluded_directories: Vec::new(),
            max_hamming_distance: 4,
            hash_algorithm: ImageHashAlgorithm::Gradient,
            min_file_size: 1024, // 1KB minimum by default
            use_cache: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScannedImage {
    pub path: PathBuf,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub hash: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarImageGroup {
    pub similarity_level: String,
    pub images: Vec<ScannedImage>,
}

pub struct SimilarImageFinder {
    config: SimilarImageConfig,
    cancellation_token: CancellationToken,
}

impl SimilarImageFinder {
    pub fn new(config: SimilarImageConfig, cancellation_token: CancellationToken) -> Self {
        Self {
            config,
            cancellation_token,
        }
    }

    /// Discovers image files, computes perceptual hashes in parallel, and clusters similar images.
    pub fn find_similar_images<F>(&self, mut on_progress: F) -> Result<Vec<SimilarImageGroup>>
    where
        F: FnMut(ProgressEvent) + Send,
    {
        if self.cancellation_token.is_cancelled() {
            return Ok(Vec::new());
        }

        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Discovering image files".to_string(),
        });

        let mut candidate_paths = Vec::new();
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

                let path = entry.path();
                let is_image = path
                    .extension()
                    .and_then(OsStr::to_str)
                    .map(|ext| {
                        let lower = ext.to_ascii_lowercase();
                        SUPPORTED_EXTENSIONS.contains(&lower.as_str())
                    })
                    .unwrap_or(false);

                if !is_image {
                    continue;
                }

                if let Ok(metadata) = entry.metadata() {
                    if metadata.len() >= self.config.min_file_size {
                        let modified_secs = metadata
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs())
                            .unwrap_or(0);

                        candidate_paths.push((path.to_path_buf(), metadata.len(), modified_secs));
                        total_discovered += 1;
                        if discovery.should_emit() {
                            on_progress(ProgressEvent::FilesDiscovered {
                                count: total_discovered,
                            });
                        }
                    }
                }
            }
        }

        on_progress(ProgressEvent::FilesDiscovered {
            count: total_discovered,
        });

        if self.cancellation_token.is_cancelled() || candidate_paths.len() < 2 {
            return Ok(Vec::new());
        }

        // --- STAGE 2: Compute perceptual hashes in parallel ---
        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Hashing images".to_string(),
        });
        on_progress(ProgressEvent::HashingStarted {
            total_files: candidate_paths.len(),
        });

        let cache = if self.config.use_cache {
            Some(crate::cache::ScanCache::load_or_create("img_cache"))
        } else {
            None
        };

        let reporter = ProgressReporter::new(on_progress);
        let hash_counter = ProgressCounter::new(candidate_paths.len());

        let algo = self.config.hash_algorithm;
        let hashed_images: Vec<ScannedImage> = candidate_paths
            .into_par_iter()
            .filter_map(|(path, size, modified_secs)| {
                if self.cancellation_token.is_cancelled() {
                    return None;
                }
                if let Some((processed, total)) = hash_counter.tick() {
                    reporter.report(ProgressEvent::HashingProgress { processed, total });
                }

                if let Some(c) = &cache {
                    if let Some((width, height, hash)) =
                        c.get_image_hash(&path, size, modified_secs)
                    {
                        return Some(ScannedImage {
                            path,
                            size,
                            width,
                            height,
                            hash,
                        });
                    }
                }

                match compute_image_hash(&path, algo) {
                    Ok((width, height, hash)) => Some(ScannedImage {
                        path,
                        size,
                        width,
                        height,
                        hash,
                    }),
                    Err(_) => None,
                }
            })
            .collect();

        let mut on_progress = reporter.into_inner();

        // Update and save cache with newly computed image hashes
        if let Some(mut c) = cache {
            for img in &hashed_images {
                let modified_secs = img
                    .path
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);

                c.insert_image_hash(
                    img.path.clone(),
                    img.size,
                    modified_secs,
                    img.width,
                    img.height,
                    img.hash,
                );
            }
            let _ = c.save();
        }

        if self.cancellation_token.is_cancelled() || hashed_images.len() < 2 {
            return Ok(Vec::new());
        }

        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Clustering similar images".to_string(),
        });

        // --- STAGE 3: Clustering using Disjoint Set (Union-Find) ---
        let max_dist = self.config.max_hamming_distance;
        let n = hashed_images.len();
        let mut parent: Vec<usize> = (0..n).collect();

        fn find(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }

        fn union(parent: &mut [usize], i: usize, j: usize) {
            let root_i = find(parent, i);
            let root_j = find(parent, j);
            if root_i != root_j {
                parent[root_i] = root_j;
            }
        }

        // Parallel or sequential pairwise distance calculation
        // For N <= 10000, N^2 / 2 comparisons of u64 XOR is < 50M ops (fractions of a second)
        #[allow(clippy::needless_range_loop)]
        for i in 0..n {
            if self.cancellation_token.is_cancelled() {
                return Ok(Vec::new());
            }
            let h1 = hashed_images[i].hash;
            for j in (i + 1)..n {
                let h2 = hashed_images[j].hash;
                let distance = (h1 ^ h2).count_ones() as u8;
                if distance <= max_dist {
                    union(&mut parent, i, j);
                }
            }
        }

        // Group into sets
        let mut clusters: HashMap<usize, Vec<ScannedImage>> = HashMap::new();
        for (i, img) in hashed_images.into_iter().enumerate() {
            let root = find(&mut parent, i);
            clusters.entry(root).or_default().push(img);
        }

        // Keep clusters with >= 2 images
        let mut groups: Vec<SimilarImageGroup> = clusters
            .into_values()
            .filter(|list| list.len() >= 2)
            .map(|mut list| {
                // Sort by resolution descending, then file size descending
                list.sort_by(|a, b| {
                    let res_a = (a.width as u64) * (a.height as u64);
                    let res_b = (b.width as u64) * (b.height as u64);
                    res_b.cmp(&res_a).then_with(|| b.size.cmp(&a.size))
                });

                SimilarImageGroup {
                    similarity_level: format!("Max Distance: {}", max_dist),
                    images: list,
                }
            })
            .collect();

        // Sort groups by number of images descending
        groups.sort_by_key(|b| std::cmp::Reverse(b.images.len()));

        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Similar image search finished".to_string(),
        });

        Ok(groups)
    }
}

/// Loads the image, extracts dimensions, and computes perceptual hash.
fn compute_image_hash(path: &Path, algo: ImageHashAlgorithm) -> Result<(u32, u32, u64)> {
    let reader = ImageReader::open(path)?.with_guessed_format()?;
    let img = reader.decode()?;
    let (width, height) = img.dimensions();

    let hash = match algo {
        ImageHashAlgorithm::Gradient => compute_dhash(&img),
        ImageHashAlgorithm::Average => compute_ahash(&img),
    };

    Ok((width, height, hash))
}

/// Compute dHash (Gradient/Difference Hash)
/// Resizes to 9x8 grayscale, compares adjacent columns across 8 rows = 64 bits.
pub fn compute_dhash(img: &image::DynamicImage) -> u64 {
    let gray = img.resize_exact(9, 8, FilterType::Triangle).to_luma8();
    let mut hash = 0u64;
    let mut bit = 0;

    for y in 0..8 {
        for x in 0..8 {
            let p_left = gray.get_pixel(x, y)[0];
            let p_right = gray.get_pixel(x + 1, y)[0];
            if p_left > p_right {
                hash |= 1u64 << bit;
            }
            bit += 1;
        }
    }

    hash
}

/// Compute aHash (Average Hash)
/// Resizes to 8x8 grayscale, calculates average pixel value, 1 if >= avg, else 0 = 64 bits.
pub fn compute_ahash(img: &image::DynamicImage) -> u64 {
    let gray = img.resize_exact(8, 8, FilterType::Triangle).to_luma8();
    let pixels = gray.as_raw();
    let sum: u64 = pixels.iter().map(|&p| p as u64).sum();
    let avg = (sum / 64) as u8;

    let mut hash = 0u64;
    for (i, &p) in pixels.iter().enumerate() {
        if p >= avg {
            hash |= 1u64 << i;
        }
    }

    hash
}
