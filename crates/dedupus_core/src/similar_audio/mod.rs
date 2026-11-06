use crate::common::{
    walk_files, CancellationToken, ProgressCounter, ProgressEvent, ProgressReporter, UnionFind,
};
use anyhow::Result;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::File;
use std::path::{Path, PathBuf};
use symphonia::core::audio::Signal;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

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
    pub duration_secs: Option<f64>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
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
        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Discovering audio files".to_string(),
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
                let is_audio = path.extension().and_then(OsStr::to_str).is_some_and(|ext| {
                    SUPPORTED_AUDIO_EXTENSIONS
                        .iter()
                        .any(|supported| supported.eq_ignore_ascii_case(ext))
                });
                if !is_audio {
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
        let mut disjoint = UnionFind::new(n);

        #[allow(clippy::needless_range_loop)]
        for i in 0..n {
            if self.cancellation_token.is_cancelled() {
                return Ok(Vec::new());
            }
            let a1 = &scanned_tracks[i];

            for j in (i + 1)..n {
                let a2 = &scanned_tracks[j];

                // Only reject on duration when both tracks report one.
                if let (Some(d1), Some(d2)) = (a1.duration_secs, a2.duration_secs) {
                    if (d1 - d2).abs() > max_duration_diff {
                        continue;
                    }
                }

                let d0 = (a1.fingerprint[0] ^ a2.fingerprint[0]).count_ones();
                let d1 = (a1.fingerprint[1] ^ a2.fingerprint[1]).count_ones();
                if d0 + d1 <= max_dist {
                    disjoint.union(i, j);
                }
            }
        }

        // Group into sets
        let mut clusters: HashMap<usize, Vec<ScannedAudio>> = HashMap::new();
        for (i, audio) in scanned_tracks.into_iter().enumerate() {
            let root = disjoint.find(i);
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
                    similarity_info: list[0]
                        .duration_secs
                        .map(|duration| format!("Duration ~{:.1}s", duration))
                        .unwrap_or_else(|| "Similar audio".to_string()),
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

    let mut sample_rate = codec_params.sample_rate;
    let mut channels = codec_params.channels.map(|c| c.count() as u32);

    // Read up to the first 10 seconds of audio and bucket the per-sample energy
    // into 128 fixed time bins. Bucketing by absolute sample index keeps the
    // envelope independent of container/codec packet framing, so identical or
    // re-encoded tracks at the same sample rate stay comparable.
    let analysis_samples = sample_rate.unwrap_or(44_100) as usize * 10;
    let bucket_size = (analysis_samples / 128).max(1);
    let mut energies: [f64; 128] = [0.0; 128];
    let mut samples_seen: usize = 0;

    let decoder_opts = DecoderOptions::default();
    let mut decoder = symphonia::default::get_codecs().make(&codec_params, &decoder_opts)?;

    while samples_seen < analysis_samples {
        let packet = match probed.format.next_packet() {
            Ok(p) => p,
            Err(_) => break,
        };
        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(_) => break,
        };

        // Convert every sample format (FLAC is S32, WAV may be S24/S32, ...) to f32.
        let mut converted = decoded.make_equivalent::<f32>();
        decoded.convert(&mut converted);

        sample_rate.get_or_insert(converted.spec().rate);
        channels.get_or_insert(converted.spec().channels.count() as u32);

        accumulate_energy(
            &mut energies,
            &mut samples_seen,
            bucket_size,
            analysis_samples,
            converted.chan(0).iter().map(|&s| s as f64),
        );
    }

    if samples_seen == 0 {
        return Err(anyhow::anyhow!("No audio samples decoded"));
    }

    let duration_secs = match (codec_params.n_frames, sample_rate) {
        (Some(frames), Some(rate)) => Some(frames as f64 / rate as f64),
        _ => None,
    };

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
