pub mod bad_extension;
pub mod big_file;
pub mod broken_file;
pub mod cache;
pub mod cleaner;
pub mod common;
pub mod duplicate;
pub mod empty_file;
pub mod empty_folder;
pub mod similar_audio;
pub mod similar_image;
pub mod similar_video;
pub mod temp_file;

pub use bad_extension::{BadExtensionFile, BadExtensionFinder, BadExtensionFinderConfig};
pub use big_file::{BigFileFinder, BigFileFinderConfig};
pub use broken_file::{BrokenFile, BrokenFileFinder, BrokenFileFinderConfig};
pub use cache::ScanCache;
pub use cleaner::{delete_permanently, move_to_trash, replace_with_hardlinks, CleanReport};
pub use common::{
    CancellationToken, HashAlgorithm, ProgressCounter, ProgressEvent, ProgressReporter, ScannedFile,
};
pub use duplicate::{DuplicateFinder, DuplicateFinderConfig, DuplicateGroup};
pub use empty_file::{EmptyFileFinder, EmptyFileFinderConfig};
pub use empty_folder::{EmptyFolderFinder, EmptyFolderFinderConfig};
pub use similar_audio::{ScannedAudio, SimilarAudioConfig, SimilarAudioFinder, SimilarAudioGroup};
pub use similar_image::{
    ImageHashAlgorithm, ScannedImage, SimilarImageConfig, SimilarImageFinder, SimilarImageGroup,
};
pub use similar_video::{ScannedVideo, SimilarVideoConfig, SimilarVideoFinder, SimilarVideoGroup};
pub use temp_file::{TempFile, TempFileFinder, TempFileFinderConfig};

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn test_duplicate_finder() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path();

        // Create 2 duplicate files
        let f1 = path.join("file1.txt");
        let f2 = path.join("file2.txt");
        let f3 = path.join("unique.txt");

        let content = b"Dedupus duplicate content test 1234567890";
        File::create(&f1).unwrap().write_all(content).unwrap();
        File::create(&f2).unwrap().write_all(content).unwrap();
        File::create(&f3)
            .unwrap()
            .write_all(b"Different content")
            .unwrap();

        let config = DuplicateFinderConfig {
            included_directories: vec![path.to_path_buf()],
            ..Default::default()
        };
        let token = CancellationToken::new();
        let finder = DuplicateFinder::new(config, token);

        let duplicates = finder.find_duplicates(|_| {}).unwrap();
        assert_eq!(duplicates.len(), 1);
        assert_eq!(duplicates[0].files.len(), 2);
    }

    #[test]
    fn test_duplicate_scan_cancellation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path();

        let content = vec![7u8; 64 * 1024];
        for i in 0..4 {
            File::create(path.join(format!("copy{}.bin", i)))
                .unwrap()
                .write_all(&content)
                .unwrap();
        }

        let config = DuplicateFinderConfig {
            included_directories: vec![path.to_path_buf()],
            ..Default::default()
        };
        let token = CancellationToken::new();
        let cancel_token = token.clone();
        let finder = DuplicateFinder::new(config, token.clone());

        let duplicates = finder
            .find_duplicates(|event| {
                if matches!(event, ProgressEvent::HashingStarted { .. }) {
                    cancel_token.cancel();
                }
            })
            .unwrap();

        assert!(token.is_cancelled());
        assert!(duplicates.is_empty());
    }

    #[test]
    fn test_duplicate_hashing_progress_events() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path();

        let content = vec![3u8; 32 * 1024];
        for i in 0..3 {
            File::create(path.join(format!("same{}.bin", i)))
                .unwrap()
                .write_all(&content)
                .unwrap();
        }

        let config = DuplicateFinderConfig {
            included_directories: vec![path.to_path_buf()],
            ..Default::default()
        };
        let token = CancellationToken::new();
        let finder = DuplicateFinder::new(config, token);

        let mut progress = Vec::new();
        let duplicates = finder
            .find_duplicates(|event| {
                if let ProgressEvent::HashingProgress { processed, total } = event {
                    progress.push((processed, total));
                }
            })
            .unwrap();

        assert_eq!(duplicates.len(), 1);
        assert!(
            progress
                .iter()
                .any(|(processed, total)| processed == total && *total >= 3),
            "expected a completed hashing progress update, got {:?}",
            progress
        );
    }

    #[test]
    fn test_empty_file_finder() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path();

        let empty = path.join("empty.txt");
        File::create(&empty).unwrap();

        let non_empty = path.join("not_empty.txt");
        File::create(&non_empty).unwrap().write_all(b"abc").unwrap();

        let config = EmptyFileFinderConfig {
            included_directories: vec![path.to_path_buf()],
            excluded_directories: vec![],
        };
        let token = CancellationToken::new();
        let finder = EmptyFileFinder::new(config, token);
        let empty_files = finder.find_empty_files(|_| {}).unwrap();

        assert_eq!(empty_files.len(), 1);
        assert_eq!(empty_files[0].path, empty);
    }

    #[test]
    fn test_similar_image_finder() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path();

        // Create 2 identical gradient images and 1 opposite gradient image
        let img1_path = path.join("img1.png");
        let img2_path = path.join("img2.png");
        let img3_path = path.join("img3.png");

        let mut grad_img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::new(64, 64);
        for (x, _y, pixel) in grad_img.enumerate_pixels_mut() {
            let val = (x * 4).min(255) as u8;
            *pixel = Rgb([val, val, val]);
        }

        let mut inv_grad_img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::new(64, 64);
        for (x, _y, pixel) in inv_grad_img.enumerate_pixels_mut() {
            let val = ((63 - x) * 4).min(255) as u8;
            *pixel = Rgb([val, val, val]);
        }

        grad_img.save(&img1_path).unwrap();
        grad_img.save(&img2_path).unwrap();
        inv_grad_img.save(&img3_path).unwrap();

        let config = SimilarImageConfig {
            included_directories: vec![path.to_path_buf()],
            excluded_directories: vec![],
            max_hamming_distance: 2,
            hash_algorithm: ImageHashAlgorithm::Gradient,
            min_file_size: 10,
            use_cache: false,
        };
        let token = CancellationToken::new();
        let finder = SimilarImageFinder::new(config, token);
        let groups = finder.find_similar_images(|_| {}).unwrap();

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].images.len(), 2);
    }

    #[test]
    fn test_cleaner_actions() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path();

        let f1 = path.join("target.txt");
        let f2 = path.join("dup1.txt");
        let f3 = path.join("delete_me.txt");

        File::create(&f1)
            .unwrap()
            .write_all(b"Original content")
            .unwrap();
        File::create(&f2)
            .unwrap()
            .write_all(b"Original content")
            .unwrap();
        File::create(&f3).unwrap().write_all(b"Delete me").unwrap();

        // Test permanent deletion
        let del_report = delete_permanently(std::slice::from_ref(&f3));
        assert_eq!(del_report.successful.len(), 1);
        assert!(!f3.exists());

        // Test hardlinking
        let link_report = replace_with_hardlinks(&f1, std::slice::from_ref(&f2));
        assert_eq!(link_report.successful.len(), 1);
        assert!(f2.exists());
        assert_eq!(std::fs::read_to_string(&f2).unwrap(), "Original content");

        // Test trash
        let trash_report = move_to_trash(std::slice::from_ref(&f2));
        assert_eq!(trash_report.successful.len(), 1);
        assert!(!f2.exists());
    }

    fn write_wav(path: &std::path::Path, seconds: f64, freq_hz: f64) {
        let sample_rate = 44_100u32;
        let num_samples = (sample_rate as f64 * seconds) as u32;
        let mut data = Vec::with_capacity(num_samples as usize * 2);

        for i in 0..num_samples {
            let t = i as f64 / sample_rate as f64;
            let sample = (2.0 * std::f64::consts::PI * freq_hz * t).sin();
            let value = (sample * 12_000.0) as i16;
            data.extend_from_slice(&value.to_le_bytes());
        }

        let data_len = data.len() as u32;
        let mut wav = Vec::with_capacity(44 + data.len());
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        wav.extend_from_slice(&data);

        std::fs::write(path, wav).unwrap();
    }

    #[test]
    fn test_similar_audio_finder() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path();

        let a1 = path.join("track1.wav");
        let a2 = path.join("track2.wav");
        let a3 = path.join("short.wav");

        write_wav(&a1, 1.0, 440.0);
        write_wav(&a2, 1.0, 440.0);
        write_wav(&a3, 0.2, 440.0);

        let config = SimilarAudioConfig {
            included_directories: vec![path.to_path_buf()],
            excluded_directories: vec![],
            max_duration_diff_secs: 0.5,
            max_distance: 16,
            min_file_size: 100,
        };
        let token = CancellationToken::new();
        let finder = SimilarAudioFinder::new(config, token);
        let groups = finder.find_similar_audio(|_| {}).unwrap();

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].tracks.len(), 2);
    }

    #[test]
    fn test_similar_video_finder() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path();

        let v1 = path.join("video1.mp4");
        let v2 = path.join("video2.mp4");
        let v3 = path.join("video3.mp4");

        // Identical-size files cluster via the size fallback (no decodable frames)
        std::fs::write(&v1, vec![0u8; 2048]).unwrap();
        std::fs::write(&v2, vec![0u8; 2048]).unwrap();
        std::fs::write(&v3, vec![0u8; 4096]).unwrap();

        let config = SimilarVideoConfig {
            included_directories: vec![path.to_path_buf()],
            excluded_directories: vec![],
            max_duration_diff_secs: 2.0,
            max_frame_distance: 10,
            min_file_size: 100,
        };
        let token = CancellationToken::new();
        let finder = SimilarVideoFinder::new(config, token);
        let groups = finder.find_similar_videos(|_| {}).unwrap();

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].videos.len(), 2);
    }

    #[test]
    fn test_big_file_finder() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path();

        let small = path.join("small.bin");
        let medium = path.join("medium.bin");
        let large = path.join("large.bin");

        std::fs::write(&small, vec![0u8; 500]).unwrap();
        std::fs::write(&medium, vec![0u8; 1500]).unwrap();
        std::fs::write(&large, vec![0u8; 3000]).unwrap();

        let config = BigFileFinderConfig {
            included_directories: vec![path.to_path_buf()],
            excluded_directories: vec![],
            min_file_size: 1000,
            max_results: 2,
        };
        let token = CancellationToken::new();
        let finder = BigFileFinder::new(config, token);
        let files = finder.find_big_files(|_| {}).unwrap();

        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, large);
        assert_eq!(files[1].path, medium);
    }

    #[test]
    fn test_temp_file_finder() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path();

        std::fs::write(path.join("report.txt~"), b"backup").unwrap();
        std::fs::write(path.join("~$slides.pptx"), b"lock").unwrap();
        std::fs::write(path.join("download.part"), b"partial").unwrap();
        std::fs::write(path.join("Thumbs.db"), b"junk").unwrap();
        std::fs::write(path.join("keep.txt"), b"keep me").unwrap();

        let config = TempFileFinderConfig {
            included_directories: vec![path.to_path_buf()],
            excluded_directories: vec![],
            min_file_size: 0,
            extra_extensions: vec![],
        };
        let token = CancellationToken::new();
        let finder = TempFileFinder::new(config, token);
        let found = finder.find_temp_files(|_| {}).unwrap();

        assert_eq!(found.len(), 4);
        assert!(found.iter().all(|f| !f.path.ends_with("keep.txt")));
        assert!(found.iter().any(|f| f.reason == "Office lock file"));
        assert!(found.iter().any(|f| f.reason == "Backup copy"));
        assert!(found.iter().any(|f| f.reason == "Extension .part"));
        assert!(found.iter().any(|f| f.reason == "OS junk file"));
    }

    #[test]
    fn test_broken_file_finder() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path();

        // Valid PNG
        let valid_png: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_pixel(8, 8, Rgb([10, 20, 30]));
        valid_png.save(path.join("valid.png")).unwrap();

        // Corrupted image (garbage with a png extension)
        std::fs::write(path.join("broken.png"), b"this is not a png").unwrap();

        // Corrupted archive and PDF
        std::fs::write(path.join("bad.zip"), b"not a zip file").unwrap();
        std::fs::write(path.join("broken.pdf"), b"just some text").unwrap();

        // Valid archive header and PDF header are not reported
        std::fs::write(path.join("good.zip"), b"PK\x03\x04rest-of-data").unwrap();
        std::fs::write(path.join("good.pdf"), b"%PDF-1.7\nrest").unwrap();

        // Unrelated file is ignored
        std::fs::write(path.join("notes.txt"), b"hello").unwrap();

        let config = BrokenFileFinderConfig {
            included_directories: vec![path.to_path_buf()],
            excluded_directories: vec![],
            check_images: true,
            check_archives: true,
            check_audio: true,
            check_symlinks: true,
        };
        let token = CancellationToken::new();
        let finder = BrokenFileFinder::new(config, token);
        let broken = finder.find_broken_files(|_| {}).unwrap();

        assert_eq!(broken.len(), 3);
        assert!(broken.iter().any(|b| b.path.ends_with("broken.png")));
        assert!(broken.iter().any(|b| b.path.ends_with("bad.zip")));
        assert!(broken.iter().any(|b| b.path.ends_with("broken.pdf")));
        assert!(!broken.iter().any(|b| b.path.ends_with("valid.png")));
        assert!(!broken.iter().any(|b| b.path.ends_with("good.zip")));
        assert!(!broken.iter().any(|b| b.path.ends_with("notes.txt")));
    }

    #[test]
    fn test_bad_extension_finder() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path();

        let img = image::RgbImage::from_fn(16, 16, |x, y| {
            image::Rgb([(x * 16) as u8, (y * 16) as u8, 128])
        });

        // PNG bytes stored behind a .jpg name: should be reported.
        img.save_with_format(path.join("photo.jpg"), image::ImageFormat::Png)
            .unwrap();
        // Properly named PNG.
        img.save_with_format(path.join("proper.png"), image::ImageFormat::Png)
            .unwrap();
        // JPEG bytes with the .jpeg alias: same family, not reported.
        img.save_with_format(path.join("proper.jpeg"), image::ImageFormat::Jpeg)
            .unwrap();
        // Plain text has no detectable signature.
        File::create(path.join("notes.txt"))
            .unwrap()
            .write_all(b"hello")
            .unwrap();
        // Generic names such as .dat are intentionally ignored.
        std::fs::copy(path.join("proper.png"), path.join("backup.dat")).unwrap();

        let config = BadExtensionFinderConfig {
            included_directories: vec![path.to_path_buf()],
            excluded_directories: vec![],
            min_file_size: 1,
        };
        let token = CancellationToken::new();
        let finder = BadExtensionFinder::new(config, token);
        let found = finder.find_bad_extensions(|_| {}).unwrap();

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, path.join("photo.jpg"));
        assert_eq!(found[0].extension, "jpg");
        assert_eq!(found[0].detected_extension, "png");
    }

    #[test]
    fn test_scan_cache() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_file = temp_dir.path().join("test_cache.json");

        let mut cache = ScanCache::load_from_path(&cache_file).unwrap();
        let dummy_path = std::path::PathBuf::from("test/file.txt");

        // Initially empty
        assert_eq!(cache.get_file_hash(&dummy_path, 100, 1000), None);

        // Insert and verify
        cache.insert_file_hash(dummy_path.clone(), 100, 1000, "hash123".to_string());
        assert_eq!(cache.get_file_hash(&dummy_path, 100, 1000), Some("hash123"));

        // Save and reload
        cache.save().unwrap();
        let reloaded = ScanCache::load_from_path(&cache_file).unwrap();
        assert_eq!(
            reloaded.get_file_hash(&dummy_path, 100, 1000),
            Some("hash123")
        );
    }
}
