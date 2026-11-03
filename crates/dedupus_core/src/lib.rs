pub mod cache;
pub mod cleaner;
pub mod common;
pub mod duplicate;
pub mod empty_file;
pub mod empty_folder;
pub mod similar_audio;
pub mod similar_image;
pub mod similar_video;

pub use cache::ScanCache;
pub use cleaner::{delete_permanently, move_to_trash, replace_with_hardlinks, CleanReport};
pub use common::{CancellationToken, HashAlgorithm, ProgressEvent, ScannedFile};
pub use duplicate::{DuplicateFinder, DuplicateFinderConfig, DuplicateGroup};
pub use empty_file::{EmptyFileFinder, EmptyFileFinderConfig};
pub use empty_folder::{EmptyFolderFinder, EmptyFolderFinderConfig};
pub use similar_audio::{
    ScannedAudio, SimilarAudioConfig, SimilarAudioFinder, SimilarAudioGroup,
};
pub use similar_image::{
    ImageHashAlgorithm, ScannedImage, SimilarImageConfig, SimilarImageFinder, SimilarImageGroup,
};
pub use similar_video::{
    ScannedVideo, SimilarVideoConfig, SimilarVideoFinder, SimilarVideoGroup,
};

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
        File::create(&f3).unwrap().write_all(b"Different content").unwrap();

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
        let empty_files = finder.find_empty_files().unwrap();

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

        File::create(&f1).unwrap().write_all(b"Original content").unwrap();
        File::create(&f2).unwrap().write_all(b"Original content").unwrap();
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
        assert_eq!(reloaded.get_file_hash(&dummy_path, 100, 1000), Some("hash123"));
    }
}
