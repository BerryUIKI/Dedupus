use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use dedupus_core::{
    CancellationToken, DuplicateFinder, DuplicateFinderConfig, EmptyFileFinder,
    EmptyFileFinderConfig, EmptyFolderFinder, EmptyFolderFinderConfig, HashAlgorithm, ProgressEvent,
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
        } => {
            println!("🔍 Scanning for duplicate files in: {:?}", directories);

            let config = DuplicateFinderConfig {
                included_directories: directories,
                excluded_directories: exclude,
                min_file_size: min_size,
                max_file_size: max_size,
                hash_algorithm: algo.into(),
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
    }

    Ok(())
}
