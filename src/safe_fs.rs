use anyhow::{Context, Result};
use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn backup_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("config");
    path.with_file_name(format!("{name}.llm2mcp.bak"))
}

pub fn backup_existing(path: &Path) -> Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let backup = backup_path(path);
    fs::copy(path, &backup).with_context(|| {
        format!(
            "failed to back up {} to {}",
            path.display(),
            backup.display()
        )
    })?;
    Ok(Some(backup))
}

pub fn atomic_write_with_backup(path: &Path, data: &[u8]) -> Result<()> {
    if let Ok(existing) = fs::read(path)
        && existing == data
    {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    backup_existing(path)?;

    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = path.with_file_name(format!(
        ".{}.llm2mcp.tmp.{}.{}",
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("config"),
        std::process::id(),
        counter
    ));
    let existing_permissions = fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());
    let mut file = File::create(&temp)
        .with_context(|| format!("failed to create temporary file {}", temp.display()))?;
    file.write_all(data)?;
    file.sync_all()?;
    if let Some(permissions) = existing_permissions {
        fs::set_permissions(&temp, permissions).with_context(|| {
            format!(
                "failed to preserve permissions while replacing {}",
                path.display()
            )
        })?;
    }

    #[cfg(windows)]
    {
        if path.exists() {
            fs::remove_file(path)
                .with_context(|| format!("failed to replace {}", path.display()))?;
        }
        fs::rename(&temp, path)?;
    }
    #[cfg(not(windows))]
    {
        fs::rename(&temp, path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "llm2mcp-safe-fs-{name}-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn backup_name_is_predictable() {
        assert_eq!(
            backup_path(Path::new("/tmp/mcp.json")),
            PathBuf::from("/tmp/mcp.json.llm2mcp.bak")
        );
    }

    #[test]
    fn atomic_write_keeps_previous_config_as_backup() {
        let dir = test_dir("backup");
        fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("mcp.json");
        fs::write(&path, b"old-config").expect("seed config");

        atomic_write_with_backup(&path, b"new-config").expect("replace config");

        assert_eq!(fs::read(&path).expect("new config"), b"new-config");
        assert_eq!(
            fs::read(backup_path(&path)).expect("backup config"),
            b"old-config"
        );
        fs::remove_dir_all(dir).expect("cleanup temp dir");
    }

    #[test]
    fn unchanged_config_does_not_create_backup() {
        let dir = test_dir("unchanged");
        fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("mcp.json");
        fs::write(&path, b"same-config").expect("seed config");

        atomic_write_with_backup(&path, b"same-config").expect("no-op write");

        assert!(!backup_path(&path).exists());
        fs::remove_dir_all(dir).expect("cleanup temp dir");
    }

    #[cfg(unix)]
    #[test]
    fn replacement_preserves_existing_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = test_dir("permissions");
        fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("mcp.json");
        fs::write(&path, b"old-config").expect("seed config");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("set permissions");

        atomic_write_with_backup(&path, b"new-config").expect("replace config");

        assert_eq!(
            fs::metadata(&path).expect("metadata").permissions().mode() & 0o777,
            0o600
        );
        fs::remove_dir_all(dir).expect("cleanup temp dir");
    }
}
