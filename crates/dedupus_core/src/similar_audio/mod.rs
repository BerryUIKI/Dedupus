use crate::common::{
    CancellationToken, DiscoveryThrottle, ProgressCounter, ProgressEvent, ProgressReporter,
};
use anyhow::Result;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::File;
use std::path::{Path, PathBuf};
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use walkdir::WalkDir;

const SUPPORTED_AUDIO_EXTENSIONS: &[&str] = &["mp3", "wav", "flac", "ogg", "m4a", "aac"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarAudioConfig {
    pub included_directories: Vec<PathBuf>,
    pub excluded_directories: Vec<PathBuf>,
    pub max_duration_diff_secs: f64,
    pub max_distance: u32,
    pub min_file_size: u64,
}

impl Default for SimilarAudioConfig {
    fn default() -> Self {
        Self {
            included_directories: Vec::new(),
            excluded_directories: Vec::new(),
            max_duration_diff_secs: 2.0, // 2 seconds tolerance
            max_distance: 16,            // up to 16 bits difference in 128-bit acoustic hash
            min_file_size: 10 * 1024,    // 10KB minimum
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScannedAudio {
    pub path: PathBuf,
    pub size: u64,
    pub duration_secs: f64,
    pub sample_rate: u32,
    pub channels: u32,
    pub fingerprint: [u64; 2], // 128-bit acoustic envelope fingerprint
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarAudioGroup {
    pub similarity_info: String,
    pub tracks: Vec<ScannedAudio>,
}

pub struct SimilarAudioFinder {
    config: SimilarAudioConfig,
    cancellation_token: CancellationToken,
}

impl SimilarAudioFinder {
    pub fn new(config: SimilarAudioConfig, cancellation_token: CancellationToken) -> Self {
        Self {
            config,
            cancellation_token,
        }
    }

    pub fn find_similar_audio<F>(&self, mut on_progress: F) -> Result<Vec<SimilarAudioGroup>>
    where
        F: FnMut(ProgressEvent) + Send,
    {
        if self.cancellation_token.is_cancelled() {
            return Ok(Vec::new());
        }

        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Discovering audio files".to_string(),
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
                let is_audio = path
                    .extension()
                    .and_then(OsStr::to_str)
                    .map(|ext| {
                        let lower = ext.to_ascii_lowercase();
                        SUPPORTED_AUDIO_EXTENSIONS.contains(&lower.as_str())
                    })
                    .unwrap_or(false);

                if !is_audio {
                    continue;
                }

                if let Ok(metadata) = entry.metadata() {
                    if metadata.len() >= self.config.min_file_size {
                        candidate_paths.push((path.to_path_buf(), metadata.len()));
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

        // --- STAGE 2: Compute acoustic fingerprints in parallel ---
        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Fingerprinting audio".to_string(),
        });
        on_progress(ProgressEvent::HashingStarted {
            total_files: candidate_paths.len(),
        });

        let reporter = ProgressReporter::new(on_progress);
        let hash_counter = ProgressCounter::new(candidate_paths.len());

        let scanned_tracks: Vec<ScannedAudio> = candidate_paths
            .into_par_iter()
            .filter_map(|(path, size)| {
                if self.cancellation_token.is_cancelled() {
                    return None;
                }
                if let Some((processed, total)) = hash_counter.tick() {
                    reporter.report(ProgressEvent::HashingProgress { processed, total });
                }
                analyze_audio(&path, size).ok()
            })
            .collect();

        let mut on_progress = reporter.into_inner();

        if self.cancellation_token.is_cancelled() || scanned_tracks.len() < 2 {
            return Ok(Vec::new());
        }

        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Clustering similar audio tracks".to_string(),
        });

        // --- STAGE 3: Clustering using Union-Find ---
        let max_duration_diff = self.config.max_duration_diff_secs;
        let max_dist = self.config.max_distance;
        let n = scanned_tracks.len();
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

        #[allow(clippy::needless_range_loop)]
        for i in 0..n {
            if self.cancellation_token.is_cancelled() {
                return Ok(Vec::new());
            }
            let a1 = &scanned_tracks[i];

            for j in (i + 1)..n {
                let a2 = &scanned_tracks[j];

                // Check duration similarity
                if (a1.duration_secs - a2.duration_secs).abs() <= max_duration_diff {
                    let d0 = (a1.fingerprint[0] ^ a2.fingerprint[0]).count_ones();
                    let d1 = (a1.fingerprint[1] ^ a2.fingerprint[1]).count_ones();
                    let distance = d0 + d1;

                    if distance <= max_dist {
                        union(&mut parent, i, j);
                    }
                }
            }
        }

        // Group into sets
        let mut clusters: HashMap<usize, Vec<ScannedAudio>> = HashMap::new();
        for (i, audio) in scanned_tracks.into_iter().enumerate() {
            let root = find(&mut parent, i);
            clusters.entry(root).or_default().push(audio);
        }

        // Keep clusters with >= 2 tracks
        let mut groups: Vec<SimilarAudioGroup> = clusters
            .into_values()
            .filter(|list| list.len() >= 2)
            .map(|mut list| {
                // Sort by size descending (higher bitrate / quality first)
                list.sort_by_key(|b| std::cmp::Reverse(b.size));

                SimilarAudioGroup {
                    similarity_info: format!("Duration ~{:.1}s", list[0].duration_secs),
                    tracks: list,
                }
            })
            .collect();

        // Sort groups by track count descending
        groups.sort_by_key(|b| std::cmp::Reverse(b.tracks.len()));

        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Similar audio search finished".to_string(),
        });

        Ok(groups)
    }
}

/// Analyze an audio file using symphonia: extracts duration, channels, sample rate,
/// and computes a 128-bit acoustic envelope fingerprint from audio sample frames.
fn analyze_audio(path: &Path, size: u64) -> Result<ScannedAudio> {
    let file = File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(OsStr::to_str) {
        hint.with_extension(ext);
    }

    let format_opts = FormatOptions::default();
    let metadata_opts = MetadataOptions::default();

    let mut probed =
        symphonia::default::get_probe().format(&hint, mss, &format_opts, &metadata_opts)?;

    let (track_id, codec_params) = {
        let track = probed
            .format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
            .ok_or_else(|| anyhow::anyhow!("No valid audio track found"))?;
        (track.id, track.codec_params.clone())
    };

    let sample_rate = codec_params.sample_rate.unwrap_or(44100);
    let channels = codec_params.channels.map(|c| c.count() as u32).unwrap_or(2);

    // Calculate duration in seconds
    let duration_secs = if let Some(n_frames) = codec_params.n_frames {
        n_frames as f64 / sample_rate as f64
    } else {
        // Fallback estimate based on bitrate or default
        (size as f64 * 8.0) / (128_000.0) // Assume ~128kbps average if unknown
    };

    let decoder_opts = DecoderOptions::default();
    let mut decoder = symphonia::default::get_codecs().make(&codec_params, &decoder_opts)?;

    // Read up to the first 10 seconds of audio and bucket the per-sample energy
    // into 128 fixed time bins. Bucketing by absolute sample index keeps the
    // envelope independent of container/codec packet framing, so identical or
    // re-encoded tracks at the same sample rate stay comparable.
    let analysis_samples = (sample_rate as usize) * 10;
    let bucket_size = (analysis_samples / 128).max(1);
    let mut energies: [f64; 128] = [0.0; 128];
    let mut samples_seen: usize = 0;

    while samples_seen < analysis_samples {
        let packet = match probed.format.next_packet() {
            Ok(p) => p,
            Err(_) => break,
        };
        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(_) => break,
        };

        match decoded {
            AudioBufferRef::F32(buf) => {
                let plane = buf.chan(0);
                accumulate_energy(
                    &mut energies,
                    &mut samples_seen,
                    bucket_size,
                    analysis_samples,
                    plane.iter().map(|&s| s as f64),
                );
            }
            AudioBufferRef::S16(buf) => {
                let plane = buf.chan(0);
                accumulate_energy(
                    &mut energies,
                    &mut samples_seen,
                    bucket_size,
                    analysis_samples,
                    plane.iter().map(|&s| s as f64 / 32768.0),
                );
            }
            _ => {}
        }
    }

    // Convert energy envelope into 128-bit hash by comparing each slice to median/average
    let total_energy: f64 = energies.iter().sum();
    let avg_energy = total_energy / 128.0;

    let mut fp0 = 0u64;
    let mut fp1 = 0u64;

    for (i, &energy) in energies.iter().enumerate().take(64) {
        if energy >= avg_energy {
            fp0 |= 1u64 << i;
        }
    }
    for (i, &energy) in energies.iter().enumerate().skip(64) {
        if energy >= avg_energy {
            fp1 |= 1u64 << (i - 64);
        }
    }

    Ok(ScannedAudio {
        path: path.to_path_buf(),
        size,
        duration_secs,
        sample_rate,
        channels,
        fingerprint: [fp0, fp1],
    })
}

/// Accumulates squared sample amplitudes into 128 fixed time bins.
fn accumulate_energy<I: Iterator<Item = f64>>(
    energies: &mut [f64; 128],
    samples_seen: &mut usize,
    bucket_size: usize,
    analysis_samples: usize,
    samples: I,
) {
    for s in samples {
        if *samples_seen >= analysis_samples {
            break;
        }
        let bucket = *samples_seen / bucket_size;
        if bucket < 128 {
            energies[bucket] += s * s;
        }
        *samples_seen += 1;
    }
}
