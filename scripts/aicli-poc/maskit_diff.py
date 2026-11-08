# -*- coding: utf-8 -*-
"""E-07 Maskit 黑盒对照工具（仅开发机运行，不进 CI / 产物）。

架构（交接说明推荐的「反向代理 + 捕获上游」形态）：

    本脚本 ──POST──▶ Maskit 反代端口 ──▶ 本脚本起的捕获上游（记录 body，
    回固定 JSON；不连任何真实上游）

用法：
    # 终端 1：启动 Maskit 引擎（源码态；配置见 --maskit-config 生成的 json）
    python engine/panel.py --config <生成的config.json> --headless
    #   （或按其 README 的任意启动方式；控制台里把上游指向本脚本）
    # 终端 2：
    python scripts/aicli-poc/maskit_diff.py --phase serve   # 起捕获上游
    python scripts/aicli-poc/maskit_diff.py --phase diff    # 跑对照，输出 E-07.md

只当黑盒：本脚本不读 Maskit 的代码/规则/夹具，只通过它的 HTTP 端口发语料、
收输出。语料来自 E-03 的 corpus.jsonl 与本目录 gen_corpus_e07.py 的补充语料
（自写，不含 Maskit 样例）。

对照口径：占位符格式不同，不能逐字节比。按「被替换的原文片段」比较——
用 difflib.SequenceMatcher 分别找两边输出相对原文的替换片段；片段归入
两边都遮 / 只我方 / 只对方 / 范围不同 四类。只对方遮的逐个归因：
我方漏遮（密钥/证件必须修）/ 对方误报 / 口径不同。
"""

import argparse
import difflib
import io
import json
import os
import re
import sys
import threading
import time
import urllib.request
import urllib.error
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CORPUS_E03 = os.path.join(ROOT, "src-tauri", "tests", "fixtures", "privacy", "corpus.jsonl")
CORPUS_E07 = os.path.join(os.path.dirname(os.path.abspath(__file__)), "corpus_e07.jsonl")
RUST_RAW = os.path.join(ROOT, "target", "rust_raw.jsonl")
OUT_MD = os.path.join(ROOT, "docs", "aicli-evidence", "E-07.md")

CAPTURE_PORT = 18990
MASKIT_PORT = 18701

_capture_lock = threading.Lock()
captured = []


class CaptureHandler(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("content-length", 0))
        body = self.rfile.read(length)
        with _capture_lock:
            captured.append(body)
        resp = json.dumps({"id": "cap", "object": "chat.completion", "choices": [
            {"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}}).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(resp)))
        self.end_headers()
        self.wfile.write(resp)

    def log_message(self, *args):
        pass


def serve():
    server = ThreadingHTTPServer(("127.0.0.1", CAPTURE_PORT), CaptureHandler)
    print(f"capture upstream listening on 127.0.0.1:{CAPTURE_PORT}")
    server.serve_forever()


# ---- 占位符形态 ----

# 我方：{{KIND_xxxxxxxx}}
OURS = re.compile(r"\{\{([A-Z][A-Z0-9_]*)_[a-z2-7]{8}\}\}")
# 对方：从其输出里找被替换片段不依赖形态（用 SequenceMatcher），类别
# 取不到就记 unknown。


def replaced_segments(original, output):
    """输出相对原文被替换的片段列表：(start, end, 原文)。"""
    matcher = difflib.SequenceMatcher(None, original, output, autojunk=False)
    segments = []
    for tag, i1, i2, j1, j2 in matcher.get_opcodes():
        if tag == "replace" and i2 > i1:
            # 替换段可能吞并相邻的插入；只取原文侧被吃掉的部分
            segments.append((i1, i2, original[i1:i2]))
        elif tag == "delete":
            segments.append((i1, i2, original[i1:i2]))
    # 合并相邻片段（占位符两侧的边界抖动）
    merged = []
    for seg in sorted(segments):
        if merged and seg[0] <= merged[-1][1]:
            merged[-1] = (merged[-1][0], max(merged[-1][1], seg[1]), None)
        else:
            merged.append(list(seg) if isinstance(seg, tuple) else seg)
    return [(a, b, original[a:b]) for a, b, _ in merged]


def send_through_maskit(text, port):
    """把 text 作为 messages[0].content 过 Maskit 反代，返回捕获上游收到的 content。"""
    body = json.dumps({
        "model": "cap-model",
        "messages": [{"role": "user", "content": text}],
    }).encode()
    req = urllib.request.Request(
        f"http://127.0.0.1:{port}/v1/chat/completions",
        data=body,
        headers={"content-type": "application/json", "authorization": "Bearer cap-key"},
    )
    with _capture_lock:
        captured.clear()
    try:
        urllib.request.urlopen(req, timeout=120).read()
    except urllib.error.HTTPError as error:
        print(f"maskit HTTP {error.code}: {error.read()[:200]!r}", file=sys.stderr)
        return None
    with _capture_lock:
        if not captured:
            return None
        got = json.loads(captured[0])
    try:
        return got["messages"][0]["content"]
    except (KeyError, IndexError):
        return None


def load_ours():
    """Rust 侧输出（JSONL，每行一个 {"out": …}；#[ignore] 测试生成，
    多行语料（PEM/.env）不破坏行对应）。"""
    with io.open(RUST_RAW, encoding="utf-8") as f:
        return [json.loads(line)["out"] for line in f if line.strip()]

def classify(only_theirs, original):
    """只对方遮的片段粗分类（人工复核写进 E-07.md）。"""
    text = only_theirs
    kinds = {
        "密钥类": re.compile(r"sk-|ghp_|AKIA|xox|api[_-]?key|secret|token|Bearer|PRIVATE KEY|AIza", re.I),
        "证件类": re.compile(r"\d{17}[\dXx]|\d{6}-?\d{2}-?\d{4}|[A-Z]{2}\d{2}[A-Z0-9 ]{10,}"),
        "手机/邮箱": re.compile(r"1[3-9]\d{9}|@", re.I),
        "IP": re.compile(r"\d+\.\d+\.\d+\.\d+"),
    }
    for name, pattern in kinds.items():
        if pattern.search(text):
            return name
    return "其他"


def main_diff(maskit_port, corpus_paths, limit=None):
    corpus = []
    polluted = 0
    for path in corpus_paths:
        with io.open(path, encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                record = json.loads(line)
                text = record.get("text", record.get("Text", ""))
                if "{{" in text:
                    # E-03 语料存档时部分原值已被本地网关替换成占位符
                    #（E-03.txt 已记录）：原值不可知，两边对占位符文本的处理
                    # 只是口径噪声，剔除
                    polluted += 1
                    corpus.append(None)
                    continue
                corpus.append(text)
    ours = load_ours()
    if limit:
        corpus = corpus[:limit]
        ours = ours[:limit]
    assert len(corpus) == len(ours), f"corpus {len(corpus)} vs rust {len(ours)}"

    stats = {"both": 0, "ours_only": 0, "theirs_only": 0, "range_diff": 0}
    theirs_only_segments = []
    for index, (text, our_out) in enumerate(zip(corpus, ours)):
        if text is None:
            continue  # 被网关污染的语料行
        their_out = send_through_maskit(text, maskit_port)
        if their_out is None:
            print(f"line {index}: maskit returned nothing, skipping", file=sys.stderr)
            continue
        ours_seg = replaced_segments(text, our_out)
        theirs_seg = replaced_segments(text, their_out)
        for a in ours_seg:
            overlap = [b for b in theirs_seg if b[0] < a[1] and a[0] < b[1]]
            if any(b[0] == a[0] and b[1] == a[1] for b in theirs_seg):
                stats["both"] += 1
            elif overlap:
                stats["range_diff"] += 1
            else:
                stats["ours_only"] += 1
        for b in theirs_seg:
            overlap = [a for a in ours_seg if a[0] < b[1] and b[0] < a[1]]
            if not any(a[0] == b[0] and a[1] == b[1] for a in ours_seg) and not overlap:
                stats["theirs_only"] += 1
                theirs_only_segments.append((index, b[2][:80]))

    with io.open(OUT_MD, "w", encoding="utf-8", newline="\n") as f:
        f.write("# E-07 Maskit 黑盒对照（差异统计与结论）\n\n")
        comparable = sum(1 for t in corpus if t is not None)
        f.write(f"- 语料：{len(corpus)} 条（E-03 corpus.jsonl + E-07 补充语料），"
                f"其中 {polluted} 条因存档时原值被本地网关替换成占位符（E-03.txt 已记录）剔除，"
                f"可比 {comparable} 条\n")
        f.write(f"- 我方输出：固定密钥 #[ignore] 导出（与 golden 同管线）\n")
        f.write(f"- 片段统计：两边都遮 {stats['both']}｜只我方 {stats['ours_only']}｜"
                f"只对方 {stats['theirs_only']}｜范围不同 {stats['range_diff']}\n\n")
        f.write("## 只对方遮的片段（逐个归因，人工填写）\n\n")
        for index, seg in theirs_only_segments:
            f.write(f"- L{index}: {seg!r} → 粗分：{classify(seg, '')}\n")
    print(f"wrote {OUT_MD}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--phase", choices=["serve", "diff"], required=True)
    parser.add_argument("--maskit-port", type=int, default=MASKIT_PORT)
    parser.add_argument("--limit", type=int, default=None)
    args = parser.parse_args()
    if args.phase == "serve":
        serve()
    else:
        paths = [CORPUS_E03]
        if os.path.exists(CORPUS_E07):
            paths.append(CORPUS_E07)
        main_diff(args.maskit_port, paths, args.limit)
