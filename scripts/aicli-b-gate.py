#!/usr/bin/env python3
"""CI-only real Tauri dev/IPC smoke check in a fresh, disposable HOME."""

import http.server
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import tempfile
import threading
import time


ROOT = Path(__file__).resolve().parents[1]


def main():
    assert os.environ.get("GITHUB_ACTIONS") == "true", "Run only on the disposable CI runner"
    with tempfile.TemporaryDirectory(prefix="aicli-b-gate-", dir=os.environ["RUNNER_TEMP"]) as temp:
        home = Path(temp)
        report = {}
        received = threading.Event()

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_GET(self):
                state_path = home / ".aicli-client/live-state.json"
                apps = json.loads(state_path.read_text()).get("apps", {}) if state_path.exists() else {}
                ready = all(apps.get(app, {}).get("mode") == "direct" for app in
                            ("claude", "codex", "gemini", "grokbuild", "claude-desktop"))
                self.send_response(200 if ready else 503)
                self.send_header("Access-Control-Allow-Origin", "http://localhost:3000")
                self.end_headers()

            def do_POST(self):
                nonlocal report
                length = int(self.headers["Content-Length"])
                assert 0 < length < 8192
                report = json.loads(self.rfile.read(length))
                self.send_response(200)
                self.send_header("Access-Control-Allow-Origin", "http://localhost:3000")
                self.end_headers()
                received.set()

        server = http.server.HTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        endpoint = f"http://127.0.0.1:{server.server_port}"
        html_path = ROOT / "src/index.html"
        original_html = html_path.read_bytes()
        probe_path = ROOT / "src/aicli-b-gate-probe.js"
        assert not probe_path.exists()
        probe_path.write_text('''import { invoke } from "@tauri-apps/api/core";
const endpoint = ENDPOINT;
(async () => {
  try {
    const deadline = Date.now() + 90000;
    while (!(await fetch(endpoint)).ok) {
      if (Date.now() > deadline) throw new Error("Startup modes did not settle");
      await new Promise(resolve => setTimeout(resolve, 250));
    }
    const initError = await invoke("get_init_error");
    if (initError) throw new Error(JSON.stringify(initError));
    const info = await invoke("start_proxy_server");
    const status = await invoke("get_proxy_status");
    await fetch(endpoint, {method: "POST", body: JSON.stringify({info, status})});
  } catch (error) {
    await fetch(endpoint, {method: "POST", body: JSON.stringify({error: String(error)})});
  }
})();
'''.replace("ENDPOINT", json.dumps(endpoint)))
        html_path.write_bytes(original_html.replace(b"</body>",
                             b'<script type="module" src="./aicli-b-gate-probe.js"></script></body>'))
        env = os.environ.copy()
        env.update(HOME=temp, CC_SWITCH_TEST_HOME=temp,
                   XDG_CONFIG_HOME=str(home / ".config"),
                   XDG_DATA_HOME=str(home / ".local/share"),
                   XDG_CACHE_HOME=str(home / ".cache"))
        log_path = Path(os.environ["RUNNER_TEMP"]) / "aicli-evidence/B-GATE-startup.txt"
        process = None
        try:
            with log_path.open("w") as log:
                process = subprocess.Popen(["xvfb-run", "-a", "pnpm", "tauri", "dev"],
                                           cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT,
                                           start_new_session=True)
                deadline = time.monotonic() + 600
                while not received.wait(1):
                    assert process.poll() is None, f"Tauri dev exited: {process.returncode}"
                    assert time.monotonic() < deadline, "Tauri dev/IPC probe timed out"
                assert process.poll() is None, "Desktop process must remain running"
                assert "error" not in report, report
                assert report["info"]["address"] == "127.0.0.1", report
                assert report["info"]["port"] == 15821, report
                assert report["status"]["running"] is True, report
                assert report["status"]["address"] == "127.0.0.1", report
                assert report["status"]["port"] == 15821, report
                with socket.create_connection(("127.0.0.1", 15821), timeout=5):
                    pass
                listeners = subprocess.check_output(["ss", "-ltnp", "sport = :15821"], text=True)
                print(listeners, flush=True)
                assert "127.0.0.1:15821" in listeners, listeners
                assert "0.0.0.0:15821" not in listeners and "[::]:15821" not in listeners, listeners
                assert (home / ".aicli-client/aicli-client.db").is_file()
                for legacy in (".cc-switch", ".claude", ".codex", ".gemini"):
                    assert not (home / legacy).exists(), f"Unexpected application directory: {legacy}"
                entries = {p.name for p in home.iterdir()}
                assert entries <= {".aicli-client", ".config", ".cache", ".local"}, entries
                print("B-GATE IPC:", json.dumps(report, sort_keys=True), flush=True)
                print("B-GATE HOME entries:", sorted(entries), flush=True)
                print("B-GATE: PASS; real Tauri dev startup, fresh AICLI configuration, "
                      "real IPC proxy start and loopback listener verified", flush=True)
        finally:
            if process is not None and process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
            html_path.write_bytes(original_html)
            probe_path.unlink()
            server.shutdown()
            print(log_path.read_text(), flush=True)


if __name__ == "__main__":
    main()
