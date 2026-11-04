use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedFileEntry {
    pub size: u64,
    pub modified_secs: u64,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedImageEntry {
    pub size: u64,
    pub modified_secs: u64,
    pub width: u32,
    pub height: u32,
    pub hash: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScanCache {
    /// Maps full path -> cached file hash
    pub file_hashes: HashMap<PathBuf, CachedFileEntry>,
    /// Maps full path -> cached perceptual image hash
    pub image_hashes: HashMap<PathBuf, CachedImageEntry>,
    #[serde(skip)]
    custom_path: Option<PathBuf>,
}

impl ScanCache {
    /// Default location in the OS cache directory (~/AppData/Local/Dedupus/cache on Windows)
    pub fn default_cache_path(name: &str) -> PathBuf {
        let base_dir = dirs::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("Dedupus");
        base_dir.join(format!("{}.json", name))
    }

    /// Load cache from disk if it exists, or returns empty cache
    pub fn load_or_create(name: &str) -> Self {
        let path = Self::default_cache_path(name);
        Self::load_from_path(&path).unwrap_or_else(|_| Self {
            custom_path: Some(path),
            ..Default::default()
        })
    }

    pub fn load_from_path(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self {
                custom_path: Some(path.to_path_buf()),
                ..Default::default()
            });
        }

        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut cache: Self = serde_json::from_reader(reader)?;
        cache.custom_path = Some(path.to_path_buf());
        Ok(cache)
    }

    /// Save cache to disk
    pub fn save(&self) -> Result<()> {
        let path = self
            .custom_path
            .clone()
            .unwrap_or_else(|| Self::default_cache_path("scan_cache"));

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let file = File::create(&path)?;
        let writer = BufWriter::new(file);
        serde_json::to_writer(writer, self)?;
        Ok(())
    }

    pub fn get_file_hash(&self, path: &Path, size: u64, modified_secs: u64) -> Option<&str> {
        let entry = self.file_hashes.get(path)?;
        if entry.size == size && entry.modified_secs == modified_secs {
            Some(&entry.hash)
        } else {
            None
        }
    }

    pub fn insert_file_hash(&mut self, path: PathBuf, size: u64, modified_secs: u64, hash: String) {
        self.file_hashes.insert(
            path,
            CachedFileEntry {
                size,
                modified_secs,
                hash,
            },
        );
    }

    pub fn get_image_hash(
        &self,
        path: &Path,
        size: u64,
        modified_secs: u64,
    ) -> Option<(u32, u32, u64)> {
        let entry = self.image_hashes.get(path)?;
        if entry.size == size && entry.modified_secs == modified_secs {
            Some((entry.width, entry.height, entry.hash))
        } else {
            None
        }
    }

    pub fn insert_image_hash(
        &mut self,
        path: PathBuf,
        size: u64,
        modified_secs: u64,
        width: u32,
        height: u32,
        hash: u64,
    ) {
        self.image_hashes.insert(
            path,
            CachedImageEntry {
                size,
                modified_secs,
                width,
                height,
                hash,
            },
        );
    }

    /// Prune cached entries for files that no longer exist on disk.
    /// Returns the number of pruned entries.
    pub fn prune_missing(&mut self) -> usize {
        let initial_count = self.file_hashes.len() + self.image_hashes.len();
        self.file_hashes.retain(|p, _| p.exists());
        self.image_hashes.retain(|p, _| p.exists());
        initial_count - (self.file_hashes.len() + self.image_hashes.len())
    }

    /// Clear all cached hashes
    pub fn clear(&mut self) -> Result<()> {
        self.file_hashes.clear();
        self.image_hashes.clear();
        if let Some(path) = &self.custom_path {
            if path.exists() {
                fs::remove_file(path)?;
            }
        }
        Ok(())
    }

    /// Number of cached file hashes and image hashes
    pub fn count(&self) -> (usize, usize) {
        (self.file_hashes.len(), self.image_hashes.len())
    }
}
