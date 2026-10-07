use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
struct Http {
    url: String,
    requests: mpsc::Receiver<Value>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Http {
    fn new(mode: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let count = Arc::new(AtomicUsize::new(0));
        let (sender, requests) = mpsc::channel();
        let handle = thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let sender = sender.clone();
                        let count = count.clone();
                        thread::spawn(move || serve(stream, mode, &sender, &count));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Err(error) => panic!("mock accept: {error}"),
                }
            }
        });
        Self {
            url,
            requests,
            stop,
            thread: Some(handle),
        }
    }
}
impl Drop for Http {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}
fn serve(mut stream: TcpStream, mode: &str, sender: &mpsc::Sender<Value>, count: &AtomicUsize) {
    // Accepted sockets inherit the nonblocking listener mode on Windows.
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut first = String::new();
    reader.read_line(&mut first).unwrap();
    let mut length = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" || line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse::<usize>().unwrap();
        }
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes).unwrap();
    let listing = first.contains("/models");
    let payload = if listing {
        json!({"path":"models"})
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    let _ = sender.send(payload.clone());
    let number = count.fetch_add(1, Ordering::Relaxed);
    if mode == "slow" && !listing {
        thread::sleep(Duration::from_secs(30));
    }
    let (status, body) = if mode == "bad" && !listing {
        (401, json!({"error":"inference rejected"}))
    } else if mode == "retry" && number == 0 {
        (429, json!({"error":"temporarily busy"}))
    } else if listing {
        (200, json!({"data":[{"id":"mock"}]}))
    } else {
        let system = payload["messages"][0]["content"].as_str().unwrap_or("");
        let user = payload["messages"][1]["content"].as_str().unwrap_or("");
        let mut content = json!({"conclusion":"Mock analysis","findings":[],"risks":[],"tests":[],"actions":[],"read_next":[]}).to_string();
        if mode == "debug_evidence" && system.contains("software-debugging specialist") {
            content = json!({
                "diagnosis":"Worker connection fails", "confidence":"high",
                "root_cause":"The inference service is unavailable", "intermittency":"unknown",
                "evidence":[{"severity":"high", "text":"Observed worker failure", "evidence":[
                    "log:worker failed: connection refused", "invented.rs:999"
                ]}],
                "execution_path":[], "alternatives":[], "verification":["Check service availability"],
                "fix_area":[], "read_next":[]
            }).to_string();
        }
        if mode.starts_with("supplement") && system.contains("secondary code-analysis") {
            if number > 0 && mode == "supplement_slow" {
                thread::sleep(Duration::from_secs(30));
            }
            content = if number == 0 && mode == "supplement_text" {
                json!({"conclusion":"Need literal evidence","findings":[],"risks":[],"actions":[],"read_next":[],"context_requests":[{"query":"late_literal_marker","paths":[]}]}).to_string()
            } else if number == 0 {
                json!({"conclusion":"Need worker evidence","findings":[],"risks":[],"actions":[],"read_next":[],
                    "context_requests":[{"query":"worker_timeout_secs","paths":["worker.rs"]},
                    {"query":"PRIVATE_FILE_MUST_NOT_APPEAR","paths":["secrets.txt","../outside.rs"]}]}).to_string()
            } else {
                let evidence = if mode == "supplement_text" {
                    assert!(user.contains("late_literal_marker = 42"));
                    "data.rs:202"
                } else {
                    assert!(user.contains("fn worker_timeout_secs"));
                    "worker.rs:1"
                };
                let mut findings=(0..20).map(|index| json!({"severity":"low","text":format!("PAGE_ONLY_{index} 中文🙂"),"evidence":[evidence]})).collect::<Vec<_>>();
                findings.push(json!({"severity":"critical","text":"Worker timeout requires verification","evidence":[evidence]}));
                json!({"conclusion":"Worker inspected","findings":findings,"risks":[],"actions":[],"read_next":[],
                    "context_requests":[{"query":"more evidence","paths":["missing.rs"]}]}).to_string()
            };
        }
        if mode == "scan_tail" && system.contains("mapping one chunk") {
            content = if user.contains("scan_item_69") {
                "LATE_TAIL_EVIDENCE".to_owned()
            } else {
                "mapped earlier evidence".to_owned()
            };
        }
        if mode == "document_fragments"
            && system.contains("updating software project documentation")
        {
            let snapshots = user
                .split("DOCUMENT SNAPSHOTS\n")
                .nth(1)
                .unwrap()
                .split("\n\nDOCS_TRUNCATED\n")
                .next()
                .unwrap();
            let snapshot: Value = serde_json::from_str(snapshots.trim()).unwrap();
            assert!(
                snapshot["preview_fragments"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|fragment| fragment["text"]
                        .as_str()
                        .unwrap()
                        .contains("worker_timeout_secs = 30"))
            );
            content = json!({"edits":[{
                "path":snapshot["path"], "original_sha256":snapshot["original_sha256"],
                "old_text":"worker_timeout_secs = 30", "new_text":"worker_timeout_secs = 60"
            }], "new_documents":[]})
            .to_string();
        }
        if system.contains("synthesizing project documentation") {
            let targets = user
                .split("TARGET DOCUMENTS\n")
                .nth(1)
                .unwrap()
                .split("\n\n")
                .next()
                .unwrap();
            let body = if mode == "scan_tail" && user.contains("LATE_TAIL_EVIDENCE") {
                "LATE_TAIL_EVIDENCE"
            } else {
                "Mock documentation"
            };
            content = targets
                .lines()
                .map(|path| {
                    format!("===== DOCUMENT: {path} =====\n# {body}\n===== END DOCUMENT =====")
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
        if mode == "malformed_once" && number == 0 {
            content = "{broken".into();
        }
        let finish = if mode == "always_truncated" || (mode == "truncated_once" && number == 0) {
            "length"
        } else {
            "stop"
        };
        (
            200,
            json!({"choices":[{"message":{"content":content},"finish_reason":finish}],"usage":{"prompt_tokens":100,"completion_tokens":20}}),
        )
    };
    let body = body.to_string();
    let _ = write!(
        stream,
        "HTTP/1.1 {status} Mock\r\nContent-Type: application/json\r\nContent-Length: {}\r\nRetry-After: 0\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}
struct Fixture {
    _root: tempfile::TempDir,
    data: PathBuf,
    config: PathBuf,
    workspace: PathBuf,
}
impl Fixture {
    fn new(http: &Http, execution: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let config = root.path().join("config");
        let workspace = root.path().join("repo");
        fs::create_dir_all(&config).unwrap();
        fs::create_dir_all(&workspace).unwrap();
        fs::write(
            workspace.join("main.rs"),
            "fn login() { let password = \"FAKE_SOURCE_SECRET\"; }\n",
        )
        .unwrap();
        fs::write(config.join("config.json"), json!({"base_url":http.url,"model":"mock","api_key":"FAKE_CONFIG_SECRET","timeout_secs":10,"max_concurrent_jobs":1,"max_concurrent_requests":1,"input_usd_per_million":1.0,"output_usd_per_million":2.0,"tools":{"analyze":{"reasoning":"off","max_output_tokens":128,"execution":execution}}}).to_string()).unwrap();
        Self {
            _root: root,
            data,
            config,
            workspace,
        }
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_llm2mcp"));
        command
            .current_dir(&self.workspace)
            .env("LLM2MCP_CONFIG_DIR", &self.config)
            .env("LLM2MCP_DATA_DIR", &self.data)
            .env_remove("LLM2MCP_BASE_URL")
            .env_remove("LLM2MCP_MODEL")
            .env_remove("LLM2MCP_API_KEY");
        command
    }
    fn records(&self) -> Vec<Value> {
        let output = self.command().args(["jobs", "--json"]).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn cancel(&self, id: &str) {
        assert!(
            self.command()
                .args(["jobs", "--cancel", id])
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    fn terminal(&self, id: &str) -> Value {
        for _ in 0..60 {
            let records = self.records();
            if let Some(record) = records.into_iter().find(|record| record["id"] == id)
                && record["state"] != "working"
            {
                return record;
            }
            thread::sleep(Duration::from_millis(100));
        }
        panic!("job {id} did not terminate");
    }
}
struct Mcp {
    child: Child,
    stdin: ChildStdin,
    replies: mpsc::Receiver<Value>,
}
impl Mcp {
    fn new(fixture: &Fixture) -> Self {
        let mut child = fixture
            .command()
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, replies) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let _ = sender.send(serde_json::from_str(&line).unwrap());
            }
        });
        Self {
            child,
            stdin,
            replies,
        }
    }
    fn send(&mut self, value: Value) {
        writeln!(self.stdin, "{value}").unwrap();
        self.stdin.flush().unwrap();
    }
    fn receive(&self) -> Value {
        self.replies
            .recv_timeout(Duration::from_secs(8))
            .expect("MCP did not reply")
    }
    fn analyze(&mut self, id: i64) {
        self.send(json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"analyze","arguments":{"task":"检查登录","paths":["main.rs"]}}}));
    }
}
impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
#[test]
fn slow_http_does_not_block_ping_and_cancel_aborts_sync_job() {
    let http = Http::new("slow");
    let fixture = Fixture::new(&http, "sync");
    let mut mcp = Mcp::new(&fixture);
    mcp.analyze(10);
    http.requests.recv_timeout(Duration::from_secs(5)).unwrap();
    mcp.send(json!({"jsonrpc":"2.0","id":11,"method":"ping"}));
    let ping = mcp.replies.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(ping["id"], 11);
    mcp.send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":10}}));
    let cancelled = mcp.replies.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(cancelled["id"], 10);
    assert_eq!(cancelled["result"]["isError"], true);
    assert_eq!(fixture.records()[0]["state"], "cancelled");
}
#[test]
fn retry_privacy_profile_routing_and_cost_metrics() {
    let http = Http::new("retry");
    let fixture = Fixture::new(&http, "sync");
    let mut config: Value =
        serde_json::from_slice(&fs::read(fixture.config.join("config.json")).unwrap()).unwrap();
    config["profiles"] = json!([{"name":"analysis","base_url":http.url,"model":"routed-mock","api_key":"FAKE_PROFILE_SECRET","send_temperature":false,"completion_token_parameter":"max_completion_tokens","input_usd_per_million":1.0,"output_usd_per_million":2.0}]);
    config["tools"]["analyze"]["model_profile"] = json!("analysis");
    fs::write(fixture.config.join("config.json"), config.to_string()).unwrap();
    let mut mcp = Mcp::new(&fixture);
    mcp.analyze(1);
    let reply = mcp.receive();
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    let first = http.requests.recv_timeout(Duration::from_secs(1)).unwrap();
    let second = http.requests.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(first["model"], "routed-mock");
    assert_eq!(first["max_completion_tokens"], 128);
    assert!(first.get("max_tokens").is_none());
    assert!(first.get("temperature").is_none());
    assert_eq!(first, second);
    assert!(!first.to_string().contains("FAKE_SOURCE_SECRET"));
    let records = fixture.records();
    let record = &records[0];
    assert_eq!(record["state"], "completed");
    assert_eq!(record["prompt_tokens"], 100);
    assert_eq!(record["completion_tokens"], 20);
    assert!(record["source_tokens"].as_u64().unwrap() > 0);
    assert!(record["return_tokens"].as_u64().unwrap() > 0);
    assert_eq!(record["priced_llm_calls"], 1);
    assert!((record["estimated_llm_cost_usd"].as_f64().unwrap() - 0.00014).abs() < 1e-10);
    assert!(!record.to_string().contains("FAKE_CONFIG_SECRET"));
    assert!(!record.to_string().contains("FAKE_PROFILE_SECRET"));
}
#[test]
fn debug_runtime_citations_survive_and_invented_evidence_limits_confidence() {
    let http = Http::new("debug_evidence");
    let fixture = Fixture::new(&http, "sync");
    let mut mcp = Mcp::new(&fixture);
    mcp.send(
        json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{
            "name":"debug_issue", "arguments":{
                "issue":"Worker cannot connect", "paths":["main.rs"],
                "logs":"worker failed: connection refused"
            }
        }}),
    );
    let reply = mcp.receive();
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    let text = reply["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("log:worker failed: connection refused"));
    assert!(text.contains("CONFIDENCE\nmedium"));
    assert!(text.contains("UNVERIFIED CITATIONS REMOVED"));
    assert!(!text.contains("invented.rs"));
    assert_eq!(fixture.records()[0]["llm_calls"], 1);
}

#[test]
fn document_updates_select_late_related_fragments_and_preserve_the_original() {
    let http = Http::new("document_fragments");
    let fixture = Fixture::new(&http, "sync");
    let original = format!(
        "# Introduction\n{}\n## Worker settings\nworker_timeout_secs = 30\n\n## Appendix\nKEEP APPENDIX\n",
        "Unrelated description.\n".repeat(400)
    );
    fs::write(fixture.workspace.join("README.md"), &original).unwrap();
    fs::write(
        fixture.workspace.join("settings.rs"),
        "const WORKER_TIMEOUT_SECS: u64 = 30;\n",
    )
    .unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.name", "Test"],
        vec!["config", "user.email", "test@example.invalid"],
        vec!["add", "."],
        vec!["commit", "-qm", "fixture"],
    ] {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(&fixture.workspace)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    fs::write(
        fixture.workspace.join("settings.rs"),
        "const WORKER_TIMEOUT_SECS: u64 = 60;\n",
    )
    .unwrap();
    let mut config: Value =
        serde_json::from_slice(&fs::read(fixture.config.join("config.json")).unwrap()).unwrap();
    config["max_source_tokens"] = json!(1000);
    config["max_file_tokens"] = json!(96);
    config["tools"]["update_docs"] =
        json!({"reasoning":"off", "max_output_tokens":1000, "execution":"sync"});
    fs::write(fixture.config.join("config.json"), config.to_string()).unwrap();
    let mut mcp = Mcp::new(&fixture);
    mcp.send(
        json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{
            "name":"update_docs", "arguments":{"docs":["README.md"]}
        }}),
    );
    let reply = mcp.receive();
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    let updates: Value =
        serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(updates["format"], "llm2mcp-document-edits-v1");
    assert_eq!(updates["source_preview_truncated"], true);
    assert_eq!(updates["edits"][0]["old_text"], "worker_timeout_secs = 30");
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("README.md")).unwrap(),
        original
    );
    assert_eq!(fixture.records()[0]["llm_calls"], 1);
}

#[test]
fn doctor_rejects_models_only_connectivity() {
    let http = Http::new("bad");
    let fixture = Fixture::new(&http, "sync");
    let output = fixture
        .command()
        .args(["doctor", "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["checks"][0]["ok"], false);
    assert_eq!(report["checks"][1]["ok"], true);
}
#[test]
fn worker_crash_recovery_and_retry() {
    let http = Http::new("slow");
    let fixture = Fixture::new(&http, "async");
    let mut mcp = Mcp::new(&fixture);
    mcp.analyze(1);
    let started = mcp.receive();
    assert_eq!(started["result"]["isError"], false, "{started}");
    http.requests.recv_timeout(Duration::from_secs(5)).unwrap();
    let records = fixture.records();
    let record = &records[0];
    let pid = record["worker_pid"].as_u64().unwrap();
    #[cfg(unix)]
    assert!(
        Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status()
            .unwrap()
            .success()
    );
    #[cfg(windows)]
    assert!(
        Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .status()
            .unwrap()
            .success()
    );
    thread::sleep(Duration::from_millis(100));
    let id = record["id"].as_str().unwrap();
    let path = fixture.data.join("jobs").join(format!("{id}.json"));
    let mut state: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    state["heartbeat_at"] = json!("2000-01-01T00:00:00Z");
    fs::write(path, state.to_string()).unwrap();
    let records = fixture.records();
    assert_eq!(records[0]["state"], "failed");
    assert_eq!(records[0]["stage"], "worker_lost");
    let retry_started = std::time::Instant::now();
    let output = fixture
        .command()
        .args(["jobs", "--retry", id])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        retry_started.elapsed() < Duration::from_secs(5),
        "retry CLI waited for the background worker"
    );
    let retried: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_ne!(retried["id"], id);
    let id = retried["id"].as_str().unwrap();
    fixture.cancel(id);
    assert_eq!(fixture.terminal(id)["state"], "cancelled");
}
#[test]
fn scheduler_queues_jobs_and_releases_slots_on_cancellation() {
    let http = Http::new("slow");
    let fixture = Fixture::new(&http, "async");
    let mut mcp = Mcp::new(&fixture);
    mcp.analyze(1);
    let first = mcp.receive();
    http.requests.recv_timeout(Duration::from_secs(5)).unwrap();
    mcp.analyze(2);
    let second = mcp.receive();
    assert!(
        http.requests
            .recv_timeout(Duration::from_millis(300))
            .is_err(),
        "second worker ignored concurrency cap"
    );
    let first = first["result"]["structuredContent"]["job_id"]
        .as_str()
        .unwrap();
    let second = second["result"]["structuredContent"]["job_id"]
        .as_str()
        .unwrap();
    fixture.cancel(first);
    assert_eq!(fixture.terminal(first)["state"], "cancelled");
    http.requests.recv_timeout(Duration::from_secs(3)).unwrap();
    fixture.cancel(second);
    assert_eq!(fixture.terminal(second)["state"], "cancelled");
}
#[test]
fn cache_cleanup_leaves_job_history_intact() {
    let http = Http::new("ok");
    let fixture = Fixture::new(&http, "sync");
    let cache = fixture.data.join("repo-map-cache/old");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("test.md"), "old summary").unwrap();
    fs::create_dir_all(fixture.data.join("jobs")).unwrap();
    fs::write(fixture.data.join("jobs/keep.log"), "keep").unwrap();
    let output = fixture
        .command()
        .args(["cache", "--clear"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stats: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(stats["removed"], 1);
    assert!(fixture.data.join("jobs/keep.log").exists());
}

#[test]
fn map_reduce_profiles_and_prefix_cache_invalidation() {
    let http = Http::new("ok");
    let fixture = Fixture::new(&http, "sync");
    let mut config: Value =
        serde_json::from_slice(&fs::read(fixture.config.join("config.json")).unwrap()).unwrap();
    config["profiles"] = json!([{"name":"map","base_url":http.url,"model":"map-mock"}, {"name":"reduce","base_url":http.url,"model":"reduce-mock"}]);
    config["map_profile"] = json!("map");
    config["tools"]["document_repo"] = json!({"reasoning":"off","max_output_tokens":1000,"model_profile":"reduce","execution":"async"});
    fs::write(fixture.config.join("config.json"), config.to_string()).unwrap();
    let mut mcp = Mcp::new(&fixture);
    let run = |mcp: &mut Mcp, id| {
        mcp.send(json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"document_repo","arguments":{"paths":["main.rs"]}}}));
        let response = mcp.receive();
        let id = response["result"]["structuredContent"]["job_id"]
            .as_str()
            .unwrap();
        fixture.terminal(id)
    };
    assert_eq!(run(&mut mcp, 1)["state"], "completed");
    assert_eq!(
        http.requests.recv_timeout(Duration::from_secs(1)).unwrap()["model"],
        "map-mock"
    );
    assert_eq!(
        http.requests.recv_timeout(Duration::from_secs(1)).unwrap()["model"],
        "reduce-mock"
    );
    let warm = run(&mut mcp, 2);
    assert_eq!(warm["state"], "completed");
    assert_eq!(warm["llm_calls"], 1);
    assert_eq!(
        http.requests.recv_timeout(Duration::from_secs(1)).unwrap()["model"],
        "reduce-mock"
    );
    assert!(http.requests.try_recv().is_err());
    config["system_prompt_prefix"] = json!("Team rules changed");
    fs::write(fixture.config.join("config.json"), config.to_string()).unwrap();
    let refreshed = run(&mut mcp, 3);
    assert_eq!(refreshed["state"], "completed");
    assert_eq!(refreshed["llm_calls"], 2);
    let map = http.requests.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(map["model"], "map-mock");
    assert!(
        map["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("Team rules changed")
    );
    assert_eq!(
        http.requests.recv_timeout(Duration::from_secs(1)).unwrap()["model"],
        "reduce-mock"
    );
}
#[test]
fn oversized_context_is_rejected_before_http() {
    let http = Http::new("ok");
    let fixture = Fixture::new(&http, "sync");
    let mut config: Value =
        serde_json::from_slice(&fs::read(fixture.config.join("config.json")).unwrap()).unwrap();
    config["model_input_limit"] = json!(64);
    fs::write(fixture.config.join("config.json"), config.to_string()).unwrap();
    let mut mcp = Mcp::new(&fixture);
    mcp.analyze(1);
    let response = mcp.receive();
    assert_eq!(response["result"]["isError"], true);
    assert!(
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("exceeds model context limit")
    );
    assert!(
        http.requests
            .recv_timeout(Duration::from_millis(200))
            .is_err()
    );
    assert_eq!(fixture.records()[0]["state"], "failed");
}

#[test]
fn cache_age_and_size_prune_oldest_entries() {
    use std::time::SystemTime;
    let http = Http::new("ok");
    let fixture = Fixture::new(&http, "sync");
    let mut config: Value =
        serde_json::from_slice(&fs::read(fixture.config.join("config.json")).unwrap()).unwrap();
    config["cache_max_mib"] = json!(16);
    config["cache_ttl_days"] = json!(1);
    fs::write(fixture.config.join("config.json"), config.to_string()).unwrap();
    let cache = fixture.data.join("repo-map-cache/test");
    fs::create_dir_all(&cache).unwrap();
    for (name, size, age) in [
        ("expired.md", 1, 2 * 86400),
        ("older.md", 10 * 1024 * 1024, 3600),
        ("newest.md", 10 * 1024 * 1024, 0),
    ] {
        let file = fs::File::create(cache.join(name)).unwrap();
        file.set_len(size).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(age))
            .unwrap();
    }
    let output = fixture.command().arg("cache").output().unwrap();
    assert!(output.status.success());
    let stats: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(stats["removed"], 2);
    assert_eq!(stats["files"], 1);
    assert_eq!(stats["bytes"], 10 * 1024 * 1024);
    assert!(cache.join("newest.md").exists());
    assert!(!cache.join("expired.md").exists());
    assert!(!cache.join("older.md").exists());
}

#[test]
fn incomplete_and_malformed_outputs_recover_with_bounded_attempts() {
    for (mode, success, calls) in [
        ("truncated_once", true, 2),
        ("malformed_once", true, 2),
        ("always_truncated", false, 3),
    ] {
        let http = Http::new(mode);
        let fixture = Fixture::new(&http, "sync");
        let mut mcp = Mcp::new(&fixture);
        mcp.analyze(1);
        let reply = mcp.receive();
        assert_eq!(reply["result"]["isError"], !success, "{mode}: {reply}");
        for _ in 0..calls {
            http.requests.recv_timeout(Duration::from_secs(1)).unwrap();
        }
        assert!(http.requests.try_recv().is_err());
        let record = &fixture.records()[0];
        assert_eq!(
            record["state"],
            if success { "completed" } else { "failed" }
        );
        assert_eq!(record["prompt_tokens"], calls * 100);
    }
}

#[test]
fn small_routed_model_receives_bounded_source_and_output() {
    let http = Http::new("ok");
    let fixture = Fixture::new(&http, "sync");
    fs::write(
        fixture.workspace.join("main.rs"),
        "fn item() {}\n".repeat(10000),
    )
    .unwrap();
    let mut config: Value =
        serde_json::from_slice(&fs::read(fixture.config.join("config.json")).unwrap()).unwrap();
    config["profiles"] = json!([{"name":"small","base_url":http.url,"model":"small","max_input_tokens":4096,"max_output_tokens":512}]);
    config["tools"]["analyze"]["model_profile"] = json!("small");
    fs::write(fixture.config.join("config.json"), config.to_string()).unwrap();
    let mut mcp = Mcp::new(&fixture);
    mcp.analyze(1);
    let reply = mcp.receive();
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    let request = http.requests.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(request["model"], "small");
    assert!(request["messages"][1]["content"].as_str().unwrap().len() < 10000);
    assert!(fixture.records()[0]["source_truncated"].as_bool().unwrap());
}

fn tasks_meta() -> Value {
    json!({"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{"extensions":{"io.modelcontextprotocol/tasks":{}}}})
}

#[test]
fn modern_tasks_complete_after_reconnect_and_match_portable_result() {
    let http = Http::new("ok");
    let fixture = Fixture::new(&http, "async");
    let mut mcp = Mcp::new(&fixture);
    mcp.send(
        json!({"jsonrpc":"2.0","id":1,"method":"server/discover","params":{"_meta":tasks_meta()}}),
    );
    let discovery = mcp.receive();
    assert!(
        discovery["result"]["capabilities"]["extensions"]["io.modelcontextprotocol/tasks"]
            .is_object()
    );
    mcp.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"_meta":tasks_meta(),"name":"analyze","arguments":{"task":"explain main","paths":["main.rs"]}}}));
    let reply = mcp.receive();
    assert_eq!(reply["result"]["resultType"], "task", "{reply}");
    let id = reply["result"]["taskId"].as_str().unwrap().to_owned();
    drop(mcp);
    assert_eq!(fixture.terminal(&id)["state"], "completed");
    let mut reconnected = Mcp::new(&fixture);
    reconnected.send(json!({"jsonrpc":"2.0","id":3,"method":"tasks/get","params":{"_meta":tasks_meta(),"taskId":id}}));
    let task = reconnected.receive();
    assert_eq!(task["result"]["status"], "completed");
    reconnected.send(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"job_result","arguments":{"job_id":id}}}));
    assert_eq!(reconnected.receive()["result"], task["result"]["result"]);
}

#[test]
fn modern_tasks_cancel_active_http_and_reject_missing_capability() {
    let http = Http::new("slow");
    let fixture = Fixture::new(&http, "async");
    let mut mcp = Mcp::new(&fixture);
    mcp.send(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"_meta":tasks_meta(),"name":"analyze","arguments":{"task":"explain main","paths":["main.rs"]}}}));
    let started = mcp.receive();
    let id = started["result"]["taskId"].as_str().unwrap().to_owned();
    http.requests.recv_timeout(Duration::from_secs(5)).unwrap();
    mcp.send(json!({"jsonrpc":"2.0","id":2,"method":"tasks/get","params":{"taskId":id}}));
    assert_eq!(mcp.receive()["error"]["code"], -32003);
    mcp.send(json!({"jsonrpc":"2.0","id":3,"method":"tasks/cancel","params":{"_meta":tasks_meta(),"taskId":id}}));
    assert!(mcp.receive().get("error").is_none());
    assert_eq!(fixture.terminal(&id)["state"], "cancelled");
}

#[test]
fn deep_doctor_verifies_real_analysis_and_durable_lifecycle() {
    let http = Http::new("ok");
    let fixture = Fixture::new(&http, "sync");
    let output = fixture
        .command()
        .args([
            "doctor",
            "--json",
            "--deep",
            "--workspace",
            fixture.workspace.to_str().unwrap(),
            "--path",
            "main.rs",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| check["name"] == "Deep MCP lifecycle" && check["ok"] == true)
    );
    assert!(
        fixture
            .records()
            .iter()
            .any(|record| record["state"] == "completed" && record["llm_calls"] == 1)
    );
    assert!(
        fixture
            .records()
            .iter()
            .any(|record| record["state"] == "cancelled" && record["llm_calls"] == 0)
    );
}

fn call(mcp: &mut Mcp, id: i64, name: &str, arguments: Value) -> Value {
    mcp.send(json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":arguments}}));
    let reply = mcp.receive();
    assert_eq!(reply["id"], id);
    reply
}
fn complete_pages(mcp: &mut Mcp, job: &str) -> String {
    let mut cursor = None;
    let mut output = String::new();
    let mut pages = 0;
    loop {
        let mut args = json!({"job_id":job,"page_tokens":512});
        if let Some(cursor) = &cursor {
            args["cursor"] = json!(cursor);
        }
        let reply = call(mcp, 900, "result_page", args);
        assert!(reply.get("error").is_none(), "{reply}");
        let page: Value =
            serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(page["offset"].as_u64().unwrap() as usize, output.len());
        output.push_str(page["text"].as_str().unwrap());
        pages += 1;
        cursor = page["next_cursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            assert_eq!(page["complete"], true);
            break;
        }
        assert!(pages < 200);
    }
    output
}
fn supplemental_fixture(http: &Http, execution: &str) -> Fixture {
    let fixture = Fixture::new(http, execution);
    fs::write(
        fixture.workspace.join("worker.rs"),
        "fn worker_timeout_secs() -> u64 { 30 }\n",
    )
    .unwrap();
    fs::write(
        fixture.workspace.join("secrets.txt"),
        "PRIVATE_FILE_MUST_NOT_APPEAR\n",
    )
    .unwrap();
    let mut config: Value =
        serde_json::from_slice(&fs::read(fixture.config.join("config.json")).unwrap()).unwrap();
    config["tools"]["analyze"]["max_output_tokens"] = json!(4096);
    config["tools"]["analyze"]["primary_return_tokens"] = json!(128);
    fs::write(fixture.config.join("config.json"), config.to_string()).unwrap();
    fixture
}
#[test]
fn supplemental_retrieval_is_bounded_and_complete_result_survives_reconnect() {
    let http = Http::new("supplement");
    let fixture = supplemental_fixture(&http, "sync");
    let mut mcp = Mcp::new(&fixture);
    let reply = call(
        &mut mcp,
        1,
        "analyze",
        json!({"task":"explain main","paths":["main.rs"]}),
    );
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    let job = reply["result"]["_meta"]["llm2mcp/job_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("FULL_RESULT")
    );
    assert_eq!(fixture.terminal(&job)["llm_calls"], 2);
    let first = http.requests.recv_timeout(Duration::from_secs(1)).unwrap();
    let second = http.requests.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(!first.to_string().contains("fn worker_timeout_secs"));
    assert!(second.to_string().contains("fn worker_timeout_secs"));
    assert!(!second.to_string().contains("PRIVATE_FILE_MUST_NOT_APPEAR"));
    assert!(!second.to_string().contains("FAKE_SOURCE_SECRET"));
    drop(mcp);
    let mut reconnected = Mcp::new(&fixture);
    let full = complete_pages(&mut reconnected, &job);
    let value: Value = serde_json::from_str(&full).unwrap();
    assert_eq!(value["findings"].as_array().unwrap().len(), 21);
    assert!(full.contains("PAGE_ONLY_19"));
    assert!(full.contains("one-round limit"));
    assert_eq!(fixture.records()[0]["llm_calls"], 2);
    assert!(http.requests.try_recv().is_err());
}
#[test]
fn disabled_or_filtered_supplement_never_starts_another_analysis() {
    for args in [
        json!({"allow_supplement":false}),
        json!({"exclude":["worker.rs"]}),
    ] {
        let http = Http::new("supplement");
        let fixture = supplemental_fixture(&http, "sync");
        let mut mcp = Mcp::new(&fixture);
        let mut arguments = json!({"task":"explain main","paths":["main.rs"]});
        for (key, value) in args.as_object().unwrap() {
            arguments[key] = value.clone();
        }
        let reply = call(&mut mcp, 1, "analyze", arguments);
        assert_eq!(reply["result"]["isError"], false, "{reply}");
        assert_eq!(fixture.records()[0]["llm_calls"], 1);
    }
}
#[test]
fn cancellation_interrupts_the_supplemental_model_call() {
    let http = Http::new("supplement_slow");
    let fixture = supplemental_fixture(&http, "sync");
    let mut mcp = Mcp::new(&fixture);
    mcp.analyze(1);
    for _ in 0..2 {
        http.requests.recv_timeout(Duration::from_secs(5)).unwrap();
    }
    mcp.send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}));
    let reply = mcp.replies.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(reply["result"]["isError"], true);
    assert_eq!(fixture.records()[0]["state"], "cancelled");
}
#[test]
fn async_pages_reject_foreign_workspaces_changed_artifacts_and_expiration() {
    let http = Http::new("ok");
    let mut fixture = Fixture::new(&http, "async");
    let mut mcp = Mcp::new(&fixture);
    mcp.analyze(1);
    let reply = mcp.receive();
    let job = reply["result"]["structuredContent"]["job_id"]
        .as_str()
        .unwrap()
        .to_owned();
    drop(mcp);
    let record = fixture.terminal(&job);
    assert_eq!(record["state"], "completed");
    let mut reconnected = Mcp::new(&fixture);
    assert!(complete_pages(&mut reconnected, &job).contains("Mock analysis"));
    for arguments in [
        json!({"job_id":job,"page_tokens":-1}),
        json!({"job_id":job,"page_tokens":128}),
        json!({"job_id":job,"cursor":123}),
    ] {
        assert!(
            call(&mut reconnected, 20, "result_page", arguments)
                .get("error")
                .is_some()
        );
    }
    assert!(
        call(&mut reconnected, 21, "continue_scan", json!({}))
            .get("error")
            .is_some()
    );
    let bad_cursor = call(
        &mut reconnected,
        2,
        "result_page",
        json!({"job_id":job,"cursor":"bad"}),
    );
    assert!(bad_cursor.get("error").is_some());
    drop(reconnected);
    let original = fixture.workspace.clone();
    fixture.workspace = fixture._root.path().join("other-repo");
    fs::create_dir(&fixture.workspace).unwrap();
    let mut foreign = Mcp::new(&fixture);
    assert!(
        call(&mut foreign, 3, "result_page", json!({"job_id":job}))["error"]["message"]
            .as_str()
            .unwrap()
            .contains("different workspace")
    );
    drop(foreign);
    fixture.workspace = original;
    let artifact = fixture.data.join("jobs").join(format!("{job}.full.txt"));
    fs::write(&artifact, "changed").unwrap();
    let unfinished = artifact.with_extension("tmp");
    fs::write(&unfinished, "abandoned artifact write").unwrap();
    let mut mcp = Mcp::new(&fixture);
    assert!(
        call(&mut mcp, 4, "result_page", json!({"job_id":job}))["error"]["message"]
            .as_str()
            .unwrap()
            .contains("changed")
    );
    let path = fixture.data.join("jobs").join(format!("{job}.json"));
    let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record["created_at"] = json!("2020-01-01T00:00:00Z");
    fs::write(path, record.to_string()).unwrap();
    assert!(
        call(&mut mcp, 5, "result_page", json!({"job_id":job}))["error"]["message"]
            .as_str()
            .unwrap()
            .contains("expired")
    );
    drop(mcp);
    let mut cleaned = Mcp::new(&fixture);
    cleaned.send(json!({"jsonrpc":"2.0","id":6,"method":"ping"}));
    cleaned.receive();
    assert!(!artifact.exists());
    assert!(!unfinished.exists());
    assert!(
        !fixture
            .data
            .join("jobs")
            .join(format!("{job}.json"))
            .exists()
    );
}
fn continuation(reply: &Value) -> Value {
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    let text = reply["result"]["content"][0]["text"].as_str().unwrap();
    let metadata = text
        .split("SCAN_CONTINUATION\n")
        .nth(1)
        .unwrap()
        .split('\n')
        .next()
        .unwrap();
    serde_json::from_str(metadata).unwrap()
}
fn scan_fixture(http: &Http) -> Fixture {
    let fixture = Fixture::new(http, "sync");
    let text = (0..70)
        .map(|index| format!("fn scan_item_{index}() {{}}\n"))
        .collect::<String>();
    fs::write(fixture.workspace.join("large.rs"), text).unwrap();
    let mut config: Value =
        serde_json::from_slice(&fs::read(fixture.config.join("config.json")).unwrap()).unwrap();
    config["max_source_tokens"] = json!(512);
    config["max_file_tokens"] = json!(180);
    config["tools"]["document_repo"] =
        json!({"reasoning":"off","max_output_tokens":2000,"execution":"sync"});
    fs::write(fixture.config.join("config.json"), config.to_string()).unwrap();
    fixture
}
#[test]
fn repository_scan_continues_after_reconnect_and_synthesizes_accumulated_evidence() {
    let http = Http::new("scan_tail");
    let fixture = scan_fixture(&http);
    let mut mcp = Mcp::new(&fixture);
    let first = call(
        &mut mcp,
        1,
        "document_repo",
        json!({"paths":["large.rs"],"max_chunks":1}),
    );
    let mut metadata = continuation(&first);
    assert_eq!(metadata["complete"], false);
    let first_cursor = metadata["scan_cursor"].as_str().unwrap().to_owned();
    drop(mcp);
    let mut mcp = Mcp::new(&fixture);
    let replay = call(
        &mut mcp,
        2,
        "continue_scan",
        json!({"scan_cursor":first_cursor}),
    );
    let replay_metadata = continuation(&replay);
    let same = call(
        &mut mcp,
        3,
        "continue_scan",
        json!({"scan_cursor":first_cursor}),
    );
    assert_eq!(continuation(&same), replay_metadata);
    metadata = replay_metadata;
    let mut pages = 2;
    let mut final_reply = replay;
    while !metadata["complete"].as_bool().unwrap() {
        let cursor = metadata["scan_cursor"].as_str().unwrap();
        final_reply = call(
            &mut mcp,
            10 + pages,
            "continue_scan",
            json!({"scan_cursor":cursor}),
        );
        metadata = continuation(&final_reply);
        pages += 1;
        assert!(pages < 30);
    }
    assert!(pages > 2);
    assert_eq!(metadata["files_complete"], 1);
    assert!(metadata["scan_cursor"].is_null());
    assert!(
        final_reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("LATE_TAIL_EVIDENCE")
    );
    let requests = http.requests.try_iter().collect::<Vec<_>>();
    let maps = requests
        .iter()
        .filter(|request| {
            request["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("mapping one chunk")
        })
        .collect::<Vec<_>>();
    let joined = maps
        .iter()
        .map(|request| request["messages"][1]["content"].as_str().unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    for index in 0..70 {
        assert_eq!(
            joined.matches(&format!("fn scan_item_{index}() ")).count(),
            1
        );
    }
    let job = final_reply["result"]["_meta"]["llm2mcp/job_id"]
        .as_str()
        .unwrap();
    assert!(complete_pages(&mut mcp, job).contains("LATE_TAIL_EVIDENCE"));
}
#[test]
fn scan_continuations_reject_changed_sources_options_profiles_and_foreign_workspaces() {
    let http = Http::new("scan_tail");
    let mut fixture = scan_fixture(&http);
    let mut mcp = Mcp::new(&fixture);
    let first = call(
        &mut mcp,
        1,
        "document_repo",
        json!({"paths":["large.rs"],"max_chunks":1}),
    );
    let metadata = continuation(&first);
    let cursor = metadata["scan_cursor"].as_str().unwrap();
    let changed_opts = call(
        &mut mcp,
        2,
        "document_repo",
        json!({"scan_cursor":cursor,"paths":["main.rs"]}),
    );
    assert!(
        changed_opts["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("options changed")
    );
    let mut config: Value =
        serde_json::from_slice(&fs::read(fixture.config.join("config.json")).unwrap()).unwrap();
    config["system_prompt_prefix"] = json!("changed rules");
    fs::write(fixture.config.join("config.json"), config.to_string()).unwrap();
    let profile = call(&mut mcp, 3, "continue_scan", json!({"scan_cursor":cursor}));
    assert!(
        profile["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("layout changed")
    );
    config["system_prompt_prefix"] = json!("");
    fs::write(fixture.config.join("config.json"), config.to_string()).unwrap();
    fs::write(fixture.workspace.join("large.rs"), "changed").unwrap();
    let changed = call(&mut mcp, 4, "continue_scan", json!({"scan_cursor":cursor}));
    assert!(
        changed["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("repository changed")
    );
    drop(mcp);
    fixture.workspace = fixture._root.path().join("foreign");
    fs::create_dir(&fixture.workspace).unwrap();
    let mut foreign = Mcp::new(&fixture);
    let reply = call(
        &mut foreign,
        5,
        "continue_scan",
        json!({"scan_cursor":cursor}),
    );
    assert!(
        reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("different workspace")
    );
    assert_eq!(http.requests.try_iter().count(), 2);
}

#[test]
fn supplemental_literal_search_reaches_evidence_beyond_the_file_prefix() {
    let http = Http::new("supplement_text");
    let fixture = supplemental_fixture(&http, "sync");
    let text = format!(
        "fn unrelated_factory() {{\n{}let late_literal_marker = 42;\n}}\n",
        "let unrelated = 1;\n".repeat(200)
    );
    fs::write(fixture.workspace.join("data.rs"), text).unwrap();
    let mut config: Value =
        serde_json::from_slice(&fs::read(fixture.config.join("config.json")).unwrap()).unwrap();
    config["max_source_tokens"] = json!(1000);
    config["max_file_tokens"] = json!(128);
    fs::write(fixture.config.join("config.json"), config.to_string()).unwrap();
    let mut mcp = Mcp::new(&fixture);
    let reply = call(
        &mut mcp,
        1,
        "analyze",
        json!({"task":"explain main","paths":["main.rs"]}),
    );
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    assert_eq!(fixture.records()[0]["llm_calls"], 2);
    let job = reply["result"]["_meta"]["llm2mcp/job_id"].as_str().unwrap();
    let full = complete_pages(&mut mcp, job);
    assert!(full.contains("data.rs:202"));
    assert!(!full.contains("UNVERIFIED"));
}

#[test]
fn small_routed_model_continuations_keep_the_original_request_budget() {
    let http = Http::new("scan_tail");
    let fixture = scan_fixture(&http);
    let text = (0..300)
        .map(|index| format!("fn scan_item_{index}() {{}}\n"))
        .collect::<String>();
    fs::write(fixture.workspace.join("large.rs"), text).unwrap();
    let mut config: Value =
        serde_json::from_slice(&fs::read(fixture.config.join("config.json")).unwrap()).unwrap();
    config["max_source_tokens"] = json!(40000);
    config["max_file_tokens"] = json!(300);
    config["profiles"] = json!([{"name":"small","base_url":http.url,"model":"small","max_input_tokens":4096,"max_output_tokens":512}]);
    config["tools"]["document_repo"]["model_profile"] = json!("small");
    config["map_profile"] = json!("small");
    fs::write(fixture.config.join("config.json"), config.to_string()).unwrap();
    let mut mcp = Mcp::new(&fixture);
    let first = call(
        &mut mcp,
        1,
        "document_repo",
        json!({"paths":["large.rs"],"max_chunks":1,"audience":"developer ".repeat(80)}),
    );
    let mut metadata = continuation(&first);
    assert_eq!(metadata["complete"], false);
    let mut pages = 1;
    while !metadata["complete"].as_bool().unwrap() {
        let reply = call(
            &mut mcp,
            2,
            "continue_scan",
            json!({"scan_cursor":metadata["scan_cursor"]}),
        );
        metadata = continuation(&reply);
        pages += 1;
        assert!(pages < 30);
    }
    assert_eq!(metadata["files_complete"], 1);
    assert!(pages > 1);
}
