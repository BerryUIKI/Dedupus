use dedupus_core::{
    delete_permanently as core_delete_permanently, move_to_trash as core_move_to_trash,
    replace_with_hardlinks as core_replace_with_hardlinks, BadExtensionFile, BadExtensionFinder,
    BadExtensionFinderConfig, BigFileFinder, BigFileFinderConfig, BrokenFile, BrokenFileFinder,
    BrokenFileFinderConfig, CancellationToken, CleanReport, DuplicateFinder, DuplicateFinderConfig,
    DuplicateGroup, EmptyFileFinder, EmptyFileFinderConfig, EmptyFolderFinder,
    EmptyFolderFinderConfig, HashAlgorithm, ImageHashAlgorithm, ProgressEvent, ScanCache,
    ScannedFile, SimilarAudioConfig, SimilarAudioFinder, SimilarAudioGroup, SimilarImageConfig,
    SimilarImageFinder, SimilarImageGroup, SimilarVideoConfig, SimilarVideoFinder,
    SimilarVideoGroup, TempFile, TempFileFinder, TempFileFinderConfig,
};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};

/// Coordinates the single in-flight scan and its cooperative cancellation token.
#[derive(Default)]
struct ScanState {
    token: Mutex<Option<CancellationToken>>,
}

impl ScanState {
    fn begin(&self) -> Result<CancellationToken, String> {
        let mut guard = self.token.lock().expect("scan state mutex poisoned");
        if guard.is_some() {
            return Err("A scan is already running".to_string());
        }
        let token = CancellationToken::new();
        *guard = Some(token.clone());
        Ok(token)
    }

    fn end(&self) {
        *self.token.lock().expect("scan state mutex poisoned") = None;
    }

    fn cancel(&self) {
        if let Some(token) = self
            .token
            .lock()
            .expect("scan state mutex poisoned")
            .as_ref()
        {
            token.cancel();
        }
    }
}

/// Clears the active scan even if the scan panics.
struct ScanGuard<'a> {
    state: &'a ScanState,
}

impl Drop for ScanGuard<'_> {
    fn drop(&mut self) {
        self.state.end();
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanOutcome<T> {
    results: T,
    cancelled: bool,
}

fn finish_scan<T, E: ToString>(
    token: &CancellationToken,
    result: Result<T, E>,
) -> Result<ScanOutcome<T>, String> {
    let cancelled = token.is_cancelled();
    result
        .map(|results| ScanOutcome { results, cancelled })
        .map_err(|error| error.to_string())
}

/// Progress payload sent to the webview as a `scan-progress` event.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanProgressPayload {
    scan_id: u64,
    kind: &'static str,
    event: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    processed: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total: Option<usize>,
}

fn progress_emitter(
    app: AppHandle,
    scan_id: u64,
    kind: &'static str,
) -> impl FnMut(ProgressEvent) + Send {
    move |event| {
        let mut payload = ScanProgressPayload {
            scan_id,
            kind,
            event: "",
            stage: None,
            path: None,
            count: None,
            processed: None,
            total: None,
        };

        match event {
            ProgressEvent::ScanningDirectory { path } => {
                payload.event = "directory";
                payload.path = Some(path.to_string_lossy().into_owned());
            }
            ProgressEvent::FilesDiscovered { count } => {
                payload.event = "discovered";
                payload.count = Some(count);
            }
            ProgressEvent::HashingStarted { total_files } => {
                payload.event = "hashing_started";
                payload.total = Some(total_files);
            }
            ProgressEvent::HashingProgress { processed, total } => {
                payload.event = "hashing";
                payload.processed = Some(processed);
                payload.total = Some(total);
            }
            ProgressEvent::StageCompleted { stage_name } => {
                payload.event = "stage";
                payload.stage = Some(stage_name);
            }
        }

        let _ = app.emit("scan-progress", payload);
    }
}

#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
fn scan_duplicates(
    app: AppHandle,
    state: State<'_, ScanState>,
    scan_id: u64,
    directories: Vec<String>,
    exclude: Vec<String>,
    min_size: Option<u64>,
    max_size: Option<u64>,
    algo: Option<String>,
    use_cache: Option<bool>,
) -> Result<ScanOutcome<Vec<DuplicateGroup>>, String> {
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

    let token = state.begin()?;
    let _guard = ScanGuard {
        state: state.inner(),
    };
    let finder = DuplicateFinder::new(config, token.clone());
    let result = finder.find_duplicates(progress_emitter(app, scan_id, "duplicates"));
    finish_scan(&token, result)
}

#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
fn scan_similar_images(
    app: AppHandle,
    state: State<'_, ScanState>,
    scan_id: u64,
    directories: Vec<String>,
    exclude: Vec<String>,
    distance: Option<u8>,
    algo: Option<String>,
    use_cache: Option<bool>,
) -> Result<ScanOutcome<Vec<SimilarImageGroup>>, String> {
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

    let token = state.begin()?;
    let _guard = ScanGuard {
        state: state.inner(),
    };
    let finder = SimilarImageFinder::new(config, token.clone());
    let result = finder.find_similar_images(progress_emitter(app, scan_id, "similar_images"));
    finish_scan(&token, result)
}

#[tauri::command(async)]
fn scan_similar_audio(
    app: AppHandle,
    state: State<'_, ScanState>,
    scan_id: u64,
    directories: Vec<String>,
    exclude: Vec<String>,
    max_duration_diff: Option<f64>,
) -> Result<ScanOutcome<Vec<SimilarAudioGroup>>, String> {
    let config = SimilarAudioConfig {
        included_directories: directories.into_iter().map(PathBuf::from).collect(),
        excluded_directories: exclude.into_iter().map(PathBuf::from).collect(),
        max_duration_diff_secs: max_duration_diff.unwrap_or(2.0),
        max_distance: 16,
        min_file_size: 10 * 1024,
    };

    let token = state.begin()?;
    let _guard = ScanGuard {
        state: state.inner(),
    };
    let finder = SimilarAudioFinder::new(config, token.clone());
    let result = finder.find_similar_audio(progress_emitter(app, scan_id, "similar_audio"));
    finish_scan(&token, result)
}

#[tauri::command(async)]
fn scan_similar_videos(
    app: AppHandle,
    state: State<'_, ScanState>,
    scan_id: u64,
    directories: Vec<String>,
    exclude: Vec<String>,
    max_duration_diff: Option<f64>,
) -> Result<ScanOutcome<Vec<SimilarVideoGroup>>, String> {
    let config = SimilarVideoConfig {
        included_directories: directories.into_iter().map(PathBuf::from).collect(),
        excluded_directories: exclude.into_iter().map(PathBuf::from).collect(),
        max_duration_diff_secs: max_duration_diff.unwrap_or(2.0),
        max_frame_distance: 10,
        min_file_size: 100 * 1024,
    };

    let token = state.begin()?;
    let _guard = ScanGuard {
        state: state.inner(),
    };
    let finder = SimilarVideoFinder::new(config, token.clone());
    let result = finder.find_similar_videos(progress_emitter(app, scan_id, "similar_video"));
    finish_scan(&token, result)
}

#[tauri::command(async)]
fn scan_empty_files(
    app: AppHandle,
    state: State<'_, ScanState>,
    scan_id: u64,
    directories: Vec<String>,
    exclude: Vec<String>,
) -> Result<ScanOutcome<Vec<ScannedFile>>, String> {
    let config = EmptyFileFinderConfig {
        included_directories: directories.into_iter().map(PathBuf::from).collect(),
        excluded_directories: exclude.into_iter().map(PathBuf::from).collect(),
    };

    let token = state.begin()?;
    let _guard = ScanGuard {
        state: state.inner(),
    };
    let finder = EmptyFileFinder::new(config, token.clone());
    let result = finder.find_empty_files(progress_emitter(app, scan_id, "empty_files"));
    finish_scan(&token, result)
}

#[tauri::command(async)]
fn scan_empty_folders(
    app: AppHandle,
    state: State<'_, ScanState>,
    scan_id: u64,
    directories: Vec<String>,
    exclude: Vec<String>,
) -> Result<ScanOutcome<Vec<PathBuf>>, String> {
    let config = EmptyFolderFinderConfig {
        included_directories: directories.into_iter().map(PathBuf::from).collect(),
        excluded_directories: exclude.into_iter().map(PathBuf::from).collect(),
    };

    let token = state.begin()?;
    let _guard = ScanGuard {
        state: state.inner(),
    };
    let finder = EmptyFolderFinder::new(config, token.clone());
    let result = finder.find_empty_folders(progress_emitter(app, scan_id, "empty_folders"));
    finish_scan(&token, result)
}

#[tauri::command(async)]
fn scan_big_files(
    app: AppHandle,
    state: State<'_, ScanState>,
    scan_id: u64,
    directories: Vec<String>,
    exclude: Vec<String>,
    min_size: Option<u64>,
    max_results: Option<usize>,
) -> Result<ScanOutcome<Vec<ScannedFile>>, String> {
    let config = BigFileFinderConfig {
        included_directories: directories.into_iter().map(PathBuf::from).collect(),
        excluded_directories: exclude.into_iter().map(PathBuf::from).collect(),
        min_file_size: min_size.unwrap_or(100 * 1024 * 1024),
        max_results: max_results.unwrap_or(100),
    };

    let token = state.begin()?;
    let _guard = ScanGuard {
        state: state.inner(),
    };
    let finder = BigFileFinder::new(config, token.clone());
    let result = finder.find_big_files(progress_emitter(app, scan_id, "big_files"));
    finish_scan(&token, result)
}

#[tauri::command(async)]
fn scan_temp_files(
    app: AppHandle,
    state: State<'_, ScanState>,
    scan_id: u64,
    directories: Vec<String>,
    exclude: Vec<String>,
    min_size: Option<u64>,
) -> Result<ScanOutcome<Vec<TempFile>>, String> {
    let config = TempFileFinderConfig {
        included_directories: directories.into_iter().map(PathBuf::from).collect(),
        excluded_directories: exclude.into_iter().map(PathBuf::from).collect(),
        min_file_size: min_size.unwrap_or(0),
        extra_extensions: Vec::new(),
    };

    let token = state.begin()?;
    let _guard = ScanGuard {
        state: state.inner(),
    };
    let finder = TempFileFinder::new(config, token.clone());
    let result = finder.find_temp_files(progress_emitter(app, scan_id, "temp_files"));
    finish_scan(&token, result)
}

#[tauri::command(async)]
fn scan_broken_files(
    app: AppHandle,
    state: State<'_, ScanState>,
    scan_id: u64,
    directories: Vec<String>,
    exclude: Vec<String>,
) -> Result<ScanOutcome<Vec<BrokenFile>>, String> {
    let config = BrokenFileFinderConfig {
        included_directories: directories.into_iter().map(PathBuf::from).collect(),
        excluded_directories: exclude.into_iter().map(PathBuf::from).collect(),
        check_images: true,
        check_archives: true,
        check_audio: true,
        check_symlinks: true,
        check_shortcuts: true,
    };

    let token = state.begin()?;
    let _guard = ScanGuard {
        state: state.inner(),
    };
    let finder = BrokenFileFinder::new(config, token.clone());
    let result = finder.find_broken_files(progress_emitter(app, scan_id, "broken_files"));
    finish_scan(&token, result)
}

#[tauri::command(async)]
fn scan_bad_extensions(
    app: AppHandle,
    state: State<'_, ScanState>,
    scan_id: u64,
    directories: Vec<String>,
    exclude: Vec<String>,
    min_size: Option<u64>,
) -> Result<ScanOutcome<Vec<BadExtensionFile>>, String> {
    let config = BadExtensionFinderConfig {
        included_directories: directories.into_iter().map(PathBuf::from).collect(),
        excluded_directories: exclude.into_iter().map(PathBuf::from).collect(),
        min_file_size: min_size.unwrap_or(1),
    };

    let token = state.begin()?;
    let _guard = ScanGuard {
        state: state.inner(),
    };
    let finder = BadExtensionFinder::new(config, token.clone());
    let result = finder.find_bad_extensions(progress_emitter(app, scan_id, "bad_extensions"));
    finish_scan(&token, result)
}

#[tauri::command]
fn cancel_scan(state: State<'_, ScanState>) {
    state.cancel();
}

#[tauri::command]
fn delete_to_trash(paths: Vec<PathBuf>) -> CleanReport {
    core_move_to_trash(&paths)
}

#[tauri::command]
fn delete_permanently(paths: Vec<PathBuf>) -> CleanReport {
    core_delete_permanently(&paths)
}

#[tauri::command]
fn replace_with_hardlinks(target: PathBuf, duplicates: Vec<PathBuf>) -> CleanReport {
    core_replace_with_hardlinks(&target, &duplicates)
}

#[tauri::command]
fn get_cache_stats() -> (usize, usize) {
    let dup_cache = ScanCache::load_or_create("dup_cache");
    let img_cache = ScanCache::load_or_create("img_cache");
    (dup_cache.file_hashes.len(), img_cache.image_hashes.len())
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
        .manage(ScanState::default())
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
            scan_bad_extensions,
            cancel_scan,
            delete_to_trash,
            delete_permanently,
            replace_with_hardlinks,
            get_cache_stats,
            clear_cache
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
