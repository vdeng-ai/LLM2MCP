import argparse
import json
import os
import subprocess
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import paired


class EvaluationTest(unittest.TestCase):
    def test_missing_usage_and_scores_never_establish_savings(self):
        rows = [{"case_id":"a", "repeat":0, "variant":variant, "status":"ok", "primary_usage":usage}
                for variant, usage in [("baseline", {"prompt_tokens":100,"completion_tokens":10}),
                                       ("offload", None)]]
        self.assertIsNone(paired.report(rows, {})["pairs"][0]["primary_tokens_saved"])
        self.assertFalse(paired.report(rows, {"a:0":{"baseline":1,"offload":1}})["pairs"][0]["validated_saving"])
        rows[1]["primary_usage"] = {"prompt_tokens":20,"completion_tokens":10}
        self.assertFalse(paired.report(rows, {})["pairs"][0]["validated_saving"])
        self.assertTrue(paired.report(rows, {"a:0":{"baseline":1,"offload":1}})["pairs"][0]["validated_saving"])
        self.assertFalse(paired.report(rows, {"a:0":{"baseline":1,"offload":0.5}})["pairs"][0]["validated_saving"])
        self.assertIsNone(paired.actual_usage({"prompt_tokens":True,"completion_tokens":0}))

    @unittest.skipUnless(os.environ.get("LLM2MCP_TEST_BINARY"), "set binary for full HTTP + MCP experiment")
    def test_real_stdio_mcp_and_primary_http_pair(self):
        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_POST(self):
                payload = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                is_primary = payload["model"] == "primary"
                answer = "Measured answer" if is_primary else json.dumps({"conclusion":"Code summary","findings":[],"risks":[],"actions":[],"read_next":[]})
                response = {"choices":[{"message":{"content":answer},"finish_reason":"stop"}],
                            "usage":{"prompt_tokens":777 if is_primary else 111,"completion_tokens":23}}
                body = json.dumps(response).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            with tempfile.TemporaryDirectory() as root:
                root = Path(root)
                config = root / "config"
                repo = root / "repo"
                config.mkdir()
                repo.mkdir()
                (repo / "main.rs").write_text("fn main() {}\n")
                url = f"http://127.0.0.1:{server.server_port}/v1"
                (config / "config.json").write_text(json.dumps({"base_url":url,"model":"secondary","tools":{"analyze":{"reasoning":"off","max_output_tokens":128,"execution":"sync"}}}))
                cases = root / "cases.json"
                cases.write_text(json.dumps([{"id":"a","workspace":str(repo),"paths":["main.rs"],"task":"explain main","baseline_context":"fn main() {}"}]))
                output = root / "results.jsonl"
                env = os.environ.copy()
                env.update(LLM2MCP_CONFIG_DIR=str(config), LLM2MCP_DATA_DIR=str(root / "data"))
                for key in ("LLM2MCP_BASE_URL", "LLM2MCP_MODEL", "LLM2MCP_API_KEY"):
                    env.pop(key, None)
                subprocess.run(["python3", str(Path(paired.__file__).resolve()), "--cases", str(cases),
                                "--executable", env["LLM2MCP_TEST_BINARY"], "--primary-url", url, "--primary-model", "primary",
                                "--run", "--repeats", "1", "--output", str(output), "--timeout", "15"], env=env, check=True)
                rows = [json.loads(line) for line in output.read_text().splitlines()]
                self.assertEqual([row["status"] for row in rows], ["ok", "ok"], rows)
                self.assertEqual(rows[0]["primary_usage"]["prompt_tokens"], 777)
                self.assertEqual(rows[1]["secondary_metrics"][0]["prompt_tokens"], 111)
                self.assertFalse(paired.report(rows, {})["pairs"][0]["validated_saving"])
        finally:
            server.shutdown()
            server.server_close()
            thread.join()


if __name__ == "__main__":
    unittest.main()
