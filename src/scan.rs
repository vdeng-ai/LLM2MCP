//! Persistent repository scan continuations; each page resumes exact segments.
use crate::{
    config::AppConfig,
    workspace::{self, CollectedSource, ScanCoverage},
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Position {
    pub file: usize,
    pub part: usize,
    pub content_hash: Option<String>,
}
#[derive(Serialize, Deserialize)]
pub struct Checkpoint {
    version: u32,
    pub workspace: PathBuf,
    pub args: Value,
    pub snapshot: String,
    pub layout: String,
    pub position: Position,
    pub summary_keys: Vec<String>,
    pub coverage: ScanCoverage,
}
pub struct Page {
    pub chunks: Vec<CollectedSource>,
    pub coverage: ScanCoverage,
    pub snapshot: String,
    pub next: Option<Position>,
}

fn path(cursor: &str) -> Result<PathBuf> {
    if cursor.len() != 64 || !cursor.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("invalid scan_cursor");
    }
    Ok(crate::config::data_dir()?
        .join("repo-scan-cache")
        .join(format!("{cursor}.json")))
}
pub fn load(cursor: &str, root: &Path) -> Result<Checkpoint> {
    let path = path(cursor)?;
    if fs::metadata(&path)
        .context("scan cursor expired or missing; start a new scan")?
        .len()
        > 16 * 1024 * 1024
    {
        bail!("scan checkpoint exceeds storage limit");
    }
    let bytes = fs::read(path)?;
    if format!("{:x}", Sha256::digest(&bytes)) != cursor {
        bail!("scan checkpoint changed");
    }
    let checkpoint: Checkpoint = serde_json::from_slice(&bytes)?;
    if checkpoint.version != 1 || checkpoint.workspace != root.canonicalize()? {
        bail!("scan cursor belongs to a different workspace or version");
    }
    Ok(checkpoint)
}
pub fn save(checkpoint: &Checkpoint) -> Result<String> {
    let bytes = serde_json::to_vec(checkpoint)?;
    if bytes.len() > 16 * 1024 * 1024 {
        bail!("scan checkpoint exceeds storage limit");
    }
    let id = format!("{:x}", Sha256::digest(&bytes));
    let path = path(&id)?;
    fs::create_dir_all(path.parent().context("missing checkpoint directory")?)?;
    if !path.exists() {
        let count = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp = path.with_extension(format!("tmp.{}.{}", std::process::id(), count));
        fs::write(&temp, bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temp, fs::Permissions::from_mode(0o600))?;
        }
        match fs::rename(&temp, &path) {
            Ok(()) => {}
            Err(_) if path.exists() => {
                let _ = fs::remove_file(temp);
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(id)
}
pub fn checkpoint(
    root: &Path,
    args: Value,
    page: &Page,
    layout: String,
    keys: Vec<String>,
) -> Result<Option<String>> {
    page.next
        .as_ref()
        .map(|next| {
            save(&Checkpoint {
                version: 1,
                workspace: root.canonicalize()?,
                args,
                snapshot: page.snapshot.clone(),
                layout,
                position: next.clone(),
                summary_keys: keys,
                coverage: page.coverage.clone(),
            })
        })
        .transpose()
}

fn files_and_snapshot(root: &Path, args: &Value) -> Result<(Vec<PathBuf>, String)> {
    let strings = |name: &str| {
        args.get(name)
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let requested = strings("paths");
    let default_request = [".".to_owned()];
    let files = workspace::requested_files(
        root,
        if requested.is_empty() {
            &default_request
        } else {
            &requested
        },
        &strings("include"),
        &strings("exclude"),
    )?
    .into_iter()
    .collect::<Vec<_>>();
    let mut hash = Sha256::new();
    for file in &files {
        crate::control::Control::current().check()?;
        hash.update(workspace::normalized_relative(file.strip_prefix(root)?).as_bytes());
        hash.update([0]);
        let stamp = crate::repo_cache::file_stamp(file)?;
        hash.update(stamp.size.to_le_bytes());
        hash.update(stamp.modified_ns.to_le_bytes());
    }
    Ok((files, format!("{:x}", hash.finalize())))
}
pub fn validate_snapshot(root: &Path, args: &Value, expected: &str) -> Result<()> {
    if files_and_snapshot(root, args)?.1 != expected {
        bail!("repository changed since scan snapshot; start a new document_repo scan");
    }
    Ok(())
}
pub fn collect(
    root: &Path,
    args: &Value,
    config: &AppConfig,
    chunk_tokens: usize,
    previous: Option<&Checkpoint>,
) -> Result<Page> {
    let root = root.canonicalize()?;
    let (files, snapshot) = files_and_snapshot(&root, args)?;
    if let Some(previous) = previous
        && previous.snapshot != snapshot
    {
        bail!("repository changed since scan snapshot; start a new document_repo scan");
    }
    let max_chunks = args
        .get("max_chunks")
        .map(|value| value.as_u64().context("max_chunks must be an integer"))
        .transpose()?
        .unwrap_or(12);
    let max_chunks = usize::try_from(max_chunks).context("max_chunks exceeds supported range")?;
    if !(1..=12).contains(&max_chunks) {
        bail!("max_chunks must be between 1 and 12");
    }
    let limit = chunk_tokens.max(128).min(config.max_source_tokens.max(128));
    let segment_limit = config
        .max_file_tokens
        .min(limit.saturating_sub(128).max(16))
        .saturating_sub(128)
        .max(16);
    let manifest = workspace::truncate_tokens_strict(
        &workspace::manifest(&root)?,
        1024,
        "[MANIFEST TRUNCATED]",
    );
    let mut position = previous
        .map(|previous| previous.position.clone())
        .unwrap_or_default();
    if position.file > files.len() {
        bail!("invalid scan position");
    }
    let mut coverage = previous
        .map(|previous| previous.coverage.clone())
        .unwrap_or_default();
    coverage.total_files = files.len();
    let mut chunks = Vec::new();
    let mut body = String::new();
    let mut included = Vec::new();
    while position.file < files.len() {
        crate::control::Control::current().check()?;
        let file = &files[position.file];
        let relative = workspace::normalized_relative(file.strip_prefix(&root)?);
        let text = match workspace::read_text_tokens(file, usize::MAX) {
            Ok(text) => text,
            Err(error) => {
                coverage.omissions.push(format!("- {relative}: {error}"));
                position = Position {
                    file: position.file + 1,
                    ..Default::default()
                };
                continue;
            }
        };
        let hash = crate::result_pages::digest(&text);
        if position
            .content_hash
            .as_ref()
            .is_some_and(|expected| expected != &hash)
        {
            bail!("in-progress source file changed; start a new scan");
        }
        let segments = workspace::text_segments(&text, segment_limit);
        if position.part > segments.len() {
            bail!("invalid scan segment position");
        }
        while position.part < segments.len() {
            let (start, end, text) = &segments[position.part];
            let segment = format!(
                "\n===== FILE: {relative} LINES {start}-{end} PART {}/{} =====\n{text}\n===== END FILE =====\n",
                position.part + 1,
                segments.len()
            );
            if workspace::estimate_tokens(&segment) > limit {
                coverage.omissions.push(format!(
                    "- {relative}: segment {} exceeds chunk budget",
                    position.part + 1
                ));
                position.part += 1;
                continue;
            }
            if !body.is_empty()
                && workspace::estimate_tokens(&body) + workspace::estimate_tokens(&segment) > limit
            {
                chunks.push(CollectedSource {
                    manifest: manifest.clone(),
                    body: std::mem::take(&mut body),
                    included_files: std::mem::take(&mut included),
                    truncated: false,
                    evidence_cache_hits: 0,
                    evidence_cache_misses: 0,
                });
            }
            if chunks.len() >= max_chunks {
                position.content_hash = Some(hash);
                return Ok(Page {
                    chunks,
                    coverage,
                    snapshot,
                    next: Some(position),
                });
            }
            body.push_str(&segment);
            if !included.contains(&relative) {
                included.push(relative.clone());
            }
            coverage.segments += 1;
            position.part += 1;
        }
        if !coverage
            .omissions
            .iter()
            .any(|omission| omission.starts_with(&format!("- {relative}:")))
        {
            coverage.complete_files += 1;
        }
        position = Position {
            file: position.file + 1,
            ..Default::default()
        };
    }
    if !body.is_empty() {
        chunks.push(CollectedSource {
            manifest,
            body,
            included_files: included,
            truncated: false,
            evidence_cache_hits: 0,
            evidence_cache_misses: 0,
        });
    }
    Ok(Page {
        chunks,
        coverage,
        snapshot,
        next: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn continues_large_files_without_gaps_and_rejects_changes() {
        let dir = tempfile::tempdir().unwrap();
        let text = (0..400)
            .map(|index| format!("fn unique_{index}() {{ /* 中文🙂 */ }}\n"))
            .collect::<String>();
        fs::write(dir.path().join("large.rs"), &text).unwrap();
        let config = AppConfig {
            max_source_tokens: 512,
            max_file_tokens: 180,
            ..Default::default()
        };
        let args = json!({"max_chunks":1});
        let mut previous = None;
        let mut bodies = String::new();
        let mut pages = 0;
        loop {
            let page = collect(dir.path(), &args, &config, 512, previous.as_ref()).unwrap();
            assert!(page.chunks.len() <= 1);
            for chunk in &page.chunks {
                assert!(workspace::estimate_tokens(&chunk.body) <= 512);
                bodies.push_str(&chunk.body);
            }
            pages += 1;
            if page.next.is_none() {
                assert_eq!(page.coverage.complete_files, 1);
                break;
            }
            previous = Some(Checkpoint {
                version: 1,
                workspace: dir.path().canonicalize().unwrap(),
                args: args.clone(),
                snapshot: page.snapshot,
                layout: "test".into(),
                position: page.next.unwrap(),
                summary_keys: vec![],
                coverage: page.coverage,
            });
            assert!(pages < 200);
        }
        assert!(pages > 12);
        for index in 0..400 {
            assert_eq!(bodies.matches(&format!("fn unique_{index}() ")).count(), 1);
        }
        fs::write(dir.path().join("large.rs"), "changed").unwrap();
        assert!(collect(dir.path(), &args, &config, 512, previous.as_ref()).is_err());
    }
}
