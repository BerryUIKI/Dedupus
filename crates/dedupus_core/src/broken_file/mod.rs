use crate::common::{
    walk_files, CancellationToken, ProgressCounter, ProgressEvent, ProgressReporter,
};
use anyhow::Result;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

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
    pub check_shortcuts: bool,
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
            check_shortcuts: true,
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
    /// (image decoding, archive magic bytes, audio container probing,
    /// Windows shortcut resolution).
    pub fn find_broken_files<F>(&self, mut on_progress: F) -> Result<Vec<BrokenFile>>
    where
        F: FnMut(ProgressEvent) + Send,
    {
        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Discovering files".to_string(),
        });
        let mut broken: Vec<BrokenFile> = Vec::new();
        let mut candidates: Vec<(PathBuf, FileKind)> = Vec::new();

        let completed = walk_files(
            &self.config.included_directories,
            &self.config.excluded_directories,
            &self.cancellation_token,
            &mut on_progress,
            |entry| {
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
                    return;
                }

                if !entry.file_type().is_file() {
                    return;
                }

                if let Some(kind) = classify_file(entry.path(), &self.config) {
                    candidates.push((entry.path().to_path_buf(), kind));
                }
            },
        );

        if !completed {
            return Ok(broken);
        }

        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Validating files".to_string(),
        });
        on_progress(ProgressEvent::HashingStarted {
            total_files: candidates.len(),
        });

        let reporter = ProgressReporter::new(on_progress);
        let check_counter = ProgressCounter::new(candidates.len());

        let checked: Vec<BrokenFile> = candidates
            .into_par_iter()
            .filter_map(|(path, kind)| {
                if self.cancellation_token.is_cancelled() {
                    return None;
                }
                if let Some((processed, total)) = check_counter.tick() {
                    reporter.report(ProgressEvent::HashingProgress { processed, total });
                }

                let metadata = fs::metadata(&path).ok()?;
                let size = metadata.len();
                let modified_secs = metadata
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs());

                let reason = validate_file(&path, kind)?;

                Some(BrokenFile {
                    path,
                    size,
                    modified_secs,
                    reason,
                })
            })
            .collect();

        let mut on_progress = reporter.into_inner();

        broken.extend(checked);
        broken.sort_by_key(|b| std::cmp::Reverse(b.size));

        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Broken file search finished".to_string(),
        });

        Ok(broken)
    }
}

#[derive(Debug)]
enum FileKind {
    Image,
    Archive(String),
    Pdf,
    Audio,
    Shortcut,
}

fn classify_file(path: &Path, config: &BrokenFileFinderConfig) -> Option<FileKind> {
    let ext = path
        .extension()
        .and_then(OsStr::to_str)?
        .to_ascii_lowercase();

    if config.check_shortcuts && ext == "lnk" {
        return Some(FileKind::Shortcut);
    }
    if config.check_images && IMAGE_EXTENSIONS.contains(&ext.as_str()) {
        return Some(FileKind::Image);
    }
    if config.check_archives {
        if ARCHIVE_EXTENSIONS.contains(&ext.as_str()) {
            return Some(FileKind::Archive(ext));
        }
        if ext == "pdf" {
            return Some(FileKind::Pdf);
        }
    }
    if config.check_audio && AUDIO_EXTENSIONS.contains(&ext.as_str()) {
        return Some(FileKind::Audio);
    }

    None
}

/// Returns Some(reason) when the file is broken, None when it looks valid.
/// Files that cannot be opened are left alone (neither broken nor valid).
fn validate_file(path: &Path, kind: FileKind) -> Option<String> {
    match kind {
        FileKind::Shortcut => validate_shortcut(path),
        FileKind::Image => validate_image(path),
        FileKind::Archive(ext) => match archive_magic_valid(path, &ext) {
            Some(true) => None,
            Some(false) => Some(format!("Invalid archive: bad .{} header", ext)),
            None => None,
        },
        FileKind::Pdf => match header_contains(path, b"%PDF-") {
            Some(true) => None,
            Some(false) => Some("Invalid PDF: missing %PDF- header".to_string()),
            None => None,
        },
        FileKind::Audio => validate_audio(path),
    }
}

fn validate_image(path: &Path) -> Option<String> {
    let reader = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?;

    match reader.decode() {
        Ok(_) => None,
        Err(e) => Some(format!("Invalid image: {}", first_line(&e.to_string()))),
    }
}

fn validate_audio(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let mss = symphonia::core::io::MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = symphonia::core::probe::Hint::new();
    if let Some(ext) = path.extension().and_then(OsStr::to_str) {
        hint.with_extension(ext);
    }

    let probed = match symphonia::default::get_probe().format(
        &hint,
        mss,
        &symphonia::core::formats::FormatOptions::default(),
        &symphonia::core::meta::MetadataOptions::default(),
    ) {
        Ok(probed) => probed,
        Err(e) => return Some(format!("Invalid audio: {}", first_line(&e.to_string()))),
    };

    if probed.format.tracks().is_empty() {
        return Some("Invalid audio: no audio tracks".to_string());
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

/// Validates a Windows `.lnk` shortcut: the file must parse as a shell link,
/// and when it points at a local filesystem path that path must exist.
/// Network, virtual and environment-dependent targets are left alone.
#[cfg(windows)]
fn validate_shortcut(path: &Path) -> Option<String> {
    // The parser can hit debug assertions on deliberately corrupted input,
    // so treat a panic as an invalid shortcut instead of aborting the scan.
    let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        lnk::ShellLink::open(path, lnk::encoding::WINDOWS_1252)
    }));

    let link = match parsed {
        Ok(Ok(link)) => link,
        Ok(Err(error)) => {
            return Some(format!(
                "Invalid shortcut: {}",
                first_line(&error.to_string())
            ))
        }
        Err(_) => return Some("Invalid shortcut: parser error".to_string()),
    };

    let target = shortcut_target(&link)?;

    if target.is_empty()
        || target.contains('%')
        || target.contains("://")
        || target.starts_with("::{")
        || is_network_target(&target)
    {
        // URL, network or environment-dependent targets cannot be validated offline.
        return None;
    }

    if Path::new(&target).exists() {
        None
    } else {
        Some(format!("Broken shortcut: target not found ({})", target))
    }
}

#[cfg(not(windows))]
fn validate_shortcut(_path: &Path) -> Option<String> {
    // Resolving .lnk targets is only meaningful on Windows.
    None
}

/// Resolves the target path stored in a shortcut, either from the LinkInfo
/// structure or from the shell item ID list used by modern shortcuts.
#[cfg(windows)]
fn shortcut_target(link: &lnk::ShellLink) -> Option<String> {
    if let Some(info) = link.link_info().as_ref() {
        let flags = info.link_info_flags();
        if flags.has_volume_id_and_local_base_path()
            && !flags.has_common_network_relative_link_and_path_suffix()
        {
            let base = info
                .local_base_path_unicode()
                .clone()
                .or_else(|| info.local_base_path().map(ToOwned::to_owned))
                .unwrap_or_default();
            let suffix = info
                .common_path_suffix_unicode()
                .clone()
                .unwrap_or_else(|| info.common_path_suffix().to_string());
            return Some(join_shortcut_path(&base, &suffix));
        }
    }

    target_from_id_list(link)
}

/// Rebuilds a filesystem path from a shell item ID list (used by shortcuts
/// created through the shell API, which often carry no LinkInfo at all).
#[cfg(windows)]
fn target_from_id_list(link: &lnk::ShellLink) -> Option<String> {
    let list = link.linktarget_id_list().as_ref()?;
    let mut root = String::new();
    let mut components: Vec<String> = Vec::new();

    for item in list.id_list() {
        let data = item.data();
        let kind = *data.first()?;
        match kind {
            // Namespace roots ("This PC", known folders) contribute no path component.
            0x1f => {}
            // Volume item: drive letters as a null-terminated ASCII string.
            0x2f => {
                let end = data
                    .iter()
                    .position(|&byte| byte == 0)
                    .unwrap_or(data.len());
                if end <= 1 {
                    return None;
                }
                root = String::from_utf8_lossy(&data[1..end]).to_string();
                if root.is_empty() {
                    return None;
                }
            }
            // Filesystem directory / file entries.
            0x31 | 0x32 => {
                components.push(entry_name(data)?);
            }
            // Network, URI or other shell items cannot be verified offline.
            _ => return None,
        }
    }

    if root.is_empty() || components.is_empty() {
        return None;
    }

    let mut target = root;
    for component in components {
        target = join_shortcut_path(&target, &component);
    }
    Some(target)
}

/// Extracts the display name from a filesystem shell item ID, preferring the
/// long (Unicode) name stored in the extension block.
#[cfg(windows)]
fn entry_name(data: &[u8]) -> Option<String> {
    if data.len() < 13 {
        return None;
    }

    let nul = data[12..].iter().position(|&byte| byte == 0)? + 12;
    if nul == 12 {
        return None;
    }
    let short_name = String::from_utf8_lossy(&data[12..nul]).to_string();

    // The extension block starts after the null terminator, aligned to 2 bytes.
    let block_start = if (nul + 1) % 2 == 0 { nul + 1 } else { nul + 2 };
    if block_start + 8 > data.len() {
        return Some(short_name);
    }

    let size = u16::from_le_bytes([data[block_start], data[block_start + 1]]) as usize;
    let version = u16::from_le_bytes([data[block_start + 2], data[block_start + 3]]);
    let signature = u32::from_le_bytes([
        data[block_start + 4],
        data[block_start + 5],
        data[block_start + 6],
        data[block_start + 7],
    ]);

    if signature != 0xBEEF_0004 || version < 7 {
        return Some(short_name);
    }

    // Version 7+ blocks store the Unicode long name after size/version/signature
    // and the 20-byte timestamps/identifier header.
    let name_offset = block_start + 28;
    if size < 30 || block_start + size > data.len() || name_offset >= data.len() {
        return Some(short_name);
    }

    let mut units = Vec::new();
    let mut index = name_offset;
    while index + 1 < block_start + size {
        let unit = u16::from_le_bytes([data[index], data[index + 1]]);
        if unit == 0 {
            break;
        }
        units.push(unit);
        index += 2;
    }

    if units.is_empty() {
        Some(short_name)
    } else {
        Some(String::from_utf16_lossy(&units))
    }
}

#[cfg(windows)]
fn is_network_target(target: &str) -> bool {
    matches!(
        Path::new(target).components().next(),
        Some(std::path::Component::Prefix(prefix))
            if matches!(
                prefix.kind(),
                std::path::Prefix::UNC(..) | std::path::Prefix::VerbatimUNC(..)
            )
    )
}

#[cfg(windows)]
fn join_shortcut_path(base: &str, suffix: &str) -> String {
    if suffix.is_empty() {
        return base.to_string();
    }
    if base.is_empty() {
        return suffix.to_string();
    }
    let mut joined = std::path::PathBuf::from(base);
    joined.push(suffix);
    joined.to_string_lossy().into_owned()
}

#[cfg(all(test, windows))]
mod tests {
    use super::entry_name;

    fn shell_item(short_name: &[u8], long_name: &str) -> Vec<u8> {
        let mut data = vec![0u8; 12];
        data[0] = 0x31;
        data.extend_from_slice(short_name);
        data.push(0);
        if !data.len().is_multiple_of(2) {
            data.push(0);
        }

        let long_units: Vec<u8> = long_name
            .encode_utf16()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        let block_size = 30 + long_units.len();

        data.extend_from_slice(&(block_size as u16).to_le_bytes());
        data.extend_from_slice(&9u16.to_le_bytes());
        data.extend_from_slice(&0xBEEF_0004u32.to_le_bytes());
        data.extend_from_slice(&[0u8; 20]);
        data.extend_from_slice(&long_units);
        data.extend_from_slice(&[0, 0]);
        data
    }

    #[test]
    fn shell_item_prefers_unicode_long_name() {
        assert_eq!(
            entry_name(&shell_item(b"MYDOCU~1", "My Documents")),
            Some("My Documents".to_string())
        );
        assert_eq!(
            entry_name(&shell_item(b"PHOTO~1", "照片 2026")),
            Some("照片 2026".to_string())
        );
        assert_eq!(
            entry_name(&shell_item(b"Users", "Users")),
            Some("Users".to_string())
        );
    }
}
