pub mod cleaner;
pub mod common;
pub mod duplicate;
pub mod empty_file;
pub mod empty_folder;
pub mod similar_image;

pub use cleaner::{delete_permanently, move_to_trash, replace_with_hardlinks, CleanReport};
pub use common::{CancellationToken, HashAlgorithm, ProgressEvent, ScannedFile};
pub use duplicate::{DuplicateFinder, DuplicateFinderConfig, DuplicateGroup};
pub use empty_file::{EmptyFileFinder, EmptyFileFinderConfig};
pub use empty_folder::{EmptyFolderFinder, EmptyFolderFinderConfig};
pub use similar_image::{
    ImageHashAlgorithm, ScannedImage, SimilarImageConfig, SimilarImageFinder, SimilarImageGroup,
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
}
