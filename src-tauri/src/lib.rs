use dedupus_core::{
    CancellationToken, DuplicateFinder, DuplicateFinderConfig, DuplicateGroup, EmptyFileFinder,
    EmptyFileFinderConfig, EmptyFolderFinder, EmptyFolderFinderConfig, HashAlgorithm,
    ImageHashAlgorithm, ScannedFile, SimilarImageConfig, SimilarImageFinder, SimilarImageGroup,
};
use std::path::PathBuf;

#[tauri::command]
fn scan_duplicates(
    directories: Vec<String>,
    exclude: Vec<String>,
    min_size: Option<u64>,
    max_size: Option<u64>,
    algo: Option<String>,
) -> Result<Vec<DuplicateGroup>, String> {
    let hash_algorithm = match algo.as_deref() {
        Some("xxh3_64") => HashAlgorithm::Xxh3_64,
        Some("xxh3_128") => HashAlgorithm::Xxh3_128,
        _ => HashAlgorithm::Blake3,
    };

    let config = DuplicateFinderConfig {
        included_directories: directories.into_iter().map(PathBuf::from).collect(),
        excluded_directories: exclude.into_iter().map(PathBuf::from).collect(),
        min_file_size: min_size.unwrap_or(1),
        max_file_size: max_size,
        hash_algorithm,
    };

    let token = CancellationToken::new();
    let finder = DuplicateFinder::new(config, token);

    finder.find_duplicates(|_| {}).map_err(|e| e.to_string())
}

#[tauri::command]
fn scan_similar_images(
    directories: Vec<String>,
    exclude: Vec<String>,
    distance: Option<u8>,
    algo: Option<String>,
) -> Result<Vec<SimilarImageGroup>, String> {
    let hash_algorithm = if algo.as_deref() == Some("average") {
        ImageHashAlgorithm::Average
    } else {
        ImageHashAlgorithm::Gradient
    };

    let config = SimilarImageConfig {
        included_directories: directories.into_iter().map(PathBuf::from).collect(),
        excluded_directories: exclude.into_iter().map(PathBuf::from).collect(),
        max_hamming_distance: distance.unwrap_or(4),
        hash_algorithm,
        min_file_size: 1024,
    };

    let token = CancellationToken::new();
    let finder = SimilarImageFinder::new(config, token);
    finder.find_similar_images(|_| {}).map_err(|e| e.to_string())
}

#[tauri::command]
fn scan_empty_files(
    directories: Vec<String>,
    exclude: Vec<String>,
) -> Result<Vec<ScannedFile>, String> {
    let config = EmptyFileFinderConfig {
        included_directories: directories.into_iter().map(PathBuf::from).collect(),
        excluded_directories: exclude.into_iter().map(PathBuf::from).collect(),
    };

    let token = CancellationToken::new();
    let finder = EmptyFileFinder::new(config, token);
    finder.find_empty_files().map_err(|e| e.to_string())
}

#[tauri::command]
fn scan_empty_folders(
    directories: Vec<String>,
    exclude: Vec<String>,
) -> Result<Vec<PathBuf>, String> {
    let config = EmptyFolderFinderConfig {
        included_directories: directories.into_iter().map(PathBuf::from).collect(),
        excluded_directories: exclude.into_iter().map(PathBuf::from).collect(),
    };

    let token = CancellationToken::new();
    let finder = EmptyFolderFinder::new(config, token);
    finder.find_empty_folders().map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            scan_duplicates,
            scan_similar_images,
            scan_empty_files,
            scan_empty_folders
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
