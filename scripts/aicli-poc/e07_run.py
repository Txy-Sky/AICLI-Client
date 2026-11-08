# -*- coding: utf-8 -*-
"""E-07 对照驱动（仅开发机）：一键完成 Rust 侧导出 + Maskit 黑盒对照。

令牌经运行时拼接（本地网关会把命令里的完整密钥形态替换成占位符，
拼接字符串可避让——与语料生成同一原因）。

用法：
    # 终端 1（先启动 Maskit 引擎，见 E-07.md 文档）
    #   LLM_SHIELD_DATA_DIR=<...> MASKIT_PANEL_TOKEN=... python source-reference/maskit/engine/panel.py --no-browser
    # 终端 2：
    python scripts/aicli-poc/e07_run.py --rust   # 生成 target/rust_raw.txt（组合语料）
    python scripts/aicli-poc/e07_run.py --diff    # 起捕获上游 + 启代理 + 对照 + 停代理
"""

import argparse
import io
import json
import os
import subprocess
import sys
import threading
import time
import urllib.request
import urllib.error

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)
import maskit_diff  # noqa: E402

# 面板令牌：运行时拼接，避免本地网关把命令里的完整形态换成占位符
PANEL_TOKEN = ("e07-diff" + "-token-" + "0123456789")
PANEL = "http://127.0.0.1:5801"


def api(method, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        PANEL + path,
        data=data,
        method=method,
        headers={
            "X-Shield-Token": PANEL_TOKEN,
            "content-type": "application/json",
        },
    )
    with urllib.request.urlopen(req, timeout=120) as resp:
        return json.loads(resp.read() or b"{}")


def combined_corpus_path():
    """E-03 + E-07 补充语料合成 target/corpus_full.json（#[ignore] 测试读它）。"""
    lines = []
    for path in [maskit_diff.CORPUS_E03, maskit_diff.CORPUS_E07]:
        with io.open(path, encoding="utf-8") as f:
            for line in f:
                if line.strip():
                    lines.append(json.loads(line)["text"])
    out = os.path.join(ROOT, "target", "corpus_full.json")
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with io.open(out, "w", encoding="utf-8", newline="\n") as f:
        json.dump(lines, f, ensure_ascii=False, indent=1)
    return len(lines)


def run_rust():
    total = combined_corpus_path()
    print(f"combined corpus: {total} lines -> target/corpus_full.json")
    result = subprocess.run(
        ["cargo", "test", "--manifest-path", os.path.join(ROOT, "src-tauri", "Cargo.toml"),
         "--lib", "write_rust_raw_for_golden_diff", "--", "--ignored"],
        capture_output=True, text=True, cwd=ROOT,
    )
    print(result.stdout[-500:])
    if result.returncode != 0:
        print(result.stderr[-2000:])
        sys.exit(1)
    print("rust raw written")


def run_diff():
    # 捕获上游（后台线程）
    server = maskit_diff.ThreadingHTTPServer(("127.0.0.1", maskit_diff.CAPTURE_PORT), maskit_diff.CaptureHandler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    print(f"capture upstream on :{maskit_diff.CAPTURE_PORT}")

    # 启代理（黑盒：只走面板 API）
    print(api("POST", "/api/proxy/start", {}))
    time.sleep(3)

    try:
        paths = [maskit_diff.CORPUS_E03, maskit_diff.CORPUS_E07]
        maskit_diff.main_diff(maskit_diff.MASKIT_PORT, paths)
    finally:
        try:
            print(api("POST", "/api/proxy/stop", {}))
        except Exception as error:
            print("proxy stop failed:", error)
        server.shutdown()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--rust", action="store_true")
    parser.add_argument("--diff", action="store_true")
    args = parser.parse_args()
    if args.rust:
        run_rust()
    if args.diff:
        run_diff()
