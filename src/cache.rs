use anyhow::{Context, Result};
use fs2::FileExt;
use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
#[derive(Default, Debug, Serialize)]
pub struct CacheStats {
    pub files: usize,
    pub bytes: u64,
    pub removed: usize,
}
fn entries(root: &Path, found: &mut Vec<(PathBuf, u64, SystemTime)>) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            entries(&entry.path(), found)?;
        } else if kind.is_file()
            && matches!(
                entry.path().extension().and_then(|value| value.to_str()),
                Some("json" | "md" | "txt")
            )
        {
            let info = entry.metadata()?;
            found.push((
                entry.path(),
                info.len(),
                info.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            ));
        }
    }
    Ok(())
}
pub fn maintain(max_mib: u64, ttl_days: u64, clear: bool) -> Result<CacheStats> {
    let dir = crate::config::data_dir()?;
    fs::create_dir_all(&dir)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join("cache-maintenance.lock"))?;
    lock.lock_exclusive()?;
    let mut files = Vec::new();
    for root in [dir.join("repository-cache"), dir.join("repo-map-cache")] {
        entries(&root, &mut files)?;
    }
    files.sort_by_key(|entry| entry.2);
    let mut stats = CacheStats {
        files: files.len(),
        bytes: files.iter().map(|entry| entry.1).sum(),
        removed: 0,
    };
    let now = SystemTime::now();
    let limit = max_mib.clamp(16, 16384) * 1024 * 1024;
    for (path, size, modified) in files {
        if (clear
            || now.duration_since(modified).unwrap_or_default()
                > Duration::from_secs(ttl_days.clamp(1, 365) * 86400)
            || stats.bytes > limit)
            && fs::remove_file(&path).is_ok()
        {
            stats.bytes = stats.bytes.saturating_sub(size);
            stats.files -= 1;
            stats.removed += 1;
        }
    }
    FileExt::unlock(&lock).context("failed to unlock cache maintenance")?;
    Ok(stats)
}
