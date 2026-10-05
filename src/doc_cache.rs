use anyhow::{Context, Result};
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::{
    fs,
    fs::OpenOptions,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{config::AppConfig, workspace::CollectedSource};

/// Bump this whenever the repository-map prompt semantics change.
pub const MAP_CACHE_VERSION: &str = "repo-map-v3";
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn map_key(config: &AppConfig, map_tokens: u32, chunk: &CollectedSource) -> String {
    let mut hasher = Sha256::new();
    hasher.update(MAP_CACHE_VERSION.as_bytes());
    hasher.update([0]);
    hasher.update(config.base_url.as_bytes());
    hasher.update([0]);
    hasher.update(config.model.as_bytes());
    hasher.update([0]);
    hasher.update(config.system_prompt_prefix.trim().as_bytes());
    hasher.update([0]);
    hasher.update(crate::mcp::DOC_MAP_SYSTEM.as_bytes());
    hasher.update([0]);
    hasher.update(format!("{:?}", config.reasoning_transport).as_bytes());
    hasher.update([0]);
    hasher.update(config.temperature.to_le_bytes());
    hasher.update([u8::from(config.send_temperature)]);
    hasher.update(config.completion_token_parameter.as_bytes());
    hasher.update(config.model_output_limit.to_le_bytes());
    hasher.update(map_tokens.to_le_bytes());
    for path in &chunk.included_files {
        hasher.update(path.as_bytes());
        hasher.update([0]);
    }
    hasher.update(chunk.body.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn load_summary(key: &str) -> Result<Option<String>> {
    let path = summary_path(key)?;
    if !path.exists() {
        return Ok(None);
    }
    let summary = fs::read_to_string(&path)
        .with_context(|| format!("failed to read map cache {}", path.display()))?;
    if summary.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(summary))
}

pub fn store_summary(key: &str, summary: &str) -> Result<()> {
    if summary.trim().is_empty() {
        return Ok(());
    }
    let path = summary_path(key)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let lock_path = path.with_extension("lock");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)?;
    lock.lock_exclusive()?;

    let result = (|| {
        if path.exists() {
            return Ok(());
        }
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp = path.with_extension(format!("tmp.{}.{}", std::process::id(), counter));
        fs::write(&temp, summary.as_bytes())?;
        #[cfg(windows)]
        {
            if !path.exists() {
                fs::rename(&temp, &path)?;
            } else {
                let _ = fs::remove_file(&temp);
            }
        }
        #[cfg(not(windows))]
        {
            fs::rename(&temp, &path)?;
        }
        Ok(())
    })();

    FileExt::unlock(&lock)?;
    result
}

fn summary_path(key: &str) -> Result<PathBuf> {
    let data_dir = crate::config::data_dir()?;
    Ok(data_dir
        .join("repo-map-cache")
        .join(MAP_CACHE_VERSION)
        .join(format!("{key}.md")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    fn chunk(body: &str) -> CollectedSource {
        CollectedSource {
            manifest: String::new(),
            body: body.to_owned(),
            included_files: vec!["src/main.rs".to_owned()],
            truncated: false,
            evidence_cache_hits: 0,
            evidence_cache_misses: 0,
        }
    }

    #[test]
    fn map_key_changes_with_content_and_budget() {
        let config = AppConfig::default();
        let a = map_key(&config, 1_000, &chunk("fn main() {}"));
        let b = map_key(&config, 1_000, &chunk("fn main() { println!(\"x\"); }"));
        let c = map_key(&config, 1_200, &chunk("fn main() {}"));
        assert_ne!(a, b);
        assert_ne!(a, c);
        let mut changed = config.clone();
        changed.system_prompt_prefix = "Team conventions".to_owned();
        assert_ne!(a, map_key(&changed, 1_000, &chunk("fn main() {}")));
        changed = config.clone();
        changed.send_temperature = false;
        assert_ne!(a, map_key(&changed, 1_000, &chunk("fn main() {}")));
    }
}
