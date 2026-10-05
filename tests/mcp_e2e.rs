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
        if system.contains("synthesizing project documentation") {
            let targets = user
                .split("TARGET DOCUMENTS\n")
                .nth(1)
                .unwrap()
                .split("\n\n")
                .next()
                .unwrap();
            content = targets.lines().map(|path| format!("===== DOCUMENT: {path} =====\n# Mock documentation\n===== END DOCUMENT =====")).collect::<Vec<_>>().join("\n");
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
