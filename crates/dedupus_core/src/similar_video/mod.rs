use crate::common::{
    walk_files, CancellationToken, ProgressCounter, ProgressEvent, ProgressReporter, UnionFind,
};
use crate::similar_image::compute_dhash;
use anyhow::Result;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SUPPORTED_VIDEO_EXTENSIONS: &[&str] =
    &["mp4", "mkv", "avi", "mov", "webm", "flv", "wmv", "m4v"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarVideoConfig {
    pub included_directories: Vec<PathBuf>,
    pub excluded_directories: Vec<PathBuf>,
    pub max_duration_diff_secs: f64,
    pub max_frame_distance: u8,
    pub min_file_size: u64,
}

impl Default for SimilarVideoConfig {
    fn default() -> Self {
        Self {
            included_directories: Vec::new(),
            excluded_directories: Vec::new(),
            max_duration_diff_secs: 2.0, // 2 seconds tolerance
            max_frame_distance: 10,      // Hamming distance threshold across frames
            min_file_size: 100 * 1024,   // 100KB minimum
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScannedVideo {
    pub path: PathBuf,
    pub size: u64,
    pub duration_secs: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub frame_hashes: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarVideoGroup {
    pub similarity_info: String,
    pub videos: Vec<ScannedVideo>,
}

pub struct SimilarVideoFinder {
    config: SimilarVideoConfig,
    cancellation_token: CancellationToken,
}

impl SimilarVideoFinder {
    pub fn new(config: SimilarVideoConfig, cancellation_token: CancellationToken) -> Self {
        Self {
            config,
            cancellation_token,
        }
    }

    pub fn find_similar_videos<F>(&self, mut on_progress: F) -> Result<Vec<SimilarVideoGroup>>
    where
        F: FnMut(ProgressEvent) + Send,
    {
        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Discovering video files".to_string(),
        });

        let mut candidate_paths = Vec::new();

        let completed = walk_files(
            &self.config.included_directories,
            &self.config.excluded_directories,
            &self.cancellation_token,
            &mut on_progress,
            |entry| {
                if !entry.file_type().is_file() {
                    return;
                }

                let path = entry.path();
                let is_video = path.extension().and_then(OsStr::to_str).is_some_and(|ext| {
                    SUPPORTED_VIDEO_EXTENSIONS
                        .iter()
                        .any(|supported| supported.eq_ignore_ascii_case(ext))
                });
                if !is_video {
                    return;
                }

                let Ok(metadata) = entry.metadata() else {
                    return;
                };
                if metadata.len() < self.config.min_file_size {
                    return;
                }

                candidate_paths.push((path.to_path_buf(), metadata.len()));
            },
        );

        if !completed || candidate_paths.len() < 2 {
            return Ok(Vec::new());
        }

        // --- STAGE 2: Extract video metadata & perceptual frame hashes ---
        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Analyzing video frames".to_string(),
        });
        on_progress(ProgressEvent::HashingStarted {
            total_files: candidate_paths.len(),
        });

        let has_ffmpeg = check_ffmpeg_available();

        let reporter = ProgressReporter::new(on_progress);
        let hash_counter = ProgressCounter::new(candidate_paths.len());

        let scanned_videos: Vec<ScannedVideo> = candidate_paths
            .into_par_iter()
            .filter_map(|(path, size)| {
                if self.cancellation_token.is_cancelled() {
                    return None;
                }
                if let Some((processed, total)) = hash_counter.tick() {
                    reporter.report(ProgressEvent::HashingProgress { processed, total });
                }
                analyze_video(&path, size, has_ffmpeg).ok()
            })
            .collect();

        let mut on_progress = reporter.into_inner();

        if self.cancellation_token.is_cancelled() || scanned_videos.len() < 2 {
            return Ok(Vec::new());
        }

        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Clustering similar videos".to_string(),
        });

        // --- STAGE 3: Clustering using Union-Find ---
        let max_duration_diff = self.config.max_duration_diff_secs;
        let max_dist = self.config.max_frame_distance;
        let n = scanned_videos.len();
        let mut disjoint = UnionFind::new(n);

        #[allow(clippy::needless_range_loop)]
        for i in 0..n {
            if self.cancellation_token.is_cancelled() {
                return Ok(Vec::new());
            }
            let v1 = &scanned_videos[i];

            for j in (i + 1)..n {
                let v2 = &scanned_videos[j];

                // Only reject on duration when both videos report one.
                if let (Some(d1), Some(d2)) = (v1.duration_secs, v2.duration_secs) {
                    if (d1 - d2).abs() > max_duration_diff {
                        continue;
                    }
                }

                // 2. Perceptual frame hash comparison
                if !v1.frame_hashes.is_empty() && !v2.frame_hashes.is_empty() {
                    let min_frames = v1.frame_hashes.len().min(v2.frame_hashes.len());
                    let mut total_distance: u32 = 0;

                    for k in 0..min_frames {
                        total_distance += (v1.frame_hashes[k] ^ v2.frame_hashes[k]).count_ones();
                    }

                    let avg_distance = (total_distance / min_frames as u32) as u8;
                    if avg_distance <= max_dist {
                        disjoint.union(i, j);
                    }
                } else if v1.size == v2.size {
                    // Fallback when ffmpeg is not available: matching size + duration
                    disjoint.union(i, j);
                }
            }
        }

        // Group into sets
        let mut clusters: HashMap<usize, Vec<ScannedVideo>> = HashMap::new();
        for (i, video) in scanned_videos.into_iter().enumerate() {
            let root = disjoint.find(i);
            clusters.entry(root).or_default().push(video);
        }

        // Keep clusters with >= 2 videos
        let mut groups: Vec<SimilarVideoGroup> = clusters
            .into_values()
            .filter(|list| list.len() >= 2)
            .map(|mut list| {
                // Sort by resolution descending, then size descending
                list.sort_by(|a, b| {
                    let res_a = a.width.unwrap_or(0) as u64 * a.height.unwrap_or(0) as u64;
                    let res_b = b.width.unwrap_or(0) as u64 * b.height.unwrap_or(0) as u64;
                    res_b.cmp(&res_a).then_with(|| b.size.cmp(&a.size))
                });

                let dur_str = list[0]
                    .duration_secs
                    .map(|d| format!("Duration ~{:.1}s", d))
                    .unwrap_or_else(|| "Similar Video".to_string());

                SimilarVideoGroup {
                    similarity_info: dur_str,
                    videos: list,
                }
            })
            .collect();

        groups.sort_by_key(|b| std::cmp::Reverse(b.videos.len()));

        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Similar video search finished".to_string(),
        });

        Ok(groups)
    }
}

fn create_command(program: &str) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn check_ffmpeg_available() -> bool {
    create_command("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Reads the container duration in seconds via ffprobe (best effort).
fn probe_duration(path: &Path) -> Option<f64> {
    let out = create_command("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;

    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|d| d.is_finite() && *d > 0.0)
}

fn analyze_video(path: &Path, size: u64, has_ffmpeg: bool) -> Result<ScannedVideo> {
    if !has_ffmpeg {
        return Ok(ScannedVideo {
            path: path.to_path_buf(),
            size,
            duration_secs: None,
            width: None,
            height: None,
            frame_hashes: Vec::new(),
        });
    }

    let duration = probe_duration(path);

    // Sample frames at ~10%, 50% and 80% of the runtime so the fingerprint
    // works for short clips too; fall back to fixed offsets when the
    // duration is unknown.
    let timestamps: Vec<String> = match duration {
        Some(d) => [0.1, 0.5, 0.8]
            .iter()
            .map(|p| format!("{:.3}", d * p))
            .collect(),
        None => vec![
            "00:00:02".to_string(),
            "00:00:10".to_string(),
            "00:00:20".to_string(),
        ],
    };

    let mut frame_hashes = Vec::new();
    let mut detected_width = None;
    let mut detected_height = None;

    for ts in &timestamps {
        let output = create_command("ffmpeg")
            .arg("-ss")
            .arg(ts)
            .arg("-i")
            .arg(path)
            .arg("-vframes")
            .arg("1")
            .arg("-f")
            .arg("image2pipe")
            .arg("-vcodec")
            .arg("png")
            .arg("-")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output();

        if let Ok(out) = output {
            if !out.stdout.is_empty() {
                if let Ok(img) = image::load_from_memory(&out.stdout) {
                    let (w, h) = (img.width(), img.height());
                    detected_width = Some(w);
                    detected_height = Some(h);
                    frame_hashes.push(compute_dhash(&img));
                }
            }
        }
    }

    Ok(ScannedVideo {
        path: path.to_path_buf(),
        size,
        duration_secs: duration,
        width: detected_width,
        height: detected_height,
        frame_hashes,
    })
}
