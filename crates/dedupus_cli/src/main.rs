use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use dedupus_core::{
    CancellationToken, DuplicateFinder, DuplicateFinderConfig, EmptyFileFinder,
    EmptyFileFinderConfig, EmptyFolderFinder, EmptyFolderFinderConfig, HashAlgorithm,
    ImageHashAlgorithm, ProgressEvent, ScanCache, SimilarImageConfig, SimilarImageFinder,
};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "dedupus")]
#[command(author = "BerryUIKI")]
#[command(version = "0.1.0")]
#[command(about = "A blazing fast, modern duplicate finder and disk cleaner", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum CliHashAlgo {
    Blake3,
    Xxh3_64,
    Xxh3_128,
}

impl From<CliHashAlgo> for HashAlgorithm {
    fn from(algo: CliHashAlgo) -> Self {
        match algo {
            CliHashAlgo::Blake3 => HashAlgorithm::Blake3,
            CliHashAlgo::Xxh3_64 => HashAlgorithm::Xxh3_64,
            CliHashAlgo::Xxh3_128 => HashAlgorithm::Xxh3_128,
        }
    }
}

#[derive(Subcommand, Debug)]
enum CacheAction {
    /// Show cached hash statistics
    Stats,
    /// Prune cache entries for files that no longer exist
    Prune,
    /// Clear all cached hashes
    Clear,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Search for duplicate files
    Dup {
        /// Directories to scan
        #[arg(required = true)]
        directories: Vec<PathBuf>,

        /// Excluded directories
        #[arg(short, long)]
        exclude: Vec<PathBuf>,

        /// Minimum file size in bytes (default: 1)
        #[arg(short, long, default_value_t = 1)]
        min_size: u64,

        /// Maximum file size in bytes
        #[arg(long)]
        max_size: Option<u64>,

        /// Hashing algorithm
        #[arg(short, long, value_enum, default_value_t = CliHashAlgo::Blake3)]
        algo: CliHashAlgo,

        /// Disable hash caching
        #[arg(long)]
        no_cache: bool,
    },

    /// Search for similar images using perceptual hashing
    Image {
        /// Directories to scan
        #[arg(required = true)]
        directories: Vec<PathBuf>,

        /// Excluded directories
        #[arg(short, long)]
        exclude: Vec<PathBuf>,

        /// Maximum Hamming distance (0 = identical, 1-3 = very high, 4-8 = high, default: 4)
        #[arg(short, long, default_value_t = 4)]
        distance: u8,

        /// Perceptual algorithm: gradient (dHash) or average (aHash)
        #[arg(short, long, default_value = "gradient")]
        algo: String,

        /// Disable hash caching
        #[arg(long)]
        no_cache: bool,
    },

    /// Search for similar audio tracks using acoustic envelope fingerprinting
    Audio {
        /// Directories to scan
        #[arg(required = true)]
        directories: Vec<PathBuf>,

        /// Excluded directories
        #[arg(short, long)]
        exclude: Vec<PathBuf>,

        /// Maximum duration difference in seconds (default: 2.0)
        #[arg(short, long, default_value_t = 2.0)]
        duration_diff: f64,
    },

    /// Search for similar videos using keyframe perceptual hashing
    Video {
        /// Directories to scan
        #[arg(required = true)]
        directories: Vec<PathBuf>,

        /// Excluded directories
        #[arg(short, long)]
        exclude: Vec<PathBuf>,

        /// Maximum duration difference in seconds (default: 2.0)
        #[arg(short, long, default_value_t = 2.0)]
        duration_diff: f64,
    },

    /// Search for empty files
    EmptyFiles {
        /// Directories to scan
        #[arg(required = true)]
        directories: Vec<PathBuf>,

        /// Excluded directories
        #[arg(short, long)]
        exclude: Vec<PathBuf>,
    },

    /// Search for empty folders
    EmptyFolders {
        /// Directories to scan
        #[arg(required = true)]
        directories: Vec<PathBuf>,

        /// Excluded directories
        #[arg(short, long)]
        exclude: Vec<PathBuf>,
    },

    /// List the largest files in the given directories
    BigFiles {
        /// Directories to scan
        #[arg(required = true)]
        directories: Vec<PathBuf>,

        /// Excluded directories
        #[arg(short, long)]
        exclude: Vec<PathBuf>,

        /// Minimum file size in bytes (default: 100 MiB)
        #[arg(short, long, default_value_t = 100 * 1024 * 1024)]
        min_size: u64,

        /// Maximum number of results
        #[arg(short = 'l', long, default_value_t = 100)]
        limit: usize,
    },

    /// Find temporary, backup and junk files
    TempFiles {
        /// Directories to scan
        #[arg(required = true)]
        directories: Vec<PathBuf>,

        /// Excluded directories
        #[arg(short, long)]
        exclude: Vec<PathBuf>,

        /// Minimum file size in bytes
        #[arg(short, long, default_value_t = 0)]
        min_size: u64,

        /// Additional extensions to treat as temporary (e.g. .myext)
        #[arg(short = 'x', long)]
        extra_extension: Vec<String>,
    },

    /// Find broken or corrupted files (images, archives, PDFs, audio, symlinks)
    BrokenFiles {
        /// Directories to scan
        #[arg(required = true)]
        directories: Vec<PathBuf>,

        /// Excluded directories
        #[arg(short, long)]
        exclude: Vec<PathBuf>,

        /// Skip image validation
        #[arg(long)]
        no_images: bool,

        /// Skip archive and PDF validation
        #[arg(long)]
        no_archives: bool,

        /// Skip audio validation
        #[arg(long)]
        no_audio: bool,
    },

    /// Move specified files to the system Recycle Bin / Trash
    Trash {
        /// Files to move to Trash
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },

    /// Manage persistent scan cache
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let token = CancellationToken::new();

    match cli.command {
        Commands::Dup {
            directories,
            exclude,
            min_size,
            max_size,
            algo,
            no_cache,
        } => {
            println!("🔍 Scanning for duplicate files in: {:?}", directories);

            let config = DuplicateFinderConfig {
                included_directories: directories,
                excluded_directories: exclude,
                min_file_size: min_size,
                max_file_size: max_size,
                hash_algorithm: algo.into(),
                use_cache: !no_cache,
            };

            let finder = DuplicateFinder::new(config, token);
            let duplicates = finder.find_duplicates(|event| match event {
                ProgressEvent::ScanningDirectory { path } => {
                    println!("📂 Scanning: {}", path.display());
                }
                ProgressEvent::FilesDiscovered { count } => {
                    println!("📄 Discovered {} files...", count);
                }
                ProgressEvent::HashingStarted { total_files } => {
                    println!("⚡ Computing hashes for {} candidate files...", total_files);
                }
                ProgressEvent::StageCompleted { stage_name } => {
                    println!("✅ Stage: {}", stage_name);
                }
                _ => {}
            })?;

            println!(
                "\n🎉 Found {} groups of duplicate files:\n",
                duplicates.len()
            );
            for (idx, group) in duplicates.iter().enumerate() {
                let wasted = group.file_size * (group.files.len() as u64 - 1);
                println!(
                    "Group #{} [Size: {} bytes each, Wasted: {} bytes, Hash: {}]:",
                    idx + 1,
                    group.file_size,
                    wasted,
                    group.hash
                );
                for file in &group.files {
                    println!("  - {}", file.path.display());
                }
                println!();
            }
        }

        Commands::Image {
            directories,
            exclude,
            distance,
            algo,
            no_cache,
        } => {
            println!("🖼️ Scanning for similar images in: {:?}", directories);

            let hash_algorithm = if algo.to_lowercase() == "average" {
                ImageHashAlgorithm::Average
            } else {
                ImageHashAlgorithm::Gradient
            };

            let config = SimilarImageConfig {
                included_directories: directories,
                excluded_directories: exclude,
                max_hamming_distance: distance,
                hash_algorithm,
                min_file_size: 1024,
                use_cache: !no_cache,
            };

            let finder = SimilarImageFinder::new(config, token);
            let groups = finder.find_similar_images(|event| match event {
                ProgressEvent::ScanningDirectory { path } => {
                    println!("📂 Scanning: {}", path.display());
                }
                ProgressEvent::FilesDiscovered { count } => {
                    println!("🖼️ Discovered {} image files...", count);
                }
                ProgressEvent::HashingStarted { total_files } => {
                    println!(
                        "⚡ Computing perceptual hashes for {} images...",
                        total_files
                    );
                }
                ProgressEvent::StageCompleted { stage_name } => {
                    println!("✅ Stage: {}", stage_name);
                }
                _ => {}
            })?;

            println!("\n🎉 Found {} groups of similar images:\n", groups.len());
            for (idx, group) in groups.iter().enumerate() {
                println!("Group #{} [{}]:", idx + 1, group.similarity_level);
                for img in &group.images {
                    println!(
                        "  - {} [{}x{}, {} bytes, Hash: {:016x}]",
                        img.path.display(),
                        img.width,
                        img.height,
                        img.size,
                        img.hash
                    );
                }
                println!();
            }
        }

        Commands::Audio {
            directories,
            exclude,
            duration_diff,
        } => {
            println!("🎵 Scanning for similar audio tracks in: {:?}", directories);
            let config = dedupus_core::SimilarAudioConfig {
                included_directories: directories,
                excluded_directories: exclude,
                max_duration_diff_secs: duration_diff,
                max_distance: 16,
                min_file_size: 10 * 1024,
            };

            let finder = dedupus_core::SimilarAudioFinder::new(config, token);
            let groups = finder.find_similar_audio(|event| match event {
                ProgressEvent::ScanningDirectory { path } => {
                    println!("📂 Scanning: {}", path.display());
                }
                ProgressEvent::FilesDiscovered { count } => {
                    println!("🎵 Discovered {} audio files...", count);
                }
                ProgressEvent::HashingStarted { total_files } => {
                    println!(
                        "⚡ Computing acoustic fingerprints for {} audio files...",
                        total_files
                    );
                }
                ProgressEvent::StageCompleted { stage_name } => {
                    println!("✅ Stage: {}", stage_name);
                }
                _ => {}
            })?;

            println!(
                "\n🎉 Found {} groups of similar audio tracks:\n",
                groups.len()
            );
            for (idx, group) in groups.iter().enumerate() {
                println!("Group #{} [{}]:", idx + 1, group.similarity_info);
                for track in &group.tracks {
                    println!(
                        "  - {} [{:.1}s, {}Hz, {}ch, {} bytes]",
                        track.path.display(),
                        track.duration_secs,
                        track.sample_rate,
                        track.channels,
                        track.size
                    );
                }
                println!();
            }
        }

        Commands::Video {
            directories,
            exclude,
            duration_diff,
        } => {
            println!("🎬 Scanning for similar videos in: {:?}", directories);
            let config = dedupus_core::SimilarVideoConfig {
                included_directories: directories,
                excluded_directories: exclude,
                max_duration_diff_secs: duration_diff,
                max_frame_distance: 10,
                min_file_size: 100 * 1024,
            };

            let finder = dedupus_core::SimilarVideoFinder::new(config, token);
            let groups = finder.find_similar_videos(|event| match event {
                ProgressEvent::ScanningDirectory { path } => {
                    println!("📂 Scanning: {}", path.display());
                }
                ProgressEvent::FilesDiscovered { count } => {
                    println!("🎬 Discovered {} video files...", count);
                }
                ProgressEvent::HashingStarted { total_files } => {
                    println!(
                        "⚡ Extracting keyframe hashes for {} videos...",
                        total_files
                    );
                }
                ProgressEvent::StageCompleted { stage_name } => {
                    println!("✅ Stage: {}", stage_name);
                }
                _ => {}
            })?;

            println!("\n🎉 Found {} groups of similar videos:\n", groups.len());
            for (idx, group) in groups.iter().enumerate() {
                println!("Group #{} [{}]:", idx + 1, group.similarity_info);
                for vid in &group.videos {
                    let res_str = match (vid.width, vid.height) {
                        (Some(w), Some(h)) => format!("{}x{}", w, h),
                        _ => "Unknown res".to_string(),
                    };
                    println!(
                        "  - {} [{}, {} bytes]",
                        vid.path.display(),
                        res_str,
                        vid.size
                    );
                }
                println!();
            }
        }

        Commands::EmptyFiles {
            directories,
            exclude,
        } => {
            println!("🔍 Scanning for empty files in: {:?}", directories);
            let config = EmptyFileFinderConfig {
                included_directories: directories,
                excluded_directories: exclude,
            };
            let finder = EmptyFileFinder::new(config, token);
            let empty_files = finder.find_empty_files(|_| {})?;

            println!("\nFound {} empty files:", empty_files.len());
            for f in empty_files {
                println!("  - {}", f.path.display());
            }
        }

        Commands::EmptyFolders {
            directories,
            exclude,
        } => {
            println!("🔍 Scanning for empty folders in: {:?}", directories);
            let config = EmptyFolderFinderConfig {
                included_directories: directories,
                excluded_directories: exclude,
            };
            let finder = EmptyFolderFinder::new(config, token);
            let empty_folders = finder.find_empty_folders(|_| {})?;

            println!("\nFound {} empty folders:", empty_folders.len());
            for folder in empty_folders {
                println!("  - {}", folder.display());
            }
        }

        Commands::BigFiles {
            directories,
            exclude,
            min_size,
            limit,
        } => {
            println!(
                "📦 Scanning for files >= {} bytes in: {:?}",
                min_size, directories
            );
            let config = dedupus_core::BigFileFinderConfig {
                included_directories: directories,
                excluded_directories: exclude,
                min_file_size: min_size,
                max_results: limit,
            };
            let finder = dedupus_core::BigFileFinder::new(config, token);
            let files = finder.find_big_files(|event| match event {
                ProgressEvent::ScanningDirectory { path } => {
                    println!("📂 Scanning: {}", path.display());
                }
                ProgressEvent::FilesDiscovered { count } => {
                    println!("📄 Scanned {} files...", count);
                }
                ProgressEvent::StageCompleted { stage_name } => {
                    println!("✅ Stage: {}", stage_name);
                }
                _ => {}
            })?;

            let total: u64 = files.iter().map(|f| f.size).sum();
            println!(
                "\nFound {} large files (Total: {:.2} MiB):",
                files.len(),
                total as f64 / (1024.0 * 1024.0)
            );
            for (idx, f) in files.iter().enumerate() {
                println!(
                    "  {:>3}. {:>10.2} MiB  {}",
                    idx + 1,
                    f.size as f64 / (1024.0 * 1024.0),
                    f.path.display()
                );
            }
        }

        Commands::TempFiles {
            directories,
            exclude,
            min_size,
            extra_extension,
        } => {
            println!("🧹 Scanning for temporary files in: {:?}", directories);
            let config = dedupus_core::TempFileFinderConfig {
                included_directories: directories,
                excluded_directories: exclude,
                min_file_size: min_size,
                extra_extensions: extra_extension
                    .into_iter()
                    .map(|e| e.trim_start_matches('.').to_ascii_lowercase())
                    .collect(),
            };
            let finder = dedupus_core::TempFileFinder::new(config, token);
            let found = finder.find_temp_files(|event| match event {
                ProgressEvent::ScanningDirectory { path } => {
                    println!("📂 Scanning: {}", path.display());
                }
                ProgressEvent::FilesDiscovered { count } => {
                    println!("📄 Scanned {} files...", count);
                }
                ProgressEvent::StageCompleted { stage_name } => {
                    println!("✅ Stage: {}", stage_name);
                }
                _ => {}
            })?;

            let total: u64 = found.iter().map(|f| f.size).sum();
            println!(
                "\nFound {} temporary files (Total: {:.2} MiB):",
                found.len(),
                total as f64 / (1024.0 * 1024.0)
            );
            for f in &found {
                println!(
                    "  {:>10.2} KiB  [{}]  {}",
                    f.size as f64 / 1024.0,
                    f.reason,
                    f.path.display()
                );
            }
        }

        Commands::BrokenFiles {
            directories,
            exclude,
            no_images,
            no_archives,
            no_audio,
        } => {
            println!("🩺 Scanning for broken files in: {:?}", directories);
            let config = dedupus_core::BrokenFileFinderConfig {
                included_directories: directories,
                excluded_directories: exclude,
                check_images: !no_images,
                check_archives: !no_archives,
                check_audio: !no_audio,
                check_symlinks: true,
            };
            let finder = dedupus_core::BrokenFileFinder::new(config, token);
            let broken = finder.find_broken_files(|event| match event {
                ProgressEvent::ScanningDirectory { path } => {
                    println!("📂 Scanning: {}", path.display());
                }
                ProgressEvent::FilesDiscovered { count } => {
                    println!("📄 Scanned {} files...", count);
                }
                ProgressEvent::StageCompleted { stage_name } => {
                    println!("✅ Stage: {}", stage_name);
                }
                _ => {}
            })?;

            println!("\nFound {} broken files:", broken.len());
            for f in &broken {
                println!(
                    "  {:>10.2} KiB  [{}]  {}",
                    f.size as f64 / 1024.0,
                    f.reason,
                    f.path.display()
                );
            }
        }

        Commands::Trash { files } => {
            println!("🗑️ Moving {} files to Recycle Bin...", files.len());
            let report = dedupus_core::move_to_trash(&files);
            println!(
                "✅ Successfully moved {} files (Freed: {} bytes)",
                report.successful.len(),
                report.bytes_freed
            );
            if !report.failed.is_empty() {
                println!("⚠️ Failed to move {} files:", report.failed.len());
                for (path, err) in report.failed {
                    println!("  - {}: {}", path.display(), err);
                }
            }
        }

        Commands::Cache { action } => match action {
            CacheAction::Stats => {
                let dup_cache = ScanCache::load_or_create("dup_cache");
                let img_cache = ScanCache::load_or_create("img_cache");
                println!("📊 Scan Cache Statistics:");
                println!(
                    "  - Cached duplicate file hashes: {}",
                    dup_cache.file_hashes.len()
                );
                println!(
                    "  - Cached perceptual image hashes: {}",
                    img_cache.image_hashes.len()
                );
                println!(
                    "  - Cache directory: {}",
                    ScanCache::default_cache_path("")
                        .parent()
                        .unwrap()
                        .display()
                );
            }
            CacheAction::Prune => {
                let mut dup_cache = ScanCache::load_or_create("dup_cache");
                let mut img_cache = ScanCache::load_or_create("img_cache");
                let pruned_dup = dup_cache.prune_missing();
                let pruned_img = img_cache.prune_missing();
                let _ = dup_cache.save();
                let _ = img_cache.save();
                println!(
                    "🧹 Pruned {} missing file hashes and {} missing image hashes.",
                    pruned_dup, pruned_img
                );
            }
            CacheAction::Clear => {
                let mut dup_cache = ScanCache::load_or_create("dup_cache");
                let mut img_cache = ScanCache::load_or_create("img_cache");
                dup_cache.clear()?;
                img_cache.clear()?;
                println!("✨ Cache cleared successfully!");
            }
        },
    }

    Ok(())
}
