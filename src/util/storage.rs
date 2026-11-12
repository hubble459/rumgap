//! How much space rumgap takes up, for the admin storage page: the image folder on disk (walked
//! file by file), the database, the volume both live on, and which sites and manga the stored
//! pages come from.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use chrono::Utc;
use sea_orm::{ConnectionTrait, DatabaseConnection, QueryResult, Statement};
use tonic::Status;

use crate::data::manga::source_cover_url;
use crate::proto::{StorageBucket, StorageGroup, StorageReply};

const TOP_TABLES: i64 = 8;
const TOP_MANGA: i64 = 10;

fn internal<E: ToString>(e: E) -> Status {
    Status::internal(e.to_string())
}

async fn query(db: &DatabaseConnection, sql: &str) -> Result<Vec<QueryResult>, Status> {
    db.query_all_raw(Statement::from_string(db.get_database_backend(), sql))
        .await
        .map_err(internal)
}

/// Same root as `LocalDiskStore::from_env`.
fn image_root() -> PathBuf {
    std::env::var("IMAGE_STORAGE_PATH")
        .unwrap_or_else(|_| "data/images".to_string())
        .into()
}

/// What a file under the image root is, from its path relative to the root:
/// `covers/...`, `{chapter}/{page}.{ext}`, or a resized copy `{chapter}/{page}.{ext}.v{dim}q{quality}.jpg`.
pub fn classify(relative: &Path) -> &'static str {
    let mut components = relative.components();
    let first = components.next().and_then(|c| c.as_os_str().to_str()).unwrap_or_default();
    if first == "covers" {
        return "covers";
    }
    let name = relative.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let parts: Vec<&str> = name.split('.').collect();
    let in_chapter_dir = first.parse::<u64>().is_ok() && relative.components().count() == 2;
    if !in_chapter_dir {
        return "other";
    }
    match parts.as_slice() {
        [page, _ext] if page.parse::<u64>().is_ok() => "pages",
        [page, _ext, variant, "jpg"] if page.parse::<u64>().is_ok() && variant.starts_with('v') => "resized",
        _ => "other",
    }
}

/// Bytes and file count per kind of file under `root`.
fn walk(root: &Path) -> HashMap<&'static str, (i64, i64)> {
    let mut totals: HashMap<&'static str, (i64, i64)> = HashMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else { continue };
            let path = entry.path();
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() {
                let size = entry.metadata().map(|m| m.len() as i64).unwrap_or_default();
                let kind = path.strip_prefix(root).map(classify).unwrap_or("other");
                let total = totals.entry(kind).or_default();
                total.0 += size;
                total.1 += 1;
            }
        }
    }
    totals
}

/// (total, free) bytes of the volume holding `path`.
#[cfg(unix)]
fn volume(path: &Path) -> Option<(i64, i64)> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c_path` is a valid NUL-terminated string and `stats` a writable statvfs.
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut stats) } != 0 {
        return None;
    }
    let block = stats.f_frsize as i64;
    Some((stats.f_blocks as i64 * block, stats.f_bavail as i64 * block))
}

#[cfg(not(unix))]
fn volume(_path: &Path) -> Option<(i64, i64)> {
    None
}

fn group(row: &QueryResult) -> Result<StorageGroup, sea_orm::DbErr> {
    let cover_source_id: Option<i32> = row.try_get("", "cover_source_id").ok().flatten();
    Ok(StorageGroup {
        key: row.try_get("", "key")?,
        title: row.try_get("", "title")?,
        cover: cover_source_id.map(source_cover_url),
        bytes: row.try_get("", "bytes")?,
        pages: row.try_get("", "pages")?,
        chapters: row.try_get("", "chapters")?,
    })
}

pub async fn measure(db: &DatabaseConnection) -> Result<StorageReply, Status> {
    let started = Instant::now();
    let root = image_root();

    let walk_root = root.clone();
    let disk = tokio::task::spawn_blocking(move || walk(&walk_root)).await.map_err(internal)?;
    let mut buckets: Vec<StorageBucket> = ["pages", "resized", "covers", "other"]
        .into_iter()
        .map(|name| {
            let (bytes, files) = disk.get(name).copied().unwrap_or_default();
            StorageBucket {
                name: name.to_string(),
                bytes,
                files,
            }
        })
        .collect();
    buckets.retain(|bucket| bucket.files > 0 || bucket.name != "other");

    let database_bytes: i64 = query(db, "SELECT pg_database_size(current_database()) AS bytes")
        .await?
        .first()
        .map(|row| row.try_get("", "bytes"))
        .transpose()
        .map_err(internal)?
        .unwrap_or_default();

    let tables = query(
        db,
        &format!(
            r#"
            SELECT c.relname AS name, pg_total_relation_size(c.oid) AS bytes,
                   GREATEST(c.reltuples, 0)::int8 AS files
            FROM pg_class c
            JOIN pg_namespace n ON n.oid = c.relnamespace
            WHERE c.relkind = 'r' AND n.nspname = 'public'
            ORDER BY bytes DESC
            LIMIT {TOP_TABLES}
            "#
        ),
    )
    .await?
    .iter()
    .map(|row| {
        Ok(StorageBucket {
            name: row.try_get("", "name")?,
            bytes: row.try_get("", "bytes")?,
            files: row.try_get("", "files")?,
        })
    })
    .collect::<Result<Vec<_>, sea_orm::DbErr>>()
    .map_err(internal)?;

    let hosts = query(
        db,
        r#"
        SELECT s.hostname AS key, s.hostname AS title,
               COALESCE(SUM(ci.byte_size), 0)::int8 AS bytes,
               COUNT(*) AS pages, COUNT(DISTINCT ci.chapter_id) AS chapters
        FROM chapter_image ci
        JOIN chapter c ON c.id = ci.chapter_id
        JOIN manga_source s ON s.id = c.manga_source_id
        WHERE ci.status = 'done'
        GROUP BY s.hostname
        ORDER BY bytes DESC
        "#,
    )
    .await?
    .iter()
    .map(group)
    .collect::<Result<Vec<_>, _>>()
    .map_err(internal)?;

    let manga = query(
        db,
        &format!(
            r#"
            SELECT s.manga_id::text AS key,
                   COALESCE(p.title, '') AS title,
                   CASE WHEN p.cover_source_url IS NOT NULL THEN p.id END AS cover_source_id,
                   COALESCE(SUM(ci.byte_size), 0)::int8 AS bytes,
                   COUNT(*) AS pages, COUNT(DISTINCT ci.chapter_id) AS chapters
            FROM chapter_image ci
            JOIN chapter c ON c.id = ci.chapter_id
            JOIN manga_source s ON s.id = c.manga_source_id
            LEFT JOIN manga_source p ON p.manga_id = s.manga_id AND p.is_primary
            WHERE ci.status = 'done'
            GROUP BY s.manga_id, p.id, p.title, p.cover_source_url
            ORDER BY bytes DESC
            LIMIT {TOP_MANGA}
            "#
        ),
    )
    .await?
    .iter()
    .map(group)
    .collect::<Result<Vec<_>, _>>()
    .map_err(internal)?;

    let mut statuses: HashMap<String, i64> = HashMap::new();
    for row in query(db, "SELECT status, COUNT(*) AS count FROM chapter_image GROUP BY status").await? {
        statuses.insert(
            row.try_get("", "status").map_err(internal)?,
            row.try_get("", "count").map_err(internal)?,
        );
    }

    let volume = volume(&root);
    Ok(StorageReply {
        disk_bytes: buckets.iter().map(|b| b.bytes).sum(),
        disk_files: buckets.iter().map(|b| b.files).sum(),
        disk: buckets,
        database_bytes,
        tables,
        volume_total_bytes: volume.map(|(total, _)| total),
        volume_free_bytes: volume.map(|(_, free)| free),
        hosts,
        manga,
        pages_stored: statuses.get("done").copied().unwrap_or_default(),
        pages_pending: ["pending", "downloading"]
            .iter()
            .filter_map(|status| statuses.get(*status))
            .sum(),
        pages_failed: statuses.get("failed").copied().unwrap_or_default(),
        measured_at: Utc::now().timestamp_millis(),
        scan_millis: started.elapsed().as_millis() as i64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_files_by_their_key() {
        assert_eq!(classify(Path::new("123/4.jpg")), "pages");
        assert_eq!(classify(Path::new("123/4.webp")), "pages");
        assert_eq!(classify(Path::new("123/4.jpg.v1200q80.jpg")), "resized");
        assert_eq!(classify(Path::new("covers/source/9.png")), "covers");
        assert_eq!(classify(Path::new("123/notes.txt")), "other");
        assert_eq!(classify(Path::new("stray.jpg")), "other");
    }

    #[test]
    fn walk_sums_each_kind() {
        let root = std::env::temp_dir().join(format!("rumgap-storage-test-{}", std::process::id()));
        std::fs::create_dir_all(root.join("7")).unwrap();
        std::fs::create_dir_all(root.join("covers/source")).unwrap();
        std::fs::write(root.join("7/0.jpg"), [0u8; 10]).unwrap();
        std::fs::write(root.join("7/1.png"), [0u8; 20]).unwrap();
        std::fs::write(root.join("7/1.png.v800q70.jpg"), [0u8; 5]).unwrap();
        std::fs::write(root.join("covers/source/3.jpg"), [0u8; 7]).unwrap();

        let totals = walk(&root);
        std::fs::remove_dir_all(&root).unwrap();

        assert_eq!(totals.get("pages"), Some(&(30, 2)));
        assert_eq!(totals.get("resized"), Some(&(5, 1)));
        assert_eq!(totals.get("covers"), Some(&(7, 1)));
        assert_eq!(totals.get("other"), None);
    }
}
