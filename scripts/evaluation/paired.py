#!/usr/bin/env python3
"""Opt-in paired measurement. No token estimate is reported as provider usage."""
import argparse
import hashlib
import json
import os
import queue
import subprocess
import threading
import time
import urllib.request
from pathlib import Path


class MCP:
    def __init__(self, executable, workspace, timeout):
        self.process = subprocess.Popen([executable, "mcp", "--workspace", workspace], stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        self.replies = queue.Queue()
        self.timeout = timeout
        self.sequence = 0
        threading.Thread(target=self.read, daemon=True).start()
        self.call("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "paired-evaluation", "version": "1"}})

    def read(self):
        for line in self.process.stdout:
            try:
                self.replies.put(json.loads(line))
            except json.JSONDecodeError:
                pass
        self.replies.put(None)

    def call(self, method, params):
        self.sequence += 1
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.sequence, "method": method, "params": params}) + "\n")
        self.process.stdin.flush()
        deadline = time.monotonic() + self.timeout
        while True:
            reply = self.replies.get(timeout=max(0.01, deadline-time.monotonic()))
            if reply is None:
                raise RuntimeError("MCP exited")
            if reply.get("id") == self.sequence:
                if "error" in reply:
                    raise RuntimeError("MCP RPC failed")
                return reply["result"]

    def tool(self, name, arguments):
        return self.call("tools/call", {"name": name, "arguments": arguments})

    def close(self):
        self.process.terminate()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()


def text(result):
    if result.get("isError"):
        raise RuntimeError("secondary tool failed")
    return "\n".join(item.get("text", "") for item in result.get("content", []) if item.get("type") == "text")


def primary(args, task, context):
    prompt = {"task": task, "context": context}
    if args.primary_command:
        result = subprocess.run(args.primary_command, input=json.dumps(prompt), text=True, capture_output=True,
                                timeout=args.timeout, check=True)
        result = json.loads(result.stdout)
        return result["answer"], result.get("usage")
    payload = {"model": args.primary_model, "messages": [
        {"role": "system", "content": "Answer the software task using supplied context. Cite evidence and identify uncertainty."},
        {"role": "user", "content": json.dumps(prompt)}], "max_tokens": args.max_output, "stream": False}
    headers = {"Content-Type": "application/json"}
    key = os.environ.get(args.primary_key_env)
    if key:
        headers["Authorization"] = "Bearer " + key
    request = urllib.request.Request(args.primary_url.rstrip("/") + "/chat/completions",
                                     json.dumps(payload).encode(), headers)
    with urllib.request.urlopen(request, timeout=args.timeout) as response:
        result = json.load(response)
    choice = result["choices"][0]
    if choice.get("finish_reason") in ("length", "content_filter"):
        raise RuntimeError("primary output incomplete")
    answer = choice["message"]["content"]
    if not answer or not answer.strip():
        raise RuntimeError("primary output empty")
    return answer, result.get("usage")


def actual_usage(usage):
    if not isinstance(usage, dict):
        return None
    fields = [usage.get("prompt_tokens"), usage.get("completion_tokens")]
    if not all(isinstance(value, int) and not isinstance(value, bool) and value >= 0 for value in fields):
        return None
    return dict(zip(("prompt_tokens", "completion_tokens"), fields))


def records(executable):
    result = subprocess.run([executable, "jobs", "--json"], capture_output=True, text=True, check=True, timeout=15)
    return json.loads(result.stdout)


def secondary(args, case):
    before = {job["id"] for job in records(args.executable)}
    mcp = MCP(args.executable, str(Path(case["workspace"]).resolve()), args.timeout)
    job_id = None
    try:
        result = mcp.tool("analyze", {"task": case["task"], "paths": case["paths"]})
        job_id = result.get("structuredContent", {}).get("job_id")
        if job_id:
            deadline = time.monotonic() + args.timeout
            while True:
                status = mcp.tool("job_status", {"job_id": job_id})["structuredContent"]["status"]
                if status != "working":
                    result = mcp.tool("job_result", {"job_id": job_id})
                    break
                if time.monotonic() > deadline:
                    raise TimeoutError("secondary job timeout")
                time.sleep(0.1)
        answer = text(result)
        new_jobs = [job for job in records(args.executable) if job["id"] not in before]
        metrics = [{key: job.get(key) for key in ("id", "state", "llm_calls", "usage_reported_calls", "prompt_tokens", "completion_tokens",
                   "duration_ms", "llm_wait_ms", "used_models", "source_tokens", "return_tokens", "source_truncated",
                   "symbol_index_hits", "evidence_cache_hits", "last_llm_diagnostics")} for job in new_jobs]
        return answer, metrics
    except Exception:
        if job_id:
            try:
                mcp.tool("job_cancel", {"job_id": job_id})
            except Exception:
                pass
        raise
    finally:
        mcp.close()


def fingerprint(case):
    root = Path(case["workspace"]).resolve()
    hashes = {}
    for relative in case["paths"]:
        target = (root / relative).resolve()
        if not target.is_relative_to(root):
            raise ValueError("evaluation path outside workspace")
        files = sorted(target.rglob("*")) if target.is_dir() else [target]
        for file in files:
            if file.is_file() and file.resolve().is_relative_to(root):
                if len(hashes) >= 10000 or file.stat().st_size > 8*1024*1024:
                    raise ValueError("evaluation evidence exceeds snapshot limit")
                hashes[str(file.relative_to(root))] = hashlib.sha256(file.read_bytes()).hexdigest()
    head = subprocess.run(["git", "rev-parse", "HEAD"], cwd=root, capture_output=True, text=True)
    return {"git_commit": head.stdout.strip() if head.returncode == 0 else None,
            "evidence_sha256": hashlib.sha256(json.dumps(hashes, sort_keys=True).encode()).hexdigest()}


def report(rows, scores):
    pairs = {}
    for row in rows:
        pairs.setdefault((row["case_id"], row["repeat"]), {})[row["variant"]] = row
    results = []
    for (case, repeat), variants in pairs.items():
        baseline, offload = variants.get("baseline", {}), variants.get("offload", {})
        measured = all(row.get("status") == "ok" and row.get("primary_usage") for row in (baseline, offload))
        measured = measured and baseline.get("workspace_snapshot") == offload.get("workspace_snapshot")
        score_key = f"{case}:{repeat}"
        score = scores.get(score_key, {})
        scored = all(isinstance(score.get(key), (int, float)) and not isinstance(score.get(key), bool)
                     and 0 <= score[key] <= 1 for key in ("baseline", "offload"))
        saved = None
        if measured:
            saved = sum(baseline["primary_usage"].values()) - sum(offload["primary_usage"].values())
        results.append({"case_id": case, "repeat": repeat, "primary_tokens_saved": saved,
                        "correctness_baseline": score.get("baseline") if scored else None,
                        "correctness_offload": score.get("offload") if scored else None,
                        "validated_saving": measured and scored and score["offload"] >= score["baseline"] and saved > 0,
                        "elapsed_ms_delta": offload.get("elapsed_ms", 0)-baseline.get("elapsed_ms", 0) if measured else None})
    return {"pairs": results, "validated_pairs": sum(row["validated_saving"] for row in results),
            "note": "Primary savings exclude secondary tokens/cost. Missing usage or external scores cannot establish effectiveness."}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cases", type=Path)
    parser.add_argument("--executable", default="llm2mcp")
    parser.add_argument("--primary-url", default=os.environ.get("PRIMARY_BASE_URL"))
    parser.add_argument("--primary-model", default=os.environ.get("PRIMARY_MODEL"))
    parser.add_argument("--primary-key-env", default="PRIMARY_API_KEY")
    parser.add_argument("--primary-command", nargs="+", help="JSON stdin adapter; answer + aggregate provider usage on stdout")
    parser.add_argument("--max-output", type=int, default=2048)
    parser.add_argument("--timeout", type=int, default=600)
    parser.add_argument("--repeats", type=int, default=2)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--run", action="store_true", help="Explicitly execute requests to configured models")
    parser.add_argument("--report", type=Path, help="Summarize existing JSONL results")
    parser.add_argument("--scores", type=Path, help="External scores keyed by case_id:repeat")
    args = parser.parse_args()
    if args.report:
        rows = [json.loads(line) for line in args.report.read_text().splitlines() if line.strip()]
        print(json.dumps(report(rows, json.loads(args.scores.read_text()) if args.scores else {}), indent=2))
        return
    if not args.cases:
        parser.error("--cases is required")
    cases = json.loads(args.cases.read_text())
    ids = set()
    for case in cases:
        for key in ("id", "task", "workspace", "paths", "baseline_context"):
            if key not in case:
                parser.error("case missing " + key)
        if case["id"] in ids:
            parser.error("duplicate case id")
        ids.add(case["id"])
    if not args.run:
        print(json.dumps({"cases": len(cases), "requests": len(cases)*args.repeats*2,
                          "status": "validated; add --run to execute; baseline_context must be approved and secret-free"}))
        return
    if not args.output or args.repeats < 1 or not (args.primary_command or (args.primary_url and args.primary_model)):
        parser.error("--run requires --output, positive repeats and primary model URL/model or command")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x", encoding="utf-8") as output:
        for case in cases:
            for repeat in range(args.repeats):
                variants = ("baseline", "offload") if repeat % 2 == 0 else ("offload", "baseline")
                for variant in variants:
                    row = {"case_id": case["id"], "repeat": repeat, "variant": variant,
                           "baseline_sha256": hashlib.sha256(case["baseline_context"].encode()).hexdigest(),
                           "primary_model": args.primary_model or "external-adapter", "cache_policy": "unchanged", "secondary_metrics": []}
                    started = time.monotonic()
                    try:
                        row["workspace_snapshot"] = fingerprint(case)
                        context = case["baseline_context"]
                        if variant == "offload":
                            context, row["secondary_metrics"] = secondary(args, case)
                        row["answer"], usage = primary(args, case["task"], context)
                        row["primary_usage"] = actual_usage(usage)
                        row["status"] = "ok" if fingerprint(case) == row["workspace_snapshot"] else "workspace_changed"
                    except Exception as error:
                        row["status"] = "failed"
                        row["error_type"] = type(error).__name__
                        row["primary_usage"] = None
                    row["elapsed_ms"] = round((time.monotonic()-started)*1000)
                    output.write(json.dumps(row, ensure_ascii=False) + "\n")
                    output.flush()
                    os.fsync(output.fileno())


if __name__ == "__main__":
    main()
