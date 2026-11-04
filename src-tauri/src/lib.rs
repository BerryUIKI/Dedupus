use dedupus_core::{
    delete_permanently as core_delete_permanently, move_to_trash as core_move_to_trash,
    replace_with_hardlinks as core_replace_with_hardlinks, BigFileFinder, BigFileFinderConfig,
    BrokenFile, BrokenFileFinder, BrokenFileFinderConfig, CancellationToken, CleanReport,
    DuplicateFinder, DuplicateFinderConfig, DuplicateGroup,
    EmptyFileFinder, EmptyFileFinderConfig, EmptyFolderFinder, EmptyFolderFinderConfig,
    HashAlgorithm, ImageHashAlgorithm, ScanCache, ScannedFile, SimilarAudioConfig,
    SimilarAudioFinder, SimilarAudioGroup, SimilarImageConfig, SimilarImageFinder,
    SimilarImageGroup, SimilarVideoConfig, SimilarVideoFinder, SimilarVideoGroup, TempFile,
    TempFileFinder, TempFileFinderConfig,
};
use std::path::PathBuf;

#[tauri::command]
fn scan_duplicates(
    directories: Vec<String>,
    exclude: Vec<String>,
    min_size: Option<u64>,
    max_size: Option<u64>,
    algo: Option<String>,
    use_cache: Option<bool>,
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
        use_cache: use_cache.unwrap_or(true),
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
    use_cache: Option<bool>,
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
        use_cache: use_cache.unwrap_or(true),
    };

    let token = CancellationToken::new();
    let finder = SimilarImageFinder::new(config, token);
    finder.find_similar_images(|_| {}).map_err(|e| e.to_string())
}

#[tauri::command]
fn scan_similar_audio(
    directories: Vec<String>,
    exclude: Vec<String>,
    max_duration_diff: Option<f64>,
) -> Result<Vec<SimilarAudioGroup>, String> {
    let config = SimilarAudioConfig {
        included_directories: directories.into_iter().map(PathBuf::from).collect(),
        excluded_directories: exclude.into_iter().map(PathBuf::from).collect(),
        max_duration_diff_secs: max_duration_diff.unwrap_or(2.0),
        max_distance: 16,
        min_file_size: 10 * 1024,
    };

    let token = CancellationToken::new();
    let finder = SimilarAudioFinder::new(config, token);
    finder.find_similar_audio(|_| {}).map_err(|e| e.to_string())
}

#[tauri::command]
fn scan_similar_videos(
    directories: Vec<String>,
    exclude: Vec<String>,
    max_duration_diff: Option<f64>,
) -> Result<Vec<SimilarVideoGroup>, String> {
    let config = SimilarVideoConfig {
        included_directories: directories.into_iter().map(PathBuf::from).collect(),
        excluded_directories: exclude.into_iter().map(PathBuf::from).collect(),
        max_duration_diff_secs: max_duration_diff.unwrap_or(2.0),
        max_frame_distance: 10,
        min_file_size: 100 * 1024,
    };

    let token = CancellationToken::new();
    let finder = SimilarVideoFinder::new(config, token);
    finder.find_similar_videos(|_| {}).map_err(|e| e.to_string())
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

#[tauri::command]
fn scan_big_files(
    directories: Vec<String>,
    exclude: Vec<String>,
    min_size: Option<u64>,
    max_results: Option<usize>,
) -> Result<Vec<ScannedFile>, String> {
    let config = BigFileFinderConfig {
        included_directories: directories.into_iter().map(PathBuf::from).collect(),
        excluded_directories: exclude.into_iter().map(PathBuf::from).collect(),
        min_file_size: min_size.unwrap_or(100 * 1024 * 1024),
        max_results: max_results.unwrap_or(100),
    };

    let token = CancellationToken::new();
    let finder = BigFileFinder::new(config, token);
    finder.find_big_files(|_| {}).map_err(|e| e.to_string())
}

#[tauri::command]
fn scan_temp_files(
    directories: Vec<String>,
    exclude: Vec<String>,
    min_size: Option<u64>,
) -> Result<Vec<TempFile>, String> {
    let config = TempFileFinderConfig {
        included_directories: directories.into_iter().map(PathBuf::from).collect(),
        excluded_directories: exclude.into_iter().map(PathBuf::from).collect(),
        min_file_size: min_size.unwrap_or(0),
        extra_extensions: Vec::new(),
    };

    let token = CancellationToken::new();
    let finder = TempFileFinder::new(config, token);
    finder.find_temp_files(|_| {}).map_err(|e| e.to_string())
}

#[tauri::command]
fn scan_broken_files(
    directories: Vec<String>,
    exclude: Vec<String>,
) -> Result<Vec<BrokenFile>, String> {
    let config = BrokenFileFinderConfig {
        included_directories: directories.into_iter().map(PathBuf::from).collect(),
        excluded_directories: exclude.into_iter().map(PathBuf::from).collect(),
        check_images: true,
        check_archives: true,
        check_audio: true,
        check_symlinks: true,
    };

    let token = CancellationToken::new();
    let finder = BrokenFileFinder::new(config, token);
    finder.find_broken_files(|_| {}).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_to_trash(paths: Vec<String>) -> Result<CleanReport, String> {
    let path_bufs: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    Ok(core_move_to_trash(&path_bufs))
}

#[tauri::command]
fn delete_permanently(paths: Vec<String>) -> Result<CleanReport, String> {
    let path_bufs: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    Ok(core_delete_permanently(&path_bufs))
}

#[tauri::command]
fn replace_with_hardlinks(target: String, duplicates: Vec<String>) -> Result<CleanReport, String> {
    let target_path = PathBuf::from(target);
    let dup_paths: Vec<PathBuf> = duplicates.into_iter().map(PathBuf::from).collect();
    Ok(core_replace_with_hardlinks(&target_path, &dup_paths))
}

#[tauri::command]
fn get_cache_stats() -> Result<(usize, usize), String> {
    let dup_cache = ScanCache::load_or_create("dup_cache");
    let img_cache = ScanCache::load_or_create("img_cache");
    Ok((dup_cache.file_hashes.len(), img_cache.image_hashes.len()))
}

#[tauri::command]
fn clear_cache() -> Result<(), String> {
    let mut dup_cache = ScanCache::load_or_create("dup_cache");
    let mut img_cache = ScanCache::load_or_create("img_cache");
    dup_cache.clear().map_err(|e| e.to_string())?;
    img_cache.clear().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            scan_duplicates,
            scan_similar_images,
            scan_similar_audio,
            scan_similar_videos,
            scan_empty_files,
            scan_empty_folders,
            scan_big_files,
            scan_temp_files,
            scan_broken_files,
            delete_to_trash,
            delete_permanently,
            replace_with_hardlinks,
            get_cache_stats,
            clear_cache
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
