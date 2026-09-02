use anyhow::{Context, Result, bail};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::{config::AppConfig, repo_cache};

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
    pub evidence_cache_hits: usize,
    pub evidence_cache_misses: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SymbolCandidate {
    pub path: String,
    pub label: String,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone)]
pub struct DiscoveryIndex {
    pub body: String,
    pub candidate_files: Vec<String>,
    pub symbols: Vec<SymbolCandidate>,
    pub truncated: bool,
    pub cache_hits: usize,
    pub cache_misses: usize,
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

pub fn estimate_tokens(text: &str) -> usize {
    let mut tokens = 0usize;
    let mut ascii_run = 0usize;
    let mut space_run = 0usize;
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            space_run = 0;
            ascii_run += 1;
            if ascii_run % 4 == 1 {
                tokens += 1;
            }
            continue;
        }
        ascii_run = 0;
        if ch == ' ' || ch == '\t' {
            space_run += 1;
            if space_run % 4 == 1 {
                tokens += 1;
            }
            continue;
        }
        space_run = 0;
        if ch == '\n' || ch == '\r' {
            tokens += 1;
            continue;
        }
        tokens += 1;
    }
    tokens
}

fn truncate_to_token_budget(text: &str, max_tokens: usize, marker: &str) -> (String, bool) {
    if estimate_tokens(text) <= max_tokens {
        return (text.to_owned(), false);
    }

    let mut output = String::new();
    let mut used = 0usize;
    let mut ascii_run = 0usize;
    let mut space_run = 0usize;
    for ch in text.chars() {
        let cost = if ch.is_ascii_alphanumeric() || ch == '_' {
            space_run = 0;
            ascii_run += 1;
            usize::from(ascii_run % 4 == 1)
        } else if ch == ' ' || ch == '\t' {
            ascii_run = 0;
            space_run += 1;
            usize::from(space_run % 4 == 1)
        } else {
            ascii_run = 0;
            space_run = 0;
            1
        };
        if used.saturating_add(cost) > max_tokens {
            break;
        }
        used += cost;
        output.push(ch);
    }
    output.push_str(marker);
    (output, true)
}

pub fn truncate_tokens(text: &str, max_tokens: usize, marker: &str) -> (String, bool) {
    truncate_to_token_budget(text, max_tokens, marker)
}

pub fn truncate_tokens_strict(text: &str, max_tokens: usize, marker: &str) -> String {
    if estimate_tokens(text) <= max_tokens {
        return text.to_owned();
    }
    let marker_tokens = estimate_tokens(marker);
    if marker_tokens >= max_tokens {
        return truncate_to_token_budget(marker, max_tokens, "").0;
    }

    let mut content_budget = max_tokens - marker_tokens;
    loop {
        let mut output = truncate_to_token_budget(text, content_budget, "").0;
        output.push_str(marker);
        let measured = estimate_tokens(&output);
        if measured <= max_tokens || content_budget == 0 {
            return output;
        }
        content_budget = content_budget.saturating_sub(measured - max_tokens + 1);
    }
}

fn read_text_tokens(path: &Path, max_tokens: usize) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    if bytes.iter().take(8192).any(|b| *b == 0) {
        bail!("binary file skipped: {}", path.display());
    }
    let text = String::from_utf8_lossy(&bytes);
    Ok(truncate_to_token_budget(&text, max_tokens, "\n\n[FILE TRUNCATED]\n").0)
}

fn normalized_relative(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_owned()
}

fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern = pattern.replace('\\', "/");
    let pattern = pattern.trim_start_matches("./").as_bytes();
    let text = text.as_bytes();
    let mut memo = std::collections::HashMap::<(usize, usize), bool>::new();

    fn matches(
        pattern: &[u8],
        text: &[u8],
        pi: usize,
        ti: usize,
        memo: &mut std::collections::HashMap<(usize, usize), bool>,
    ) -> bool {
        if let Some(value) = memo.get(&(pi, ti)) {
            return *value;
        }
        let result = if pi == pattern.len() {
            ti == text.len()
        } else if pattern[pi] == b'*' && pi + 1 < pattern.len() && pattern[pi + 1] == b'*' {
            let mut next = pi + 2;
            while next < pattern.len() && pattern[next] == b'*' {
                next += 1;
            }
            if next < pattern.len() && pattern[next] == b'/' {
                matches(pattern, text, next + 1, ti, memo)
                    || (ti < text.len() && matches(pattern, text, pi, ti + 1, memo))
            } else {
                matches(pattern, text, next, ti, memo)
                    || (ti < text.len() && matches(pattern, text, pi, ti + 1, memo))
            }
        } else if pattern[pi] == b'*' {
            matches(pattern, text, pi + 1, ti, memo)
                || (ti < text.len() && text[ti] != b'/' && matches(pattern, text, pi, ti + 1, memo))
        } else if pattern[pi] == b'?' {
            ti < text.len() && text[ti] != b'/' && matches(pattern, text, pi + 1, ti + 1, memo)
        } else {
            ti < text.len()
                && pattern[pi] == text[ti]
                && matches(pattern, text, pi + 1, ti + 1, memo)
        };
        memo.insert((pi, ti), result);
        result
    }

    matches(pattern, text, 0, 0, &mut memo)
}

fn matches_pattern(pattern: &str, relative: &str) -> bool {
    glob_match(pattern, relative)
        || (!pattern.contains('/')
            && relative
                .rsplit('/')
                .next()
                .is_some_and(|name| glob_match(pattern, name)))
}

fn matches_filters(relative: &str, include: &[String], exclude: &[String]) -> bool {
    let included = include.is_empty()
        || include
            .iter()
            .any(|pattern| matches_pattern(pattern, relative));
    let excluded = exclude
        .iter()
        .any(|pattern| matches_pattern(pattern, relative));
    included && !excluded
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

fn requested_files(
    root: &Path,
    requested: &[String],
    include: &[String],
    exclude: &[String],
) -> Result<BTreeSet<PathBuf>> {
    let mut files = BTreeSet::new();
    if requested.is_empty() {
        for path in overview_files(root) {
            let relative = normalized_relative(path.strip_prefix(root).unwrap_or(&path));
            if matches_filters(&relative, include, exclude) {
                files.insert(path);
            }
        }
        return Ok(files);
    }

    for item in requested {
        let target = secure_path(root, &root.join(item))?;
        if target.is_file() {
            let relative = normalized_relative(target.strip_prefix(root).unwrap_or(&target));
            if !is_secret(&target)
                && looks_text(&target)
                && matches_filters(&relative, include, exclude)
            {
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
                let resolved = secure_path(root, path)?;
                let relative =
                    normalized_relative(resolved.strip_prefix(root).unwrap_or(&resolved));
                if matches_filters(&relative, include, exclude) {
                    files.insert(resolved);
                }
            }
        }
    }
    Ok(files)
}

fn file_chunk(root: &Path, file: &Path, max_file_tokens: usize) -> Result<(String, String)> {
    let relative = normalized_relative(file.strip_prefix(root).unwrap_or(file));
    let text = read_text_tokens(file, max_file_tokens)?;
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
    collect_filtered(workspace, requested, config, &[], &[])
}

pub fn collect_filtered(
    workspace: &Path,
    requested: &[String],
    config: &AppConfig,
    include: &[String],
    exclude: &[String],
) -> Result<CollectedSource> {
    let root = canonical_workspace(workspace)?;
    let files = requested_files(&root, requested, include, exclude)?;
    let mut body = String::new();
    let mut body_tokens = 0usize;
    let mut included_files = Vec::new();
    let mut truncated = false;

    let per_file_budget = config
        .max_file_tokens
        .min(config.max_source_tokens.saturating_sub(250).max(500));
    for file in files {
        let Ok((relative, chunk)) = file_chunk(&root, &file, per_file_budget) else {
            continue;
        };
        let chunk_tokens = estimate_tokens(&chunk);
        if body_tokens.saturating_add(chunk_tokens) > config.max_source_tokens {
            truncated = true;
            break;
        }
        body_tokens += chunk_tokens;
        body.push_str(&chunk);
        included_files.push(relative);
    }

    Ok(CollectedSource {
        manifest: manifest(&root)?,
        body,
        included_files,
        truncated,
        evidence_cache_hits: 0,
        evidence_cache_misses: 0,
    })
}

pub fn collect_chunks_filtered(
    workspace: &Path,
    requested: &[String],
    config: &AppConfig,
    chunk_tokens: usize,
    max_chunks: usize,
    include: &[String],
    exclude: &[String],
) -> Result<Vec<CollectedSource>> {
    let root = canonical_workspace(workspace)?;
    let files = if requested.is_empty() {
        requested_files(&root, &[".".to_owned()], include, exclude)?
    } else {
        requested_files(&root, requested, include, exclude)?
    };
    let manifest = manifest(&root)?;
    let chunk_limit = chunk_tokens
        .max(2_500)
        .min(config.max_source_tokens.max(2_500));
    let mut chunks = Vec::new();
    let mut body = String::new();
    let mut body_tokens = 0usize;
    let mut included_files = Vec::new();
    let mut truncated = false;
    let max_file_tokens = config
        .max_file_tokens
        .min(chunk_limit.saturating_sub(250).max(500));

    for file in files {
        let Ok((relative, chunk)) = file_chunk(&root, &file, max_file_tokens) else {
            continue;
        };
        let chunk_cost = estimate_tokens(&chunk);
        let would_overflow =
            !body.is_empty() && body_tokens.saturating_add(chunk_cost) > chunk_limit;
        if would_overflow {
            chunks.push(CollectedSource {
                manifest: manifest.clone(),
                body: std::mem::take(&mut body),
                included_files: std::mem::take(&mut included_files),
                truncated: false,
                evidence_cache_hits: 0,
                evidence_cache_misses: 0,
            });
            body_tokens = 0;
            if chunks.len() >= max_chunks {
                truncated = true;
                break;
            }
        }
        body_tokens = body_tokens.saturating_add(chunk_cost);
        body.push_str(&chunk);
        included_files.push(relative);
    }

    if !body.is_empty() && chunks.len() < max_chunks {
        chunks.push(CollectedSource {
            manifest,
            body,
            included_files,
            truncated,
            evidence_cache_hits: 0,
            evidence_cache_misses: 0,
        });
    } else if truncated && let Some(last) = chunks.last_mut() {
        last.truncated = true;
    }

    Ok(chunks)
}

fn is_symbol_line(line: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "pub fn ",
        "pub async fn ",
        "fn ",
        "async fn ",
        "pub struct ",
        "struct ",
        "pub enum ",
        "enum ",
        "pub trait ",
        "trait ",
        "impl ",
        "class ",
        "def ",
        "async def ",
        "function ",
        "export function ",
        "export async function ",
        "export class ",
        "interface ",
        "export interface ",
        "type ",
        "export type ",
        "const ",
        "export const ",
        "# ",
        "## ",
    ];
    PREFIXES.iter().any(|prefix| line.starts_with(prefix))
}

fn file_symbol_candidates(text: &str, relative: &str) -> (Vec<SymbolCandidate>, usize) {
    const MAX_SCAN_LINES: usize = 20_000;
    const MAX_SYMBOLS_PER_FILE: usize = 64;
    const MAX_SYMBOL_SPAN_LINES: usize = 240;

    let mut declarations = Vec::<(usize, String)>::new();
    let mut total_lines = 0usize;
    for (index, line) in text.lines().take(MAX_SCAN_LINES).enumerate() {
        total_lines = index + 1;
        if declarations.len() >= MAX_SYMBOLS_PER_FILE {
            continue;
        }
        let trimmed = line.trim();
        if trimmed.len() <= 240 && is_symbol_line(trimmed) {
            declarations.push((index + 1, trimmed.to_owned()));
        }
    }

    let mut symbols = Vec::with_capacity(declarations.len());
    for (index, (start_line, label)) in declarations.iter().enumerate() {
        let next_start = declarations
            .get(index + 1)
            .map(|(line, _)| *line)
            .unwrap_or(total_lines.saturating_add(1));
        let natural_end = next_start.saturating_sub(1).max(*start_line);
        let end_line = natural_end.min(start_line.saturating_add(MAX_SYMBOL_SPAN_LINES - 1));
        symbols.push(SymbolCandidate {
            path: relative.to_owned(),
            label: label.clone(),
            start_line: *start_line,
            end_line,
        });
    }
    (symbols, total_lines)
}

fn index_file(path: &Path, relative: &str) -> Result<repo_cache::CachedFileIndex> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    if bytes.iter().take(8192).any(|byte| *byte == 0) {
        bail!("binary file skipped: {}", path.display());
    }
    let text = String::from_utf8_lossy(&bytes);
    let (symbols, total_lines) = file_symbol_candidates(&text, relative);
    Ok(repo_cache::CachedFileIndex {
        stamp: repo_cache::file_stamp(path)?,
        content_hash: repo_cache::hash_bytes(&bytes),
        total_lines,
        symbols,
    })
}

pub fn discovery_index(
    workspace: &Path,
    requested: &[String],
    config: &AppConfig,
    include: &[String],
    exclude: &[String],
) -> Result<DiscoveryIndex> {
    let root = canonical_workspace(workspace)?;
    let discovery_request = if requested.is_empty() {
        vec![".".to_owned()]
    } else {
        requested.to_vec()
    };
    let files = requested_files(&root, &discovery_request, include, exclude)?;
    let mut body = String::new();
    let mut candidate_files = Vec::new();
    let mut symbol_candidates = Vec::new();
    let mut used_tokens = 0usize;
    let mut truncated = false;
    let symbol_index = repo_cache::load_symbol_index(&root).unwrap_or_else(|error| {
        eprintln!("LLM2MCP symbol index cache read failed: {error:#}");
        repo_cache::RepositorySymbolIndex {
            version: repo_cache::SYMBOL_INDEX_VERSION.to_owned(),
            workspace: root.to_string_lossy().into_owned(),
            files: BTreeMap::new(),
        }
    });
    let mut updates = BTreeMap::new();
    let mut cache_hits = 0usize;
    let mut cache_misses = 0usize;

    for file in files {
        if candidate_files.len() >= 512 {
            truncated = true;
            break;
        }
        let relative = normalized_relative(file.strip_prefix(&root).unwrap_or(&file));
        let metadata = fs::metadata(&file).ok();
        let stamp = repo_cache::file_stamp(&file).ok();
        let cached = stamp.and_then(|stamp| {
            symbol_index
                .files
                .get(&relative)
                .filter(|entry| entry.stamp == stamp)
                .cloned()
        });
        let indexed = if let Some(cached) = cached {
            cache_hits += 1;
            cached
        } else {
            cache_misses += 1;
            let Ok(indexed) = index_file(&file, &relative) else {
                continue;
            };
            updates.insert(relative.clone(), indexed.clone());
            indexed
        };
        let symbols = indexed.symbols;
        let symbol_lines = symbols
            .iter()
            .take(32)
            .map(|symbol| {
                format!(
                    "  RANGE {}-{} {}",
                    symbol.start_line, symbol.end_line, symbol.label
                )
            })
            .collect::<Vec<_>>();
        let entry = if symbol_lines.is_empty() {
            format!(
                "FILE {relative} ({} bytes)\n",
                metadata.as_ref().map(|value| value.len()).unwrap_or(0)
            )
        } else {
            format!(
                "FILE {relative} ({} bytes)\nSYMBOLS\n{}\n",
                metadata.as_ref().map(|value| value.len()).unwrap_or(0),
                symbol_lines.join("\n")
            )
        };
        let entry_tokens = estimate_tokens(&entry);
        if used_tokens.saturating_add(entry_tokens) > config.discovery_index_tokens {
            truncated = true;
            break;
        }
        used_tokens += entry_tokens;
        body.push_str(&entry);
        body.push('\n');
        candidate_files.push(relative);
        symbol_candidates.extend(symbols);
    }

    if let Err(error) = repo_cache::merge_symbol_updates(&root, updates) {
        eprintln!("LLM2MCP symbol index cache write failed: {error:#}");
    }
    eprintln!(
        "LLM2MCP symbol index cache: hits={cache_hits} misses={cache_misses} candidates={}",
        candidate_files.len()
    );

    Ok(DiscoveryIndex {
        body,
        candidate_files,
        symbols: symbol_candidates,
        truncated,
        cache_hits,
        cache_misses,
    })
}

fn read_source_lines(path: &Path) -> Result<Vec<String>> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    if bytes.iter().take(8192).any(|byte| *byte == 0) {
        bail!("binary file skipped: {}", path.display());
    }
    Ok(String::from_utf8_lossy(&bytes)
        .lines()
        .take(20_000)
        .map(ToOwned::to_owned)
        .collect())
}

fn cached_evidence_segment(
    root: &Path,
    relative: &str,
    content_hash: &str,
    segment: &str,
    build: impl FnOnce() -> Result<String>,
    hits: &mut usize,
    misses: &mut usize,
) -> Result<String> {
    let key = repo_cache::evidence_key(relative, content_hash, segment);
    match repo_cache::load_evidence(root, &key) {
        Ok(Some(text)) => {
            *hits += 1;
            Ok(text)
        }
        Ok(None) => {
            *misses += 1;
            let text = build()?;
            if let Err(error) = repo_cache::store_evidence(root, &key, &text) {
                eprintln!("LLM2MCP evidence cache write failed for {relative}: {error:#}");
            }
            Ok(text)
        }
        Err(error) => {
            *misses += 1;
            eprintln!("LLM2MCP evidence cache read failed for {relative}: {error:#}");
            build()
        }
    }
}

pub fn collect_symbol_context(
    workspace: &Path,
    ranges: &[SymbolCandidate],
    config: &AppConfig,
) -> Result<CollectedSource> {
    const PRELUDE_LINES: usize = 30;
    const CONTEXT_LINES: usize = 4;

    let root = canonical_workspace(workspace)?;
    let mut grouped = BTreeMap::<String, Vec<SymbolCandidate>>::new();
    for range in ranges {
        grouped
            .entry(range.path.clone())
            .or_default()
            .push(range.clone());
    }

    let symbol_index = repo_cache::load_symbol_index(&root).unwrap_or_else(|error| {
        eprintln!("LLM2MCP symbol index cache read failed during evidence collection: {error:#}");
        repo_cache::RepositorySymbolIndex {
            version: repo_cache::SYMBOL_INDEX_VERSION.to_owned(),
            workspace: root.to_string_lossy().into_owned(),
            files: BTreeMap::new(),
        }
    });
    let mut index_updates = BTreeMap::new();
    let mut evidence_hits = 0usize;
    let mut evidence_misses = 0usize;
    let mut body = String::new();
    let mut used_tokens = 0usize;
    let mut included_files = Vec::new();
    let mut truncated = false;

    for (relative, mut file_ranges) in grouped {
        let path = secure_path(&root, &root.join(&relative))?;
        if is_secret(&path) || !looks_text(&path) {
            continue;
        }
        let stamp = repo_cache::file_stamp(&path)?;
        let indexed = if let Some(cached) = symbol_index
            .files
            .get(&relative)
            .filter(|entry| entry.stamp == stamp)
            .cloned()
        {
            cached
        } else {
            let indexed = index_file(&path, &relative)?;
            index_updates.insert(relative.clone(), indexed.clone());
            indexed
        };
        if indexed.total_lines == 0 {
            continue;
        }

        file_ranges.retain(|range| {
            indexed.symbols.iter().any(|current| {
                current.path == range.path
                    && current.label == range.label
                    && current.start_line == range.start_line
                    && current.end_line == range.end_line
            })
        });
        if file_ranges.is_empty() {
            eprintln!(
                "LLM2MCP selected symbol ranges became stale for {relative}; source changed during discovery"
            );
            continue;
        }
        file_ranges.sort_by_key(|range| range.start_line);
        let mut source_lines: Option<Vec<String>> = None;
        let mut file_body = format!("\n\n===== FILE: {relative} (SELECTED SYMBOLS) =====\n");

        if file_ranges
            .first()
            .is_some_and(|range| range.start_line > PRELUDE_LINES + CONTEXT_LINES)
        {
            let prelude_end = PRELUDE_LINES.min(indexed.total_lines);
            let segment = format!("prelude:1:{prelude_end}");
            let prelude = cached_evidence_segment(
                &root,
                &relative,
                &indexed.content_hash,
                &segment,
                || {
                    if source_lines.is_none() {
                        source_lines = Some(read_source_lines(&path)?);
                    }
                    let lines = source_lines.as_ref().expect("source lines initialized");
                    let mut text =
                        format!("--- FILE PRELUDE / IMPORT CONTEXT: lines 1-{prelude_end} ---\n");
                    for (index, line) in lines.iter().take(prelude_end).enumerate() {
                        text.push_str(&format!("{:>5} | {line}\n", index + 1));
                    }
                    Ok(text)
                },
                &mut evidence_hits,
                &mut evidence_misses,
            )?;
            file_body.push_str(&prelude);
        }

        let mut previous_end = 0usize;
        for range in file_ranges {
            let start = range
                .start_line
                .saturating_sub(CONTEXT_LINES)
                .max(1)
                .max(previous_end.saturating_add(1));
            let end = range
                .end_line
                .saturating_add(CONTEXT_LINES)
                .min(indexed.total_lines);
            if start > end {
                continue;
            }
            file_body.push_str(&format!(
                "--- SYMBOL {} | lines {}-{} ---\n",
                range.label, range.start_line, range.end_line
            ));
            let segment = format!("symbol:{start}:{end}:ctx{CONTEXT_LINES}");
            let evidence = cached_evidence_segment(
                &root,
                &relative,
                &indexed.content_hash,
                &segment,
                || {
                    if source_lines.is_none() {
                        source_lines = Some(read_source_lines(&path)?);
                    }
                    let lines = source_lines.as_ref().expect("source lines initialized");
                    let mut text = String::new();
                    for line_number in start..=end {
                        if let Some(line) = lines.get(line_number - 1) {
                            text.push_str(&format!("{:>5} | {line}\n", line_number));
                        }
                    }
                    Ok(text)
                },
                &mut evidence_hits,
                &mut evidence_misses,
            )?;
            file_body.push_str(&evidence);
            previous_end = end;
        }
        file_body.push_str(&format!("===== END FILE: {relative} =====\n"));

        let remaining = config.max_source_tokens.saturating_sub(used_tokens);
        if remaining < 128 {
            truncated = true;
            break;
        }
        let (file_body, clipped) = truncate_tokens(
            &file_body,
            remaining,
            "\n[SELECTED SYMBOL CONTEXT TRUNCATED]\n",
        );
        let cost = estimate_tokens(&file_body);
        body.push_str(&file_body);
        used_tokens = used_tokens.saturating_add(cost);
        included_files.push(relative);
        if clipped {
            truncated = true;
            break;
        }
    }

    if let Err(error) = repo_cache::merge_symbol_updates(&root, index_updates) {
        eprintln!("LLM2MCP symbol index refresh failed during evidence collection: {error:#}");
    }
    eprintln!(
        "LLM2MCP evidence cache: hits={evidence_hits} misses={evidence_misses} files={}",
        included_files.len()
    );

    Ok(CollectedSource {
        manifest: manifest(&root)?,
        body,
        included_files,
        truncated,
        evidence_cache_hits: evidence_hits,
        evidence_cache_misses: evidence_misses,
    })
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

pub fn git_diff(
    workspace: &Path,
    base_ref: &str,
    max_tokens: usize,
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

    let (text, truncated) = truncate_to_token_budget(
        &String::from_utf8_lossy(&diff.stdout),
        max_tokens,
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
    max_tokens: usize,
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
    Ok(truncate_to_token_budget(
        &String::from_utf8_lossy(&diff.stdout),
        max_tokens,
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

    #[test]
    fn token_estimator_and_truncation_use_token_budget() {
        assert_eq!(estimate_tokens("中文测试"), 4);
        assert_eq!(estimate_tokens("abcdefgh"), 2);
        let (clipped, truncated) = truncate_to_token_budget("中文测试更多内容", 4, "[T]");
        assert!(truncated);
        assert!(clipped.starts_with("中文测试"));
        assert!(clipped.ends_with("[T]"));
    }

    #[test]
    fn symbol_context_reads_selected_range_instead_of_irrelevant_middle() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "llm2mcp-symbol-context-{}-{nonce}",
            std::process::id()
        ));
        let src = root.join("src");
        fs::create_dir_all(&src).expect("create src");
        let path = src.join("sample.rs");
        let mut lines = vec!["fn first_helper() { }".to_owned()];
        for index in 2..=45 {
            if index == 35 {
                lines.push("// IRRELEVANT_MIDDLE_MARKER".to_owned());
            } else {
                lines.push(format!("// filler {index}"));
            }
        }
        lines.push("fn target_worker() {".to_owned());
        lines.push("    println!(\"target\");".to_owned());
        lines.push("}".to_owned());
        fs::write(&path, lines.join("\n")).expect("write source");

        let text = fs::read_to_string(&path).expect("read source");
        let (symbols, _) = file_symbol_candidates(&text, "src/sample.rs");
        let target = symbols
            .into_iter()
            .find(|symbol| symbol.label.contains("target_worker"))
            .expect("target symbol");
        let config = AppConfig {
            max_source_tokens: 2_000,
            ..AppConfig::default()
        };
        let collected = collect_symbol_context(&root, &[target], &config).expect("collect range");

        assert!(collected.body.contains("target_worker"));
        assert!(collected.body.contains("SELECTED SYMBOLS"));
        assert!(!collected.body.contains("IRRELEVANT_MIDDLE_MARKER"));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn persistent_symbol_index_hits_then_invalidates_changed_file() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "llm2mcp-symbol-index-cache-{}-{nonce}",
            std::process::id()
        ));
        let src = root.join("src");
        fs::create_dir_all(&src).expect("create src");
        let path = src.join("sample.rs");
        fs::write(&path, "fn alpha() {\n    println!(\"a\");\n}\n").expect("seed source");
        repo_cache::clear_workspace_cache(&root).expect("clear stale cache");

        let config = AppConfig::default();
        let requested = vec!["src".to_owned()];
        let first = discovery_index(&root, &requested, &config, &[], &[]).expect("first index");
        assert_eq!(first.cache_hits, 0);
        assert_eq!(first.cache_misses, 1);

        let second = discovery_index(&root, &requested, &config, &[], &[]).expect("second index");
        assert_eq!(second.cache_hits, 1);
        assert_eq!(second.cache_misses, 0);

        fs::write(
            &path,
            "fn alpha() {\n    println!(\"changed-and-longer\");\n}\n",
        )
        .expect("change source");
        let third = discovery_index(&root, &requested, &config, &[], &[]).expect("third index");
        assert_eq!(third.cache_hits, 0);
        assert_eq!(third.cache_misses, 1);

        repo_cache::clear_workspace_cache(&root).expect("clear cache");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn evidence_cache_hits_then_invalidates_when_content_changes() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "llm2mcp-evidence-cache-{}-{nonce}",
            std::process::id()
        ));
        let src = root.join("src");
        fs::create_dir_all(&src).expect("create src");
        let path = src.join("sample.rs");
        fs::write(
            &path,
            "fn target_worker() {\n    println!(\"a\");\n}\nfn other() {}\n",
        )
        .expect("seed source");
        repo_cache::clear_workspace_cache(&root).expect("clear stale cache");

        let config = AppConfig {
            max_source_tokens: 2_000,
            ..AppConfig::default()
        };
        let requested = vec!["src".to_owned()];
        let index = discovery_index(&root, &requested, &config, &[], &[]).expect("index");
        let target = index
            .symbols
            .into_iter()
            .find(|symbol| symbol.label.contains("target_worker"))
            .expect("target symbol");

        let first = collect_symbol_context(&root, std::slice::from_ref(&target), &config)
            .expect("first evidence");
        assert!(first.evidence_cache_misses > 0);
        assert_eq!(first.evidence_cache_hits, 0);

        let second = collect_symbol_context(&root, std::slice::from_ref(&target), &config)
            .expect("second evidence");
        assert!(second.evidence_cache_hits > 0);
        assert_eq!(second.evidence_cache_misses, 0);

        fs::write(
            &path,
            "fn target_worker() {\n    println!(\"changed-and-longer\");\n}\nfn other() {}\n",
        )
        .expect("change source");
        let third = collect_symbol_context(&root, std::slice::from_ref(&target), &config)
            .expect("changed evidence");
        assert!(third.evidence_cache_misses > 0);
        assert_eq!(third.evidence_cache_hits, 0);
        assert!(third.body.contains("changed-and-longer"));

        repo_cache::clear_workspace_cache(&root).expect("clear cache");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn source_globs_support_double_star_and_basename_filters() {
        assert!(glob_match("src/**/*.rs", "src/main.rs"));
        assert!(glob_match("src/**/*.rs", "src/nested/mod.rs"));
        assert!(!glob_match("src/**/*.rs", "tests/main.rs"));
        assert!(matches_filters(
            "src/nested/mod.rs",
            &["src/**/*.rs".to_owned()],
            &["generated.rs".to_owned()]
        ));
        assert!(!matches_filters(
            "src/generated.rs",
            &["src/**/*.rs".to_owned()],
            &["generated.rs".to_owned()]
        ));
    }
}
