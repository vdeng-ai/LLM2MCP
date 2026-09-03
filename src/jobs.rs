use anyhow::{Context, Result, bail};
use directories::ProjectDirs;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

#[cfg(test)]
const DEFAULT_POLL_INTERVAL_MS: u64 = 5_000;
#[cfg(test)]
const DEFAULT_TTL_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
static JOB_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Working,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRecord {
    pub id: String,
    pub tool: String,
    pub workspace: PathBuf,
    pub args: Value,
    pub state: JobState,
    pub stage: String,
    pub progress: u64,
    pub total: u64,
    pub cancel_requested: bool,
    pub created_at: String,
    pub updated_at: String,
    pub ttl_ms: u64,
    pub poll_interval_ms: u64,
    pub result: Option<Value>,
    pub error: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub llm_calls: u64,
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub last_llm_diagnostics: Option<String>,
    #[serde(default)]
    pub error_hint: Option<String>,
    #[serde(default)]
    pub symbol_index_hits: u64,
    #[serde(default)]
    pub symbol_index_misses: u64,
    #[serde(default)]
    pub evidence_cache_hits: u64,
    #[serde(default)]
    pub evidence_cache_misses: u64,
}

#[derive(Debug, Clone)]
pub struct Reporter {
    job_id: String,
}

impl Reporter {
    pub fn new(job_id: impl Into<String>) -> Self {
        Self {
            job_id: job_id.into(),
        }
    }

    pub fn update(&self, stage: &str, progress: u64, total: u64) -> Result<()> {
        update_record(&self.job_id, |record| {
            if record.state == JobState::Working {
                record.stage = stage.to_owned();
                record.progress = progress;
                record.total = total;
            }
        })?;
        self.check_cancelled()
    }

    pub fn record_llm_attempt(&self) -> Result<()> {
        update_record(&self.job_id, |record| {
            record.llm_calls = record.llm_calls.saturating_add(1);
        })
    }

    pub fn record_llm_usage(
        &self,
        prompt_tokens: Option<u64>,
        completion_tokens: Option<u64>,
        diagnostics: &str,
    ) -> Result<()> {
        update_record(&self.job_id, |record| {
            record.prompt_tokens = record
                .prompt_tokens
                .saturating_add(prompt_tokens.unwrap_or_default());
            record.completion_tokens = record
                .completion_tokens
                .saturating_add(completion_tokens.unwrap_or_default());
            record.last_llm_diagnostics = Some(diagnostics.to_owned());
        })
    }

    pub fn record_cache_usage(
        &self,
        symbol_hits: u64,
        symbol_misses: u64,
        evidence_hits: u64,
        evidence_misses: u64,
    ) -> Result<()> {
        update_record(&self.job_id, |record| {
            record.symbol_index_hits = record.symbol_index_hits.saturating_add(symbol_hits);
            record.symbol_index_misses = record.symbol_index_misses.saturating_add(symbol_misses);
            record.evidence_cache_hits = record.evidence_cache_hits.saturating_add(evidence_hits);
            record.evidence_cache_misses =
                record.evidence_cache_misses.saturating_add(evidence_misses);
        })
    }

    pub fn check_cancelled(&self) -> Result<()> {
        let record = load(&self.job_id)?;
        if record.cancel_requested || record.state == JobState::Cancelled {
            bail!("job cancelled")
        }
        Ok(())
    }
}

pub fn create(
    tool: &str,
    workspace: &Path,
    args: Value,
    ttl_hours: u64,
    poll_interval_ms: u64,
) -> Result<JobRecord> {
    let workspace = workspace
        .canonicalize()
        .with_context(|| format!("invalid workspace: {}", workspace.display()))?;
    let now = now_rfc3339();
    let id = new_job_id();
    let record = JobRecord {
        id: id.clone(),
        tool: tool.to_owned(),
        workspace,
        args,
        state: JobState::Working,
        stage: "queued".to_owned(),
        progress: 0,
        total: 0,
        cancel_requested: false,
        created_at: now.clone(),
        updated_at: now,
        ttl_ms: ttl_hours.clamp(1, 24 * 365).saturating_mul(60 * 60 * 1_000),
        poll_interval_ms: poll_interval_ms.clamp(1_000, 60_000),
        result: None,
        error: None,
        duration_ms: None,
        llm_calls: 0,
        prompt_tokens: 0,
        completion_tokens: 0,
        last_llm_diagnostics: None,
        error_hint: None,
        symbol_index_hits: 0,
        symbol_index_misses: 0,
        evidence_cache_hits: 0,
        evidence_cache_misses: 0,
    };
    save(&record)?;

    if let Err(error) = spawn_worker(&id) {
        let message = format!("failed to start job worker: {error:#}");
        fail(&id, &message)?;
        bail!(message)
    }

    load(&id)
}

pub fn load(job_id: &str) -> Result<JobRecord> {
    validate_job_id(job_id)?;
    let path = job_path(job_id)?;
    let text = fs::read_to_string(&path)
        .with_context(|| format!("unknown job_id or unreadable job: {job_id}"))?;
    serde_json::from_str(&text).with_context(|| format!("invalid job state: {}", path.display()))
}

pub fn save(record: &JobRecord) -> Result<()> {
    validate_job_id(&record.id)?;
    let path = job_path(&record.id)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut record = record.clone();
    record.updated_at = now_rfc3339();
    let data = serde_json::to_vec_pretty(&record)?;
    let temp = path.with_extension(format!("json.tmp.{}", std::process::id()));
    fs::write(&temp, data)?;
    #[cfg(windows)]
    {
        if path.exists() {
            fs::remove_file(&path)?;
        }
        fs::rename(&temp, &path)?;
    }
    #[cfg(not(windows))]
    {
        // POSIX rename replaces the destination atomically, so readers always
        // see either the previous complete record or the next complete record.
        fs::rename(&temp, &path)?;
    }
    Ok(())
}

pub fn complete(job_id: &str, result: Value) -> Result<()> {
    update_record(job_id, |record| {
        if record.cancel_requested {
            record.state = JobState::Cancelled;
            record.stage = "cancelled".to_owned();
            record.result = None;
            record.error = None;
            record.error_hint = None;
            record.duration_ms = Some(elapsed_ms(record));
            return;
        }
        record.state = JobState::Completed;
        record.stage = "completed".to_owned();
        record.result = Some(result);
        record.error = None;
        record.error_hint = None;
        record.duration_ms = Some(elapsed_ms(record));
        if record.total > 0 {
            record.progress = record.total;
        }
    })
}

pub fn fail(job_id: &str, message: &str) -> Result<()> {
    update_record(job_id, |record| {
        if record.cancel_requested {
            record.state = JobState::Cancelled;
            record.stage = "cancelled".to_owned();
            record.error = None;
            record.error_hint = None;
            record.duration_ms = Some(elapsed_ms(record));
        } else {
            record.state = JobState::Failed;
            if !record.stage.ends_with("_failed") {
                record.stage = "failed".to_owned();
            }
            record.error = Some(message.to_owned());
            record.error_hint = diagnostic_hint(message);
            record.duration_ms = Some(elapsed_ms(record));
        }
    })
}

pub fn cancel(job_id: &str) -> Result<JobRecord> {
    update_record(job_id, |record| {
        if matches!(record.state, JobState::Working) {
            record.cancel_requested = true;
            record.stage = "cancellation requested".to_owned();
        }
    })?;
    load(job_id)
}

pub fn mark_cancelled(job_id: &str) -> Result<()> {
    update_record(job_id, |record| {
        record.state = JobState::Cancelled;
        record.stage = "cancelled".to_owned();
        record.result = None;
        record.error = None;
        record.error_hint = None;
        record.duration_ms = Some(elapsed_ms(record));
    })
}

pub fn fallback_started_result(record: &JobRecord) -> Value {
    json!({
        "resultType": "complete",
        "content": [{
            "type": "text",
            "text": format!(
                "LLM2MCP started a background job. job_id={} status=working. Poll with job_status, then call job_result when completed. Use job_cancel to request cancellation.",
                record.id
            )
        }],
        "structuredContent": {
            "job_id": record.id,
            "status": "working",
            "stage": record.stage,
            "poll_after_ms": record.poll_interval_ms
        },
        "isError": false
    })
}

pub fn fallback_status_result(record: &JobRecord) -> Value {
    let status = state_name(record.state);
    let text = if record.total > 0 {
        format!(
            "job_id={} status={} stage={} progress={}/{}",
            record.id, status, record.stage, record.progress, record.total
        )
    } else {
        format!(
            "job_id={} status={} stage={}",
            record.id, status, record.stage
        )
    };
    json!({
        "resultType": "complete",
        "content": [{"type": "text", "text": text}],
        "structuredContent": {
            "job_id": record.id,
            "tool": record.tool,
            "status": status,
            "stage": record.stage,
            "progress": record.progress,
            "total": record.total,
            "cancel_requested": record.cancel_requested,
            "created_at": record.created_at,
            "updated_at": record.updated_at,
            "poll_after_ms": record.poll_interval_ms,
            "duration_ms": display_duration_ms(record),
            "llm_calls": record.llm_calls,
            "prompt_tokens": record.prompt_tokens,
            "completion_tokens": record.completion_tokens,
            "last_llm_diagnostics": record.last_llm_diagnostics,
            "symbol_index_hits": record.symbol_index_hits,
            "symbol_index_misses": record.symbol_index_misses,
            "evidence_cache_hits": record.evidence_cache_hits,
            "evidence_cache_misses": record.evidence_cache_misses,
            "error": record.error,
            "error_hint": record.error_hint
        },
        "isError": false
    })
}

pub fn fallback_result(record: &JobRecord) -> Value {
    match record.state {
        JobState::Completed => record.result.clone().unwrap_or_else(|| {
            json!({
                "resultType": "complete",
                "content": [{"type": "text", "text": "Job completed without a stored result."}],
                "isError": true
            })
        }),
        JobState::Failed => json!({
            "resultType": "complete",
            "content": [{"type": "text", "text": format!("Job failed: {}", record.error.as_deref().unwrap_or("unknown error"))}],
            "isError": true
        }),
        JobState::Cancelled => json!({
            "resultType": "complete",
            "content": [{"type": "text", "text": "Job was cancelled."}],
            "isError": false
        }),
        JobState::Working => fallback_status_result(record),
    }
}

pub fn create_task_result(record: &JobRecord) -> Value {
    json!({
        "resultType": "task",
        "taskId": record.id,
        "status": "working",
        "statusMessage": status_message(record),
        "createdAt": record.created_at,
        "lastUpdatedAt": record.updated_at,
        "ttlMs": record.ttl_ms,
        "pollIntervalMs": record.poll_interval_ms
    })
}

pub fn task_get_result(record: &JobRecord) -> Value {
    let mut value = json!({
        "resultType": "complete",
        "taskId": record.id,
        "status": state_name(record.state),
        "statusMessage": status_message(record),
        "createdAt": record.created_at,
        "lastUpdatedAt": record.updated_at,
        "ttlMs": record.ttl_ms,
        "pollIntervalMs": record.poll_interval_ms,
        "_meta": {
            "io.llm2mcp/progress": {
                "stage": record.stage,
                "progress": record.progress,
                "total": record.total,
                "cancelRequested": record.cancel_requested,
                "durationMs": display_duration_ms(record),
                "llmCalls": record.llm_calls,
                "promptTokens": record.prompt_tokens,
                "completionTokens": record.completion_tokens,
                "lastLlmDiagnostics": record.last_llm_diagnostics,
                "symbolIndexHits": record.symbol_index_hits,
                "symbolIndexMisses": record.symbol_index_misses,
                "evidenceCacheHits": record.evidence_cache_hits,
                "evidenceCacheMisses": record.evidence_cache_misses,
                "errorHint": record.error_hint
            }
        }
    });
    if let Some(object) = value.as_object_mut() {
        match record.state {
            JobState::Completed => {
                object.insert(
                    "result".to_owned(),
                    record.result.clone().unwrap_or_else(|| json!({})),
                );
            }
            JobState::Failed => {
                object.insert(
                    "error".to_owned(),
                    json!({
                        "code": -32603,
                        "message": record.error.as_deref().unwrap_or("background job failed")
                    }),
                );
            }
            JobState::Working | JobState::Cancelled => {}
        }
    }
    value
}

pub fn list_recent(limit: usize) -> Result<Vec<JobRecord>> {
    let dir = jobs_dir()?;
    if !dir.exists() || limit == 0 {
        return Ok(Vec::new());
    }
    let mut records = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        if let Ok(record) = serde_json::from_str::<JobRecord>(&text) {
            records.push(record);
        }
    }
    records.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    records.truncate(limit);
    Ok(records)
}

pub fn cleanup_expired() -> Result<usize> {
    let dir = jobs_dir()?;
    if !dir.exists() {
        return Ok(0);
    }
    let now_ms = unix_millis();
    let mut removed = 0;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(record) = serde_json::from_str::<JobRecord>(&text) else {
            continue;
        };
        let created_ms = parse_rfc3339_millis(&record.created_at).unwrap_or(now_ms);
        if now_ms.saturating_sub(created_ms) > record.ttl_ms {
            let _ = fs::remove_file(&path);
            let _ = fs::remove_file(log_path(&record.id)?);
            let _ = fs::remove_file(lock_path(&record.id)?);
            removed += 1;
        }
    }
    Ok(removed)
}

fn update_record<F>(job_id: &str, update: F) -> Result<()>
where
    F: FnOnce(&mut JobRecord),
{
    validate_job_id(job_id)?;
    let lock_path = lock_path(job_id)?;
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)?;
    lock.lock_exclusive()?;

    let result = (|| {
        let mut record = load(job_id)?;
        update(&mut record);
        save(&record)
    })();

    FileExt::unlock(&lock)?;
    result
}

fn worker_executable() -> Result<PathBuf> {
    let executable = std::env::current_exe().context("cannot resolve LLM2MCP executable")?;
    if executable.exists() {
        return Ok(executable);
    }

    #[cfg(target_os = "linux")]
    {
        // During development or an in-place update, Cargo/package managers may
        // atomically replace the executable while an MCP process is still
        // running. Linux then exposes current_exe() as `... (deleted)`, which
        // cannot be passed to execve by pathname. /proc/self/exe still points
        // at the live executable inode and lets us launch a same-version worker.
        let proc_self_exe = PathBuf::from("/proc/self/exe");
        if proc_self_exe.exists() {
            eprintln!(
                "LLM2MCP executable path no longer exists ({}); spawning worker via /proc/self/exe",
                executable.display()
            );
            return Ok(proc_self_exe);
        }
    }

    bail!(
        "LLM2MCP executable no longer exists at {}; restart/reinstall the MCP server before starting background jobs",
        executable.display()
    )
}

fn spawn_worker(job_id: &str) -> Result<()> {
    let executable = worker_executable()?;
    let log_path = log_path(job_id)?;
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let log = File::create(&log_path)?;
    let log_err = log.try_clone()?;
    let mut command = Command::new(executable);
    command
        .args(["job-worker", "--job-id", job_id])
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    command
        .spawn()
        .context("failed to spawn background worker")?;
    Ok(())
}

fn jobs_dir() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("ai", "LLM2MCP", "LLM2MCP")
        .context("cannot resolve LLM2MCP data directory")?;
    Ok(dirs.data_local_dir().join("jobs"))
}

fn job_path(job_id: &str) -> Result<PathBuf> {
    validate_job_id(job_id)?;
    Ok(jobs_dir()?.join(format!("{job_id}.json")))
}

fn log_path(job_id: &str) -> Result<PathBuf> {
    validate_job_id(job_id)?;
    Ok(jobs_dir()?.join(format!("{job_id}.log")))
}

fn lock_path(job_id: &str) -> Result<PathBuf> {
    validate_job_id(job_id)?;
    Ok(jobs_dir()?.join(format!("{job_id}.lock")))
}

fn validate_job_id(job_id: &str) -> Result<()> {
    if job_id.is_empty()
        || job_id.len() > 96
        || !job_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        bail!("invalid job_id")
    }
    Ok(())
}

fn new_job_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let counter = JOB_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("job_{nanos:x}_{:x}_{counter:x}", std::process::id())
}

fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn parse_rfc3339_millis(value: &str) -> Option<u64> {
    let timestamp = OffsetDateTime::parse(value, &Rfc3339).ok()?;
    let millis = timestamp.unix_timestamp_nanos() / 1_000_000;
    u64::try_from(millis).ok()
}

fn elapsed_ms(record: &JobRecord) -> u64 {
    let created = parse_rfc3339_millis(&record.created_at).unwrap_or_else(unix_millis);
    unix_millis().saturating_sub(created)
}

pub fn display_duration_ms(record: &JobRecord) -> u64 {
    record.duration_ms.unwrap_or_else(|| elapsed_ms(record))
}

pub fn diagnostic_hint(message: &str) -> Option<String> {
    let lower = message.to_ascii_lowercase();
    let hint = if lower.contains("401") || lower.contains("403") || lower.contains("unauthorized") {
        "Check the configured API key and provider authorization."
    } else if lower.contains("429") || lower.contains("rate limit") {
        "The LLM endpoint is rate-limiting requests; reduce concurrency or retry later."
    } else if lower.contains("timed out") || lower.contains("timeout") {
        "The LLM request timed out; increase the configured timeout or use Async execution for long tasks."
    } else if lower.contains("failed to connect") || lower.contains("connection refused") {
        "The configured LLM endpoint is unreachable; verify Base URL, network access, and whether the model server is running."
    } else if lower.contains("no final message content")
        || lower.contains("finish_reason=length")
        || lower.contains("completion_tokens") && lower.contains("reasoning")
    {
        "The model likely exhausted its output budget in reasoning; increase max output tokens or lower reasoning effort."
    } else if lower.contains("no readable source files") {
        "No files matched the requested paths/include/exclude filters after gitignore and secret filtering."
    } else if lower.contains("no such file or directory") && lower.contains("worker") {
        "Refresh the stable LLM2MCP installation and restart the MCP host before retrying the background job."
    } else {
        return None;
    };
    Some(hint.to_owned())
}

pub fn state_name(state: JobState) -> &'static str {
    match state {
        JobState::Working => "working",
        JobState::Completed => "completed",
        JobState::Failed => "failed",
        JobState::Cancelled => "cancelled",
    }
}

fn status_message(record: &JobRecord) -> String {
    if record.total > 0 {
        format!("{} ({}/{})", record.stage, record.progress, record.total)
    } else {
        record.stage.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_result_exposes_progress() {
        let record = JobRecord {
            id: "job_test".to_owned(),
            tool: "document_repo".to_owned(),
            workspace: PathBuf::from("."),
            args: json!({}),
            state: JobState::Working,
            stage: "mapping".to_owned(),
            progress: 2,
            total: 5,
            cancel_requested: false,
            created_at: "2026-09-02T00:00:00Z".to_owned(),
            updated_at: "2026-09-02T00:00:01Z".to_owned(),
            ttl_ms: DEFAULT_TTL_MS,
            poll_interval_ms: DEFAULT_POLL_INTERVAL_MS,
            result: None,
            error: None,
            duration_ms: None,
            llm_calls: 2,
            prompt_tokens: 1200,
            completion_tokens: 300,
            last_llm_diagnostics: Some("finish_reason=stop".to_owned()),
            error_hint: None,
            symbol_index_hits: 4,
            symbol_index_misses: 1,
            evidence_cache_hits: 3,
            evidence_cache_misses: 2,
        };
        let value = task_get_result(&record);
        assert_eq!(value["status"], "working");
        assert_eq!(value["_meta"]["io.llm2mcp/progress"]["progress"], 2);
        assert_eq!(value["_meta"]["io.llm2mcp/progress"]["promptTokens"], 1200);
    }

    #[test]
    fn diagnostics_cover_common_provider_failures() {
        assert!(
            diagnostic_hint("LLM API HTTP 429: rate limit")
                .expect("429 hint")
                .contains("rate-limiting")
        );
        assert!(
            diagnostic_hint("failed to connect to LLM API")
                .expect("connect hint")
                .contains("unreachable")
        );
    }
}
