use anyhow::{Context, Result, bail};
#[cfg(not(windows))]
use directories::BaseDirs;
#[cfg(windows)]
use directories::ProjectDirs;
use std::{
    fs,
    fs::File,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static INSTALL_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn stable_executable_path() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        let dirs = ProjectDirs::from("ai", "LLM2MCP", "LLM2MCP")
            .context("cannot resolve LLM2MCP data directory")?;
        return Ok(dirs.data_local_dir().join("bin").join("llm2mcp.exe"));
    }

    #[cfg(not(windows))]
    {
        let base = BaseDirs::new().context("cannot resolve home directory")?;
        Ok(base.home_dir().join(".local").join("bin").join("llm2mcp"))
    }
}

pub fn ensure_stable_install() -> Result<PathBuf> {
    let source = std::env::current_exe().context("cannot resolve current executable")?;
    let target = stable_executable_path()?;

    if same_file_path(&source, &target) {
        return Ok(target);
    }

    if !source.exists() {
        #[cfg(target_os = "linux")]
        {
            // A running development binary may have been atomically replaced by
            // cargo build. /proc/self/exe still provides the live executable.
            let proc_self = PathBuf::from("/proc/self/exe");
            if proc_self.exists() {
                return install_from(&proc_self, &target);
            }
        }
        bail!(
            "current LLM2MCP executable no longer exists at {}",
            source.display()
        );
    }

    install_from(&source, &target)
}

fn install_from(source: &Path, target: &Path) -> Result<PathBuf> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let counter = INSTALL_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = target.with_file_name(format!(
        ".{}.llm2mcp-install.{}.{}",
        target
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("llm2mcp"),
        std::process::id(),
        counter
    ));

    fs::copy(source, &temp)
        .with_context(|| format!("failed to copy {} to {}", source.display(), temp.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o755))?;
    }

    File::open(&temp)?.sync_all()?;

    #[cfg(windows)]
    {
        if target.exists() {
            let backup = target.with_extension("exe.llm2mcp-old");
            let _ = fs::remove_file(&backup);
            fs::rename(target, &backup).with_context(|| {
                format!(
                    "cannot replace running stable executable {}; close clients using it and retry",
                    target.display()
                )
            })?;
        }
        fs::rename(&temp, target)?;
    }

    #[cfg(not(windows))]
    {
        fs::rename(&temp, target)?;
    }

    Ok(target.to_path_buf())
}

fn same_file_path(source: &Path, target: &Path) -> bool {
    let source = source
        .canonicalize()
        .unwrap_or_else(|_| source.to_path_buf());
    let target = target
        .canonicalize()
        .unwrap_or_else(|_| target.to_path_buf());
    source == target
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
            "llm2mcp-install-{name}-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn stable_path_has_expected_binary_name() {
        let path = stable_executable_path().expect("stable path");
        #[cfg(windows)]
        assert_eq!(
            path.file_name().and_then(|v| v.to_str()),
            Some("llm2mcp.exe")
        );
        #[cfg(not(windows))]
        assert_eq!(path.file_name().and_then(|v| v.to_str()), Some("llm2mcp"));
    }

    #[test]
    fn install_from_copies_binary_to_requested_target() {
        let dir = test_dir("copy");
        fs::create_dir_all(&dir).expect("create temp dir");
        let source = std::env::current_exe().expect("current test executable");
        let target = dir.join(if cfg!(windows) {
            "llm2mcp.exe"
        } else {
            "llm2mcp"
        });

        let installed = install_from(&source, &target).expect("install binary");

        assert_eq!(installed, target);
        assert!(target.is_file());
        assert_eq!(
            fs::metadata(&target).expect("target metadata").len(),
            fs::metadata(source).expect("source metadata").len()
        );
        fs::remove_dir_all(dir).expect("cleanup temp dir");
    }
}
