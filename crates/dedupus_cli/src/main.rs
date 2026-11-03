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

            println!("\n🎉 Found {} groups of duplicate files:\n", duplicates.len());
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
                    println!("⚡ Computing perceptual hashes for {} images...", total_files);
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
            let empty_files = finder.find_empty_files()?;

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
            let empty_folders = finder.find_empty_folders()?;

            println!("\nFound {} empty folders:", empty_folders.len());
            for folder in empty_folders {
                println!("  - {}", folder.display());
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
                println!("  - Cached duplicate file hashes: {}", dup_cache.file_hashes.len());
                println!("  - Cached perceptual image hashes: {}", img_cache.image_hashes.len());
                println!("  - Cache directory: {}", ScanCache::default_cache_path("").parent().unwrap().display());
            }
            CacheAction::Prune => {
                let mut dup_cache = ScanCache::load_or_create("dup_cache");
                let mut img_cache = ScanCache::load_or_create("img_cache");
                let pruned_dup = dup_cache.prune_missing();
                let pruned_img = img_cache.prune_missing();
                let _ = dup_cache.save();
                let _ = img_cache.save();
                println!("🧹 Pruned {} missing file hashes and {} missing image hashes.", pruned_dup, pruned_img);
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
