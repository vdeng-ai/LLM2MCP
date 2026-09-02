use anyhow::{Context, Result, bail};
use ignore::WalkBuilder;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::config::AppConfig;

const SECRET_NAMES: &[&str] = &[
    ".env",
    ".env.local",
    ".env.production",
    ".env.development",
    "id_rsa",
    "id_ed25519",
    "credentials.json",
    "secrets.json",
];

const SECRET_SUFFIXES: &[&str] = &[".pem", ".key", ".p12", ".pfx"];

const TEXT_EXTENSIONS: &[&str] = &[
    "rs", "py", "go", "js", "jsx", "ts", "tsx", "vue", "svelte", "java", "kt", "kts", "c", "cc",
    "cpp", "cxx", "h", "hpp", "cs", "swift", "php", "rb", "sh", "bash", "zsh", "fish", "ps1",
    "sql", "html", "htm", "css", "scss", "less", "xml", "json", "json5", "yaml", "yml", "toml",
    "ini", "cfg", "conf", "md", "txt", "proto", "graphql",
];

const TEXT_NAMES: &[&str] = &[
    "Dockerfile",
    "Makefile",
    "CMakeLists.txt",
    "Cargo.toml",
    "Cargo.lock",
    "pyproject.toml",
    "package.json",
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "requirements.txt",
    "AGENTS.md",
    "README",
    "README.md",
    "README.zh-CN.md",
];

#[derive(Debug, Clone)]
pub struct CollectedSource {
    pub manifest: String,
    pub body: String,
    pub included_files: Vec<String>,
    pub truncated: bool,
}

fn canonical_workspace(workspace: &Path) -> Result<PathBuf> {
    let root = workspace
        .canonicalize()
        .with_context(|| format!("workspace does not exist: {}", workspace.display()))?;
    if !root.is_dir() {
        bail!("workspace is not a directory: {}", root.display());
    }
    Ok(root)
}

fn secure_path(root: &Path, candidate: &Path) -> Result<PathBuf> {
    let resolved = candidate
        .canonicalize()
        .with_context(|| format!("path does not exist: {}", candidate.display()))?;
    if !resolved.starts_with(root) {
        bail!("path escapes workspace: {}", resolved.display());
    }
    Ok(resolved)
}

fn is_secret(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return true;
    };
    let lower = name.to_ascii_lowercase();
    if SECRET_NAMES
        .iter()
        .any(|item| lower == item.to_ascii_lowercase())
    {
        return true;
    }
    if lower.starts_with(".env.")
        || lower.starts_with("credentials")
        || lower.starts_with("secrets")
    {
        return true;
    }
    SECRET_SUFFIXES.iter().any(|suffix| lower.ends_with(suffix))
}

fn looks_text(path: &Path) -> bool {
    if let Some(name) = path.file_name().and_then(|n| n.to_str())
        && TEXT_NAMES
            .iter()
            .any(|known| name.eq_ignore_ascii_case(known))
    {
        return true;
    }
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            TEXT_EXTENSIONS
                .iter()
                .any(|known| e.eq_ignore_ascii_case(known))
        })
        .unwrap_or(false)
}

fn read_text(path: &Path, max_chars: usize) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    if bytes.iter().take(8192).any(|b| *b == 0) {
        bail!("binary file skipped: {}", path.display());
    }
    let text = String::from_utf8_lossy(&bytes);
    if text.chars().count() <= max_chars {
        return Ok(text.into_owned());
    }
    let end = text
        .char_indices()
        .nth(max_chars)
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    let mut clipped = text[..end].to_owned();
    clipped.push_str("\n\n[FILE TRUNCATED]\n");
    Ok(clipped)
}

fn overview_files(root: &Path) -> Vec<PathBuf> {
    TEXT_NAMES
        .iter()
        .filter_map(|name| {
            let path = root.join(name);
            path.is_file().then_some(path)
        })
        .collect()
}

fn requested_files(root: &Path, requested: &[String]) -> Result<BTreeSet<PathBuf>> {
    let mut files = BTreeSet::new();
    if requested.is_empty() {
        for path in overview_files(root) {
            files.insert(path);
        }
        return Ok(files);
    }

    for item in requested {
        let target = secure_path(root, &root.join(item))?;
        if target.is_file() {
            if !is_secret(&target) && looks_text(&target) {
                files.insert(target);
            }
            continue;
        }
        for entry in WalkBuilder::new(&target)
            .hidden(false)
            .git_ignore(true)
            .git_exclude(true)
            .parents(true)
            .build()
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if path.is_file() && !is_secret(path) && looks_text(path) {
                files.insert(secure_path(root, path)?);
            }
        }
    }
    Ok(files)
}

fn file_chunk(root: &Path, file: &Path, max_file_chars: usize) -> Result<(String, String)> {
    let relative = file
        .strip_prefix(root)
        .unwrap_or(file)
        .to_string_lossy()
        .to_string();
    let text = read_text(file, max_file_chars)?;
    let chunk =
        format!("\n\n===== FILE: {relative} =====\n{text}\n===== END FILE: {relative} =====\n");
    Ok((relative, chunk))
}

pub fn manifest(workspace: &Path) -> Result<String> {
    let root = canonical_workspace(workspace)?;
    let output = Command::new("git")
        .args(["ls-files"])
        .current_dir(&root)
        .output();

    if let Ok(output) = output
        && output.status.success()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        let lines = text.lines().take(2000).collect::<Vec<_>>();
        if !lines.is_empty() {
            return Ok(lines.join("\n"));
        }
    }

    let mut files = Vec::new();
    for entry in WalkBuilder::new(&root)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .parents(true)
        .build()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if !path.is_file() || is_secret(path) {
            continue;
        }
        if let Ok(relative) = path.strip_prefix(&root) {
            files.push(relative.to_string_lossy().to_string());
            if files.len() >= 2000 {
                break;
            }
        }
    }
    Ok(files.join("\n"))
}

pub fn collect(
    workspace: &Path,
    requested: &[String],
    config: &AppConfig,
) -> Result<CollectedSource> {
    let root = canonical_workspace(workspace)?;
    let files = requested_files(&root, requested)?;
    let mut body = String::new();
    let mut included_files = Vec::new();
    let mut truncated = false;

    for file in files {
        let Ok((relative, chunk)) = file_chunk(&root, &file, config.max_file_chars) else {
            continue;
        };
        if body.chars().count() + chunk.chars().count() > config.max_source_chars {
            truncated = true;
            break;
        }
        body.push_str(&chunk);
        included_files.push(relative);
    }

    Ok(CollectedSource {
        manifest: manifest(&root)?,
        body,
        included_files,
        truncated,
    })
}

pub fn collect_chunks(
    workspace: &Path,
    requested: &[String],
    config: &AppConfig,
    chunk_chars: usize,
    max_chunks: usize,
) -> Result<Vec<CollectedSource>> {
    let root = canonical_workspace(workspace)?;
    let files = if requested.is_empty() {
        requested_files(&root, &[".".to_owned()])?
    } else {
        requested_files(&root, requested)?
    };
    let manifest = manifest(&root)?;
    let chunk_limit = chunk_chars
        .max(10_000)
        .min(config.max_source_chars.max(10_000));
    let mut chunks = Vec::new();
    let mut body = String::new();
    let mut included_files = Vec::new();
    let mut truncated = false;
    let max_file_chars = config.max_file_chars.min(chunk_limit.saturating_sub(1_000));

    for file in files {
        let Ok((relative, chunk)) = file_chunk(&root, &file, max_file_chars) else {
            continue;
        };
        let would_overflow =
            !body.is_empty() && body.chars().count() + chunk.chars().count() > chunk_limit;
        if would_overflow {
            chunks.push(CollectedSource {
                manifest: manifest.clone(),
                body: std::mem::take(&mut body),
                included_files: std::mem::take(&mut included_files),
                truncated: false,
            });
            if chunks.len() >= max_chunks {
                truncated = true;
                break;
            }
        }
        body.push_str(&chunk);
        included_files.push(relative);
    }

    if !body.is_empty() && chunks.len() < max_chunks {
        chunks.push(CollectedSource {
            manifest,
            body,
            included_files,
            truncated,
        });
    } else if truncated && let Some(last) = chunks.last_mut() {
        last.truncated = true;
    }

    Ok(chunks)
}

pub fn existing_document_paths(workspace: &Path) -> Result<Vec<String>> {
    let root = canonical_workspace(workspace)?;
    let mut docs = BTreeSet::new();

    for name in [
        "README.md",
        "README.zh-CN.md",
        "CONTRIBUTING.md",
        "CHANGELOG.md",
    ] {
        let path = root.join(name);
        if path.is_file() && !is_secret(&path) {
            docs.insert(name.to_owned());
        }
    }

    let docs_dir = root.join("docs");
    if docs_dir.is_dir() {
        for entry in WalkBuilder::new(&docs_dir)
            .hidden(false)
            .git_ignore(true)
            .git_exclude(true)
            .parents(true)
            .build()
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if !path.is_file() || is_secret(path) {
                continue;
            }
            let is_markdown = path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("md"));
            if is_markdown && let Ok(relative) = path.strip_prefix(&root) {
                docs.insert(relative.to_string_lossy().to_string());
            }
        }
    }

    Ok(docs.into_iter().collect())
}

fn validate_git_ref(value: &str) -> Result<()> {
    if value.is_empty()
        || value.starts_with('-')
        || value
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        bail!("invalid git ref");
    }
    Ok(())
}

fn truncate_text(mut text: String, max_chars: usize, marker: &str) -> (String, bool) {
    if text.chars().count() <= max_chars {
        return (text, false);
    }
    text = text.chars().take(max_chars).collect();
    text.push_str(marker);
    (text, true)
}

pub fn git_diff(
    workspace: &Path,
    base_ref: &str,
    max_chars: usize,
) -> Result<(String, String, bool)> {
    validate_git_ref(base_ref)?;
    let root = canonical_workspace(workspace)?;
    let status = Command::new("git")
        .args(["status", "--short"])
        .current_dir(&root)
        .output()
        .context("failed to run git status")?;
    let diff = Command::new("git")
        .args(["diff", base_ref, "--"])
        .current_dir(&root)
        .output()
        .context("failed to run git diff")?;
    if !diff.status.success() {
        bail!("git diff failed: {}", String::from_utf8_lossy(&diff.stderr));
    }

    let (text, truncated) = truncate_text(
        String::from_utf8_lossy(&diff.stdout).into_owned(),
        max_chars,
        "\n\n[DIFF TRUNCATED]\n",
    );
    Ok((
        String::from_utf8_lossy(&status.stdout).into_owned(),
        text,
        truncated,
    ))
}

pub fn git_diff_refs(
    workspace: &Path,
    base_ref: &str,
    target_ref: &str,
    max_chars: usize,
) -> Result<(String, bool)> {
    validate_git_ref(base_ref)?;
    validate_git_ref(target_ref)?;
    let root = canonical_workspace(workspace)?;
    let diff = Command::new("git")
        .args(["diff", base_ref, target_ref, "--"])
        .current_dir(&root)
        .output()
        .context("failed to run git diff between refs")?;
    if !diff.status.success() {
        bail!("git diff failed: {}", String::from_utf8_lossy(&diff.stderr));
    }
    Ok(truncate_text(
        String::from_utf8_lossy(&diff.stdout).into_owned(),
        max_chars,
        "\n\n[DIFF TRUNCATED]\n",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_detection_blocks_common_credentials() {
        assert!(is_secret(Path::new(".env")));
        assert!(is_secret(Path::new("foo.pem")));
        assert!(is_secret(Path::new("credentials-prod.json")));
        assert!(!is_secret(Path::new("src/main.rs")));
    }

    #[test]
    fn git_refs_reject_options_and_whitespace() {
        assert!(validate_git_ref("HEAD").is_ok());
        assert!(validate_git_ref("v0.1.0").is_ok());
        assert!(validate_git_ref("--output=/tmp/x").is_err());
        assert!(validate_git_ref("HEAD main").is_err());
    }
}
