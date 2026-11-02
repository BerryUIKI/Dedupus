pub mod common;
pub mod duplicate;
pub mod empty_file;
pub mod empty_folder;

pub use common::{CancellationToken, HashAlgorithm, ProgressEvent, ScannedFile};
pub use duplicate::{DuplicateFinder, DuplicateFinderConfig, DuplicateGroup};
pub use empty_file::{EmptyFileFinder, EmptyFileFinderConfig};
pub use empty_folder::{EmptyFolderFinder, EmptyFolderFinderConfig};

#[cfg(test)]
mod tests {
    use super::*;
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
}
