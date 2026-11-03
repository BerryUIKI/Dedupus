use crate::common::{CancellationToken, ProgressEvent};
use anyhow::Result;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "gif"];
const ARCHIVE_EXTENSIONS: &[&str] = &["zip", "gz", "7z", "rar", "bz2", "xz", "zst"];
const AUDIO_EXTENSIONS: &[&str] = &["mp3", "wav", "flac", "ogg", "m4a", "aac"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokenFile {
    pub path: PathBuf,
    pub size: u64,
    pub modified_secs: Option<u64>,
    /// Why the file was classified as broken.
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokenFileFinderConfig {
    pub included_directories: Vec<PathBuf>,
    pub excluded_directories: Vec<PathBuf>,
    pub check_images: bool,
    pub check_archives: bool,
    pub check_audio: bool,
    pub check_symlinks: bool,
}

impl Default for BrokenFileFinderConfig {
    fn default() -> Self {
        Self {
            included_directories: Vec::new(),
            excluded_directories: Vec::new(),
            check_images: true,
            check_archives: true,
            check_audio: true,
            check_symlinks: true,
        }
    }
}

pub struct BrokenFileFinder {
    config: BrokenFileFinderConfig,
    cancellation_token: CancellationToken,
}

impl BrokenFileFinder {
    pub fn new(config: BrokenFileFinderConfig, cancellation_token: CancellationToken) -> Self {
        Self {
            config,
            cancellation_token,
        }
    }

    /// Finds broken symlinks and files that fail lightweight validation
    /// (image decoding, archive magic bytes, audio container probing).
    pub fn find_broken_files<F>(&self, mut on_progress: F) -> Result<Vec<BrokenFile>>
    where
        F: FnMut(ProgressEvent),
    {
        let mut broken: Vec<BrokenFile> = Vec::new();
        let mut candidates: Vec<PathBuf> = Vec::new();
        let mut total_scanned = 0usize;

        for dir in &self.config.included_directories {
            if self.cancellation_token.is_cancelled() {
                break;
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
                    break;
                }

                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                // Broken symlink: the link itself exists but its target does not.
                if entry.file_type().is_symlink() {
                    if self.config.check_symlinks && fs::metadata(entry.path()).is_err() {
                        broken.push(BrokenFile {
                            path: entry.path().to_path_buf(),
                            size: 0,
                            modified_secs: None,
                            reason: "Broken symlink".to_string(),
                        });
                    }
                    continue;
                }

                if !entry.file_type().is_file() {
                    continue;
                }

                total_scanned += 1;
                if total_scanned.is_multiple_of(500) {
                    on_progress(ProgressEvent::FilesDiscovered {
                        count: total_scanned,
                    });
                }

                if is_checkable(entry.path(), &self.config) {
                    candidates.push(entry.path().to_path_buf());
                }
            }
        }

        on_progress(ProgressEvent::FilesDiscovered {
            count: total_scanned,
        });

        if self.cancellation_token.is_cancelled() {
            return Ok(broken);
        }

        let checked: Vec<BrokenFile> = candidates
            .into_par_iter()
            .filter_map(|path| {
                if self.cancellation_token.is_cancelled() {
                    return None;
                }

                let metadata = fs::metadata(&path).ok()?;
                let size = metadata.len();
                let modified_secs = metadata
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs());

                let reason = validate_file(&path, &self.config)?;

                Some(BrokenFile {
                    path,
                    size,
                    modified_secs,
                    reason,
                })
            })
            .collect();

        broken.extend(checked);
        broken.sort_by_key(|b| std::cmp::Reverse(b.size));

        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Broken file search finished".to_string(),
        });

        Ok(broken)
    }
}

fn is_checkable(path: &Path, config: &BrokenFileFinderConfig) -> bool {
    let ext = match path.extension().and_then(OsStr::to_str) {
        Some(e) => e.to_ascii_lowercase(),
        None => return false,
    };

    (config.check_images && IMAGE_EXTENSIONS.contains(&ext.as_str()))
        || (config.check_archives
            && (ARCHIVE_EXTENSIONS.contains(&ext.as_str()) || ext == "pdf"))
        || (config.check_audio && AUDIO_EXTENSIONS.contains(&ext.as_str()))
}

/// Returns Some(reason) when the file is broken, None when it looks valid.
fn validate_file(path: &Path, config: &BrokenFileFinderConfig) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();

    if config.check_images && IMAGE_EXTENSIONS.contains(&ext.as_str()) {
        return match image::ImageReader::open(path)
            .ok()?
            .with_guessed_format()
            .ok()?
            .decode()
        {
            Ok(_) => None,
            Err(e) => Some(format!("Invalid image: {}", first_line(&e.to_string()))),
        };
    }

    if config.check_archives {
        if ARCHIVE_EXTENSIONS.contains(&ext.as_str()) {
            return match archive_magic_valid(path, &ext) {
                Some(true) => None,
                Some(false) => Some(format!("Invalid archive: bad .{} header", ext)),
                None => None, // unreadable; leave it alone
            };
        }
        if ext == "pdf" {
            return match header_contains(path, b"%PDF-") {
                Some(true) => None,
                Some(false) => Some("Invalid PDF: missing %PDF- header".to_string()),
                None => None,
            };
        }
    }

    if config.check_audio && AUDIO_EXTENSIONS.contains(&ext.as_str()) {
        return match probe_audio(path) {
            Ok(()) => None,
            Err(e) => Some(format!("Invalid audio: {}", first_line(&e))),
        };
    }

    None
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or(text).trim().to_string()
}

fn read_header(path: &Path, len: usize) -> Option<Vec<u8>> {
    let mut file = File::open(path).ok()?;
    let mut buf = vec![0u8; len];
    let read = file.read(&mut buf).ok()?;
    buf.truncate(read);
    Some(buf)
}

fn archive_magic_valid(path: &Path, ext: &str) -> Option<bool> {
    let header = read_header(path, 8)?;
    let valid = match ext {
        "zip" => {
            header.starts_with(b"PK\x03\x04")
                || header.starts_with(b"PK\x05\x06")
                || header.starts_with(b"PK\x07\x08")
        }
        "gz" => header.starts_with(&[0x1F, 0x8B]),
        "7z" => header.starts_with(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]),
        "rar" => header.starts_with(b"Rar!"),
        "bz2" => header.starts_with(b"BZh"),
        "xz" => header.starts_with(&[0xFD, b'7', b'z', b'X', b'Z', 0x00]),
        "zst" => header.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]),
        _ => return None,
    };
    Some(valid)
}

fn header_contains(path: &Path, needle: &[u8]) -> Option<bool> {
    let header = read_header(path, 1024)?;
    Some(header.windows(needle.len()).any(|w| w == needle))
}

fn probe_audio(path: &Path) -> std::result::Result<(), String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mss = symphonia::core::io::MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = symphonia::core::probe::Hint::new();
    if let Some(ext) = path.extension().and_then(OsStr::to_str) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &symphonia::core::formats::FormatOptions::default(),
            &symphonia::core::meta::MetadataOptions::default(),
        )
        .map_err(|e| e.to_string())?;

    if probed.format.tracks().is_empty() {
        return Err("no audio tracks".to_string());
    }

    Ok(())
}
