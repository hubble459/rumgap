pub mod local_disk;

/// Storage abstraction for downloaded chapter page images.
///
/// Keyed by an **opaque string key** -- never a filesystem path -- so a
/// future `S3Store` (or any other backend) can drop in without touching any
/// caller. The key convention used throughout the codebase is
/// `{chapter_id}/{page_index}.{ext}`, but callers of this trait should treat
/// the key as an opaque identifier.
#[async_trait::async_trait]
pub trait ImageStore: Send + Sync {
    /// Store `data` under `key`, overwriting anything already there.
    async fn put(&self, key: &str, data: Vec<u8>) -> std::io::Result<()>;

    /// Retrieve the bytes stored under `key`.
    async fn get(&self, key: &str) -> std::io::Result<Vec<u8>>;

    /// Whether something is currently stored under `key`.
    async fn exists(&self, key: &str) -> bool;

    /// Remove whatever is stored under `key`, if anything.
    async fn delete(&self, key: &str) -> std::io::Result<()>;
}
