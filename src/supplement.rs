//! One supplemental retrieval round; no autonomous agent loop.
use super::*;

pub(super) struct AnalysisContext<'a> {
    pub(super) config: &'a AppConfig,
    pub(super) root: &'a Path,
    pub(super) system: &'a str,
    pub(super) tool: &'a ToolConfig,
    pub(super) filters: (&'a [String], &'a [String]),
    pub(super) allow: bool,
    pub(super) reporter: Option<&'a Reporter>,
}

pub(super) fn analyze_with_supplement(
    context: &AnalysisContext<'_>,
    prompt: &str,
    selection: &mut DeepSelection,
    sources: &mut workspace::CollectedSource,
) -> Result<String> {
    let config = context.config;
    let first = chat_with_reporter(
        config,
        context.system,
        prompt,
        context.tool.reasoning,
        context.tool.max_output_tokens,
        context.reporter,
    )?;
    let mut value: Value = serde_json::from_str(&first)?;
    let requests = value
        .get("context_requests")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if requests.is_empty() {
        return Ok(first);
    }
    if !context.allow {
        value["retrieval"] =
            json!(["Supplemental retrieval disabled; context requests remain unresolved."]);
        return Ok(value.to_string());
    }
    let supplied = crate::evidence::Catalog::new(&sources.body, "", "");
    let mut extra_selection = DeepSelection {
        files: Vec::new(),
        symbols: Vec::new(),
        truncated: false,
        used_llm: false,
    };
    let mut notes = Vec::new();
    let mut read_budget = 64 * 1024 * 1024;
    let mut snippets = Vec::new();
    let mut snippet_body = String::new();
    let mut snippet_files = Vec::new();
    for request in requests.iter().take(3) {
        crate::control::Control::current().check()?;
        let query = request.get("query").and_then(Value::as_str).unwrap_or("");
        let paths = string_list_arg(request, "paths");
        if query.len() > 2048 || paths.len() > 8 || (query.trim().is_empty() && paths.is_empty()) {
            notes.push("Rejected malformed supplemental request.".to_owned());
            continue;
        }
        let query = format!("{query} {}", paths.join(" "));
        let index = workspace::discovery_index_for_task(
            context.root,
            &[],
            config,
            context.filters.0,
            context.filters.1,
            &query,
        )?;
        if let Some(reporter) = context.reporter {
            reporter.record_cache_usage(
                index.cache_hits as u64,
                index.cache_misses as u64,
                0,
                0,
            )?;
        }
        let keywords = task_keywords(request.get("query").and_then(Value::as_str).unwrap_or(""));
        let before = extra_selection.symbols.len();
        let mut symbols = index
            .symbols
            .iter()
            .filter(|symbol| {
                (paths.is_empty() || paths.contains(&symbol.path))
                    && crate::search::score(&keywords, &symbol.label) > 0
                    && !supplied.accepts(&format!(
                        "{}:{}-{}",
                        symbol.path, symbol.start_line, symbol.end_line
                    ))
            })
            .collect::<Vec<_>>();
        symbols.sort_by_key(|symbol| {
            std::cmp::Reverse(crate::search::score(&keywords, &symbol.label))
        });
        for symbol in symbols.into_iter().take(2) {
            if extra_selection.symbols.len() + extra_selection.files.len() + snippets.len() >= 6 {
                break;
            }
            if !extra_selection.symbols.contains(symbol) {
                extra_selection.symbols.push(symbol.clone());
            }
        }
        if extra_selection.symbols.len() == before
            && extra_selection.symbols.len() + extra_selection.files.len() + snippets.len() < 6
        {
            let candidates = index
                .candidate_files
                .iter()
                .filter(|path| paths.is_empty() || paths.contains(path))
                .cloned()
                .collect::<Vec<_>>();
            let mut search_config = config.clone();
            search_config.max_source_tokens = (config.max_source_tokens / 2)
                .saturating_sub(workspace::estimate_tokens(&snippet_body));
            let previous_matches = snippets.len();
            let match_limit =
                (6 - extra_selection.symbols.len() - extra_selection.files.len() - snippets.len())
                    .min(2);
            let (matched, ranges) = workspace::supplemental_text_context(
                context.root,
                &candidates,
                request.get("query").and_then(Value::as_str).unwrap_or(""),
                &search_config,
                &mut read_budget,
                match_limit,
                &supplied,
            )?;
            for range in ranges {
                if extra_selection.symbols.len() + extra_selection.files.len() + snippets.len() >= 6
                {
                    break;
                }
                if !supplied.accepts(&format!(
                    "{}:{}-{}",
                    range.path, range.start_line, range.end_line
                )) && !snippets.contains(&range)
                {
                    snippets.push(range);
                }
            }
            if snippets.len() > previous_matches {
                snippet_body.push_str(&matched.body);
                for path in matched.included_files {
                    if !snippet_files.contains(&path) {
                        snippet_files.push(path);
                    }
                }
            }
            extra_selection.truncated |= matched.truncated;
        }
        // Whole-file fallback is limited to real indexed paths, never model-created ranges.
        for path in index
            .candidate_files
            .iter()
            .filter(|path| {
                (paths.is_empty() || paths.contains(path))
                    && crate::search::score(&keywords, path) > 0
            })
            .take(2)
        {
            if extra_selection.symbols.len() + extra_selection.files.len() + snippets.len() >= 6 {
                break;
            }
            if !extra_selection
                .symbols
                .iter()
                .any(|symbol| &symbol.path == path)
                && !snippets.iter().any(|range| &range.path == path)
                && !extra_selection.files.contains(path)
                && !supplied.accepts(path)
            {
                extra_selection.files.push(path.clone());
            }
        }
        extra_selection.truncated |= index.truncated;
    }
    if requests.len() > 3 {
        notes.push("Supplemental request limit reached (3).".into());
    }
    if extra_selection.symbols.is_empty() && extra_selection.files.is_empty() && snippets.is_empty()
    {
        notes.push(
            "No new eligible indexed evidence matched; unresolved requests were not executed."
                .into(),
        );
        value["retrieval"] = json!(notes);
        return Ok(value.to_string());
    }
    progress(context.reporter, "retrieving supplemental evidence", 0, 1)?;
    let source_marker = format!("SOURCE MATERIAL\n{}", sources.body);
    let prefix = prompt
        .rsplit_once(&source_marker)
        .context("missing source material boundary")?
        .0;
    let matched_ranges = snippets
        .iter()
        .map(|range| {
            format!(
                "{}:{}-{} {}",
                range.path, range.start_line, range.end_line, range.label
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let selected = extra_selection.description();
    let metadata = format!(
        "{prefix}\nSUPPLEMENT_SELECTED_CONTEXT\n{selected}\n{matched_ranges}\n\nOne supplemental round only. Resolve the original task from the combined source; mark remaining unknowns. Do not request another retrieval round.\n\nSOURCE MATERIAL\n"
    );
    let bounded = budgeted_config(
        config,
        context.system,
        &metadata,
        context.tool.max_output_tokens,
    )?;
    let mut extra_config = bounded.clone();
    extra_config.max_source_tokens = (bounded.max_source_tokens / 2).max(128);
    extra_config.max_file_tokens = extra_config
        .max_file_tokens
        .min(extra_config.max_source_tokens);
    let mut extra = if extra_selection.symbols.is_empty() && extra_selection.files.is_empty() {
        workspace::CollectedSource {
            manifest: String::new(),
            body: String::new(),
            included_files: Vec::new(),
            truncated: false,
            evidence_cache_hits: 0,
            evidence_cache_misses: 0,
        }
    } else {
        collect_deep_selection(
            &extra_config,
            context.root,
            &extra_selection,
            context.filters.0,
            context.filters.1,
        )?
    };
    let combined = format!("{snippet_body}\n{}", extra.body);
    let bounded_extra = workspace::truncate_tokens_strict(
        &combined,
        extra_config.max_source_tokens,
        "[SUPPLEMENTAL SOURCE TRUNCATED]",
    );
    extra.truncated |= bounded_extra != combined || extra_selection.truncated;
    extra.body = bounded_extra;
    for path in snippet_files {
        if !extra.included_files.contains(&path) {
            extra.included_files.push(path);
        }
    }
    if extra.included_files.is_empty() {
        notes.push("Supplemental candidates produced no readable evidence.".into());
        value["retrieval"] = json!(notes);
        return Ok(value.to_string());
    }
    let remaining = bounded
        .max_source_tokens
        .saturating_sub(workspace::estimate_tokens(&extra.body) + 2);
    let original =
        workspace::truncate_tokens_strict(&sources.body, remaining, "[ORIGINAL SOURCE TRUNCATED]");
    sources.truncated |= original != sources.body || extra.truncated;
    let joined = format!("{}\n\n{original}", extra.body);
    sources.body = workspace::truncate_tokens_strict(
        &joined,
        bounded.max_source_tokens,
        "[COMBINED SOURCE TRUNCATED]",
    );
    sources.truncated |= sources.body != joined;
    for path in extra.included_files {
        if !sources.included_files.contains(&path) {
            sources.included_files.push(path);
        }
    }
    selection.symbols.extend(extra_selection.symbols);
    selection.symbols.extend(snippets);
    selection.files.extend(extra_selection.files);
    let catalog = crate::evidence::Catalog::new(&sources.body, "", "");
    selection.symbols.retain(|symbol| {
        catalog.accepts(&format!(
            "{}:{}-{}",
            symbol.path, symbol.start_line, symbol.end_line
        ))
    });
    selection.files.retain(|path| catalog.accepts(path));
    if let Some(reporter) = context.reporter {
        reporter.record_context(
            workspace::estimate_tokens(&sources.body),
            sources.included_files.len(),
            sources.truncated,
        )?;
        reporter.record_cache_usage(
            0,
            0,
            extra.evidence_cache_hits as u64,
            extra.evidence_cache_misses as u64,
        )?;
    }
    let followup = format!(
        "{metadata}{}\n\nSOURCE_TRUNCATED\n{}",
        sources.body, sources.truncated
    );
    let final_result = chat_with_reporter(
        config,
        context.system,
        &followup,
        context.tool.reasoning,
        context.tool.max_output_tokens,
        context.reporter,
    )?;
    let mut value: Value = serde_json::from_str(&final_result)?;
    notes.push("One supplemental retrieval round completed; at most six new candidates.".into());
    if value
        .get("context_requests")
        .and_then(Value::as_array)
        .is_some_and(|requests| !requests.is_empty())
    {
        notes.push("Additional context requests remain unresolved (one-round limit).".into());
    }
    value["retrieval"] = json!(notes);
    Ok(value.to_string())
}
