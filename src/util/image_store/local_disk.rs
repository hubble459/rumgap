use std::path::PathBuf;

use tokio::fs;

use super::ImageStore;

/// Filesystem-backed [`ImageStore`], rooted at a configurable directory
/// (`IMAGE_STORAGE_PATH` env var, default `data/images`).
pub struct LocalDiskStore {
    root: PathBuf,
}

impl LocalDiskStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Build from the `IMAGE_STORAGE_PATH` env var (default `data/images`).
    pub fn from_env() -> Self {
        let root = std::env::var("IMAGE_STORAGE_PATH").unwrap_or_else(|_| "data/images".to_string());
        Self::new(root)
    }

    /// Resolve an opaque store key to an on-disk path.
    ///
    /// Keys are always constructed internally (`{chapter_id}/{page_index}.{ext}`)
    /// so `..` segments should never occur, but we guard against it anyway
    /// since this ultimately touches the filesystem.
    fn path_for(&self, key: &str) -> std::io::Result<PathBuf> {
        if key.split('/').any(|segment| segment == "..") {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid image store key",
            ));
        }
        Ok(self.root.join(key))
    }
}

#[async_trait::async_trait]
impl ImageStore for LocalDiskStore {
    async fn put(&self, key: &str, data: Vec<u8>) -> std::io::Result<()> {
        let path = self.path_for(key)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(path, data).await
    }

    async fn get(&self, key: &str) -> std::io::Result<Vec<u8>> {
        fs::read(self.path_for(key)?).await
    }

    async fn exists(&self, key: &str) -> bool {
        match self.path_for(key) {
            Ok(path) => fs::try_exists(path).await.unwrap_or(false),
            Err(_) => false,
        }
    }

    async fn delete(&self, key: &str) -> std::io::Result<()> {
        let path = self.path_for(key)?;
        match fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
}
