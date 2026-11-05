use crate::common::{
    CancellationToken, DiscoveryThrottle, ProgressCounter, ProgressEvent, ProgressReporter,
    ScannedFile,
};
use anyhow::Result;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::path::PathBuf;
use walkdir::WalkDir;

/// Extensions that intentionally carry no strong format guarantee.
/// Files with these names are skipped to avoid noisy false positives.
const GENERIC_EXTENSIONS: &[&str] = &[
    "tmp",
    "temp",
    "bak",
    "backup",
    "old",
    "orig",
    "part",
    "partial",
    "download",
    "crdownload",
    "dat",
    "bin",
    "blob",
    "cache",
    "copy",
];

/// Extensions that share a container format for which magic-byte detection
/// cannot tell the members apart (or where both names are common and valid).
const ZIP_FAMILY: &[&str] = &[
    "zip", "docx", "docm", "dotx", "dotm", "xlsx", "xlsm", "xltx", "xltm", "xlsb", "pptx", "pptm",
    "potx", "potm", "odt", "ott", "ods", "ots", "odp", "otp", "epub", "jar", "apk", "war", "ear",
    "cbz", "xpi", "whl", "ipa", "vsix", "nupkg", "kmz", "ora",
];
const ISO_BMFF_FAMILY: &[&str] = &[
    "mp4", "m4a", "m4b", "m4p", "m4v", "mov", "3gp", "3g2", "f4v", "heic", "heif", "hif", "avif",
    "avifs", "cr3",
];
const MATROSKA_FAMILY: &[&str] = &["mkv", "mka", "mks", "webm"];
const OGG_FAMILY: &[&str] = &["ogg", "oga", "ogv", "ogx", "opus", "spx"];
const MPEG_AUDIO_FAMILY: &[&str] = &["mp3", "mp2", "mpga", "mpa"];
const AAC_FAMILY: &[&str] = &["aac", "adts"];
const AMR_FAMILY: &[&str] = &["amr", "awb"];
const AIFF_FAMILY: &[&str] = &["aiff", "aif", "aifc"];
const MIDI_FAMILY: &[&str] = &["midi", "mid", "kar"];
const TIFF_FAMILY: &[&str] = &[
    "tif", "tiff", "nef", "arw", "dng", "orf", "rw2", "pef", "srw", "cr2",
];
const JPEG_FAMILY: &[&str] = &["jpg", "jpeg", "jpe", "jfif"];
const JP2_FAMILY: &[&str] = &["jp2", "j2k", "jpx", "jpf", "jpm"];
const PSD_FAMILY: &[&str] = &["psd", "psb"];
const JXR_FAMILY: &[&str] = &["jxr", "hdp", "wdp"];
const DWG_FAMILY: &[&str] = &["dwg", "dws", "dwt"];
const PE_FAMILY: &[&str] = &[
    "exe", "dll", "sys", "scr", "cpl", "ocx", "ax", "drv", "efi", "fon",
];
const ELF_FAMILY: &[&str] = &["elf", "so", "o", "ko"];
const MACH_FAMILY: &[&str] = &["mach", "dylib", "bundle"];
const FONT_FAMILY: &[&str] = &["ttf", "otf", "ttc"];
const XML_FAMILY: &[&str] = &[
    "xml", "svg", "xhtml", "xaml", "rss", "atom", "gpx", "kml", "xsd", "xsl", "xslt", "plist",
    "csproj", "props", "resx", "xul",
];
const HTML_FAMILY: &[&str] = &["html", "htm", "xhtml"];
const SCRIPT_FAMILY: &[&str] = &["sh", "bash", "zsh", "ksh"];
const ASF_FAMILY: &[&str] = &["wmv", "wma", "asf"];
const MPG_FAMILY: &[&str] = &["mpg", "mpeg", "m2v", "vob"];
const AVI_FAMILY: &[&str] = &["avi", "divx"];
const OLE_FAMILY: &[&str] = &[
    "doc", "dot", "xls", "xlt", "ppt", "pot", "pps", "msi", "msg", "vsd", "pub", "mpp",
];
const CERT_FAMILY: &[&str] = &["der", "cer", "crt", "pem"];
const POSTSCRIPT_FAMILY: &[&str] = &["ps", "eps"];
const PDF_FAMILY: &[&str] = &["pdf", "ai"];
const MOBI_FAMILY: &[&str] = &["mobi", "azw", "azw3"];
const AR_FAMILY: &[&str] = &["ar", "a", "lib"];
const GZIP_FAMILY: &[&str] = &["gz", "gzip", "tgz"];
const BZIP2_FAMILY: &[&str] = &["bz2", "bzip2", "tbz", "tbz2"];
const XZ_FAMILY: &[&str] = &["xz", "txz"];
const ZSTD_FAMILY: &[&str] = &["zst", "zstd", "tzst"];
const DICOM_FAMILY: &[&str] = &["dcm", "dicom"];

/// Formats for which `infer` knows the extension but the actual file may use
/// one of several conventional aliases.
fn compatible_extensions(detected: &str) -> &'static [&'static str] {
    match detected {
        "zip" | "docx" | "docm" | "xlsx" | "xlsm" | "pptx" | "pptm" | "odt" | "ods" | "odp"
        | "epub" | "ora" => ZIP_FAMILY,
        "mp4" | "m4a" | "m4v" | "mov" | "heif" | "avif" => ISO_BMFF_FAMILY,
        "mkv" | "webm" => MATROSKA_FAMILY,
        "ogg" | "opus" => OGG_FAMILY,
        "mp3" => MPEG_AUDIO_FAMILY,
        "aac" => AAC_FAMILY,
        "amr" => AMR_FAMILY,
        "aiff" => AIFF_FAMILY,
        "midi" => MIDI_FAMILY,
        "tif" | "cr2" => TIFF_FAMILY,
        "jpg" => JPEG_FAMILY,
        "jp2" => JP2_FAMILY,
        "psd" => PSD_FAMILY,
        "jxr" => JXR_FAMILY,
        "dwg" => DWG_FAMILY,
        "exe" | "dll" => PE_FAMILY,
        "elf" => ELF_FAMILY,
        "mach" => MACH_FAMILY,
        "ttf" | "otf" => FONT_FAMILY,
        "xml" => XML_FAMILY,
        "html" => HTML_FAMILY,
        "sh" => SCRIPT_FAMILY,
        "wmv" => ASF_FAMILY,
        "mpg" => MPG_FAMILY,
        "avi" => AVI_FAMILY,
        "doc" | "xls" | "ppt" | "msi" => OLE_FAMILY,
        "der" | "pem" => CERT_FAMILY,
        "ps" => POSTSCRIPT_FAMILY,
        "pdf" => PDF_FAMILY,
        "mobi" => MOBI_FAMILY,
        "ar" => AR_FAMILY,
        "gz" => GZIP_FAMILY,
        "bz2" => BZIP2_FAMILY,
        "xz" => XZ_FAMILY,
        "zst" => ZSTD_FAMILY,
        "dcm" => DICOM_FAMILY,
        _ => &[],
    }
}

fn extensions_match(detected: &str, actual: &str) -> bool {
    detected.eq_ignore_ascii_case(actual)
        || compatible_extensions(detected)
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(actual))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BadExtensionFinderConfig {
    pub included_directories: Vec<PathBuf>,
    pub excluded_directories: Vec<PathBuf>,
    pub min_file_size: u64,
}

impl Default for BadExtensionFinderConfig {
    fn default() -> Self {
        Self {
            included_directories: Vec::new(),
            excluded_directories: Vec::new(),
            min_file_size: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BadExtensionFile {
    pub path: PathBuf,
    pub size: u64,
    pub modified_secs: Option<u64>,
    /// The actual (lowercased) extension on disk.
    pub extension: String,
    /// The extension inferred from the file's content.
    pub detected_extension: String,
    pub detected_mime: String,
}

pub struct BadExtensionFinder {
    config: BadExtensionFinderConfig,
    cancellation_token: CancellationToken,
}

impl BadExtensionFinder {
    pub fn new(config: BadExtensionFinderConfig, cancellation_token: CancellationToken) -> Self {
        Self {
            config,
            cancellation_token,
        }
    }

    /// Finds files whose extension does not match the format detected from
    /// their leading bytes.
    pub fn find_bad_extensions<F>(&self, mut on_progress: F) -> Result<Vec<BadExtensionFile>>
    where
        F: FnMut(ProgressEvent) + Send,
    {
        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Scanning file contents".to_string(),
        });

        let mut candidates: Vec<ScannedFile> = Vec::new();
        let mut total_scanned = 0usize;
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

                total_scanned += 1;
                if discovery.should_emit() {
                    on_progress(ProgressEvent::FilesDiscovered {
                        count: total_scanned,
                    });
                }

                let path = entry.path();
                let Some(extension) = path.extension().and_then(OsStr::to_str) else {
                    continue;
                };
                if GENERIC_EXTENSIONS
                    .iter()
                    .any(|generic| extension.eq_ignore_ascii_case(generic))
                {
                    continue;
                }

                let metadata = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                if metadata.len() < self.config.min_file_size {
                    continue;
                }

                let modified_secs = metadata
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs());

                candidates.push(ScannedFile {
                    path: path.to_path_buf(),
                    size: metadata.len(),
                    modified_secs,
                });
            }
        }

        on_progress(ProgressEvent::FilesDiscovered {
            count: total_scanned,
        });

        if self.cancellation_token.is_cancelled() {
            return Ok(Vec::new());
        }

        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Checking file signatures".to_string(),
        });
        on_progress(ProgressEvent::HashingStarted {
            total_files: candidates.len(),
        });

        let reporter = ProgressReporter::new(on_progress);
        let check_counter = ProgressCounter::new(candidates.len());

        let mut found: Vec<BadExtensionFile> = candidates
            .into_par_iter()
            .filter_map(|file| {
                if self.cancellation_token.is_cancelled() {
                    return None;
                }
                if let Some((processed, total)) = check_counter.tick() {
                    reporter.report(ProgressEvent::HashingProgress { processed, total });
                }

                let actual = file.path.extension()?.to_str()?.to_ascii_lowercase();
                let kind = infer::get_from_path(&file.path).ok().flatten()?;

                if extensions_match(kind.extension(), &actual) {
                    return None;
                }

                Some(BadExtensionFile {
                    path: file.path,
                    size: file.size,
                    modified_secs: file.modified_secs,
                    extension: actual,
                    detected_extension: kind.extension().to_string(),
                    detected_mime: kind.mime_type().to_string(),
                })
            })
            .collect();

        let mut on_progress = reporter.into_inner();

        if self.cancellation_token.is_cancelled() {
            return Ok(Vec::new());
        }

        found.sort_by_key(|file| std::cmp::Reverse(file.size));

        on_progress(ProgressEvent::StageCompleted {
            stage_name: "Bad extension search finished".to_string(),
        });

        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::extensions_match;

    #[test]
    fn extension_families_cover_common_aliases() {
        assert!(extensions_match("jpg", "jpeg"));
        assert!(extensions_match("zip", "docx"));
        assert!(extensions_match("docx", "zip"));
        assert!(extensions_match("mp4", "mov"));
        assert!(extensions_match("mkv", "webm"));
        assert!(extensions_match("tif", "dng"));
        assert!(extensions_match("png", "png"));
        assert!(!extensions_match("png", "jpg"));
        assert!(!extensions_match("pdf", "zip"));
        assert!(!extensions_match("wav", "mp3"));
    }
}
