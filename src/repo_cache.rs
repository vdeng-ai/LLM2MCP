use anyhow::{Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::UNIX_EPOCH,
};

use crate::workspace::SymbolCandidate;

pub const SYMBOL_INDEX_VERSION: &str = "repo-symbol-index-v3";
pub const EVIDENCE_CACHE_VERSION: &str = "repo-evidence-v2";
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileStamp {
    pub size: u64,
    pub modified_ns: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedFileIndex {
    pub stamp: FileStamp,
    pub content_hash: String,
    pub total_lines: usize,
    pub symbols: Vec<SymbolCandidate>,
    #[serde(default)]
    pub imports: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositorySymbolIndex {
    pub version: String,
    pub workspace: String,
    pub files: BTreeMap<String, CachedFileIndex>,
}

impl RepositorySymbolIndex {
    fn empty(workspace: &Path) -> Self {
        Self {
            version: SYMBOL_INDEX_VERSION.to_owned(),
            workspace: workspace.to_string_lossy().into_owned(),
            files: BTreeMap::new(),
        }
    }
}

pub fn file_stamp(path: &Path) -> Result<FileStamp> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to stat repository file {}", path.display()))?;
    let modified_ns = metadata
        .modified()
        .unwrap_or(UNIX_EPOCH)
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .try_into()
        .unwrap_or(u64::MAX);
    Ok(FileStamp {
        size: metadata.len(),
        modified_ns,
    })
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub fn load_symbol_index(workspace: &Path) -> Result<RepositorySymbolIndex> {
    let root = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    let path = symbol_index_path(&root)?;
    if !path.exists() {
        return Ok(RepositorySymbolIndex::empty(&root));
    }
    let text = fs::read_to_string(&path)
        .with_context(|| format!("failed to read symbol index {}", path.display()))?;
    let index: RepositorySymbolIndex = serde_json::from_str(&text)
        .with_context(|| format!("invalid symbol index {}", path.display()))?;
    if index.version != SYMBOL_INDEX_VERSION || index.workspace != root.to_string_lossy() {
        return Ok(RepositorySymbolIndex::empty(&root));
    }
    Ok(index)
}

pub fn merge_symbol_updates(
    workspace: &Path,
    updates: BTreeMap<String, CachedFileIndex>,
) -> Result<()> {
    if updates.is_empty() {
        return Ok(());
    }
    let root = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    let path = symbol_index_path(&root)?;
    let lock = cache_lock(&path)?;
    lock.lock_exclusive()?;

    let result = {
        let mut index =
            load_symbol_index(&root).unwrap_or_else(|_| RepositorySymbolIndex::empty(&root));
        for (path, entry) in updates {
            index.files.insert(path, entry);
        }
        atomic_write_json(&path, &index)
    };

    FileExt::unlock(&lock)?;
    result
}

pub fn evidence_key(relative: &str, content_hash: &str, segment: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(EVIDENCE_CACHE_VERSION.as_bytes());
    hasher.update([0]);
    hasher.update(relative.as_bytes());
    hasher.update([0]);
    hasher.update(content_hash.as_bytes());
    hasher.update([0]);
    hasher.update(segment.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn load_evidence(workspace: &Path, key: &str) -> Result<Option<String>> {
    let path = evidence_path(workspace, key)?;
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path)
        .with_context(|| format!("failed to read evidence cache {}", path.display()))?;
    if text.is_empty() {
        return Ok(None);
    }
    Ok(Some(text))
}

pub fn store_evidence(workspace: &Path, key: &str, text: &str) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    let path = evidence_path(workspace, key)?;
    if path.exists() {
        return Ok(());
    }
    let lock = cache_lock(&path)?;
    lock.lock_exclusive()?;
    let result = if path.exists() {
        Ok(())
    } else {
        atomic_write_bytes(&path, text.as_bytes())
    };
    FileExt::unlock(&lock)?;
    result
}

fn workspace_key(workspace: &Path) -> String {
    let root = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    hash_bytes(root.to_string_lossy().as_bytes())
}

fn cache_root() -> Result<PathBuf> {
    let data_dir = crate::config::data_dir()?;
    let root = data_dir.join("repository-cache");
    fs::create_dir_all(&root)?;
    secure_directory(&root)?;
    Ok(root)
}

fn symbol_index_path(workspace: &Path) -> Result<PathBuf> {
    let dir = cache_root()?.join(SYMBOL_INDEX_VERSION);
    fs::create_dir_all(&dir)?;
    secure_directory(&dir)?;
    Ok(dir.join(format!("{}.json", workspace_key(workspace))))
}

fn evidence_path(workspace: &Path, key: &str) -> Result<PathBuf> {
    let version_dir = cache_root()?.join(EVIDENCE_CACHE_VERSION);
    fs::create_dir_all(&version_dir)?;
    secure_directory(&version_dir)?;
    let workspace_dir = version_dir.join(workspace_key(workspace));
    fs::create_dir_all(&workspace_dir)?;
    secure_directory(&workspace_dir)?;
    Ok(workspace_dir.join(format!("{key}.txt")))
}

#[cfg(test)]
pub fn clear_workspace_cache(workspace: &Path) -> Result<()> {
    let root = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    let index = symbol_index_path(&root)?;
    let _ = fs::remove_file(&index);
    let _ = fs::remove_file(index.with_extension("json.lock"));
    let evidence_dir = cache_root()?
        .join(EVIDENCE_CACHE_VERSION)
        .join(workspace_key(&root));
    let _ = fs::remove_dir_all(evidence_dir);
    Ok(())
}

fn cache_lock(path: &Path) -> Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        secure_directory(parent)?;
    }
    let lock_path = path.with_extension(format!(
        "{}.lock",
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("cache")
    ));
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .context("failed to open repository cache lock")?;
    secure_file(&lock_path)?;
    Ok(lock)
}

fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    atomic_write_bytes(path, &serde_json::to_vec_pretty(value)?)
}

fn atomic_write_bytes(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        secure_directory(parent)?;
    }
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = path.with_extension(format!("tmp.{}.{}", std::process::id(), counter));
    fs::write(&temp, data)?;
    secure_file(&temp)?;
    #[cfg(windows)]
    {
        if path.exists() {
            fs::remove_file(path)?;
        }
        fs::rename(&temp, path)?;
    }
    #[cfg(not(windows))]
    {
        fs::rename(&temp, path)?;
    }
    Ok(())
}

#[cfg(unix)]
fn secure_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn secure_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn secure_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn secure_file(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_key_changes_with_content_and_range() {
        let a = evidence_key("src/main.rs", "hash-a", "symbol:1:10:ctx4");
        let b = evidence_key("src/main.rs", "hash-b", "symbol:1:10:ctx4");
        let c = evidence_key("src/main.rs", "hash-a", "symbol:2:10:ctx4");
        assert_ne!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn file_stamp_tracks_size_and_mtime() {
        let path = std::env::current_exe().expect("current test executable");
        let stamp = file_stamp(&path).expect("file stamp");
        assert!(stamp.size > 0);
        assert!(stamp.modified_ns > 0);
    }
}
