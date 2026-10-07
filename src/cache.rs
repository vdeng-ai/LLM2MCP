use anyhow::{Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
const MAINTENANCE_INTERVAL_SECS: u64 = 300;

#[derive(Deserialize, Serialize)]
struct MaintenanceStamp {
    completed_at: u64,
    max_mib: u64,
    ttl_days: u64,
}

fn due(stamp: &MaintenanceStamp, now: u64, max_mib: u64, ttl_days: u64) -> bool {
    stamp.max_mib != max_mib
        || stamp.ttl_days != ttl_days
        || now < stamp.completed_at
        || now - stamp.completed_at >= MAINTENANCE_INTERVAL_SECS
}

fn maintenance_lock(dir: &Path) -> Result<File> {
    fs::create_dir_all(dir)?;
    Ok(OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join("cache-maintenance.lock"))?)
}

fn stamp(lock: &mut File, max_mib: u64, ttl_days: u64) -> Result<()> {
    let value = MaintenanceStamp {
        completed_at: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs(),
        max_mib,
        ttl_days,
    };
    lock.seek(SeekFrom::Start(0))?;
    lock.set_len(0)?;
    lock.write_all(&serde_json::to_vec(&value)?)?;
    Ok(())
}

/// Automatic pruning never queues behind another maintenance pass.
pub fn maintain_if_due(max_mib: u64, ttl_days: u64) -> Result<Option<CacheStats>> {
    let dir = crate::config::data_dir()?;
    let mut lock = maintenance_lock(&dir)?;
    match lock.try_lock_exclusive() {
        Ok(()) => {}
        Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
            return Ok(None);
        }
        Err(error) => return Err(error.into()),
    }
    let mut bytes = Vec::new();
    (&mut lock).take(1024).read_to_end(&mut bytes)?;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_secs();
    let previous = serde_json::from_slice::<MaintenanceStamp>(&bytes).ok();
    let stats = if previous.is_none_or(|previous| due(&previous, now, max_mib, ttl_days)) {
        let stats = maintain_locked(&dir, max_mib, ttl_days, false)?;
        stamp(&mut lock, max_mib, ttl_days)?;
        Some(stats)
    } else {
        None
    };
    FileExt::unlock(&lock)?;
    Ok(stats)
}
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
    let mut lock = maintenance_lock(&dir)?;
    lock.lock_exclusive()?;
    let stats = maintain_locked(&dir, max_mib, ttl_days, clear)?;
    stamp(&mut lock, max_mib, ttl_days)?;
    FileExt::unlock(&lock).context("failed to unlock cache maintenance")?;
    Ok(stats)
}

fn maintain_locked(dir: &Path, max_mib: u64, ttl_days: u64, clear: bool) -> Result<CacheStats> {
    let mut files = Vec::new();
    for root in [
        dir.join("repository-cache"),
        dir.join("repo-map-cache"),
        dir.join("repo-scan-cache"),
    ] {
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
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maintenance_is_throttled_but_policy_changes_and_clock_rollback_are_due() {
        let value = MaintenanceStamp {
            completed_at: 1000,
            max_mib: 128,
            ttl_days: 7,
        };
        assert!(!due(&value, 1299, 128, 7));
        assert!(due(&value, 1300, 128, 7));
        assert!(due(&value, 1001, 256, 7));
        assert!(due(&value, 1001, 128, 1));
        assert!(due(&value, 999, 128, 7));
    }
}
