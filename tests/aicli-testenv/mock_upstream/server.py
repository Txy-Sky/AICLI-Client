#!/usr/bin/env python3
"""Deterministic local upstream for protocol and failure-mode tests."""

import argparse
import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse


class State:
    def __init__(self):
        self.lock = threading.Lock()
        self.control = {}
        self.requests = []

    def record(self, item):
        with self.lock:
            self.requests.append(item)
            return dict(self.control)

    def set_control(self, values):
        with self.lock:
            self.control = dict(values)

    def snapshot(self):
        with self.lock:
            return list(self.requests)


STATE = State()


def model_controls(model, control):
    values = dict(control)
    for key in ("delay_first_byte", "idle_gap", "fail_status", "fail_after_n_chunks", "stream_error_event"):
        marker = "__" + key + "="
        if marker in model:
            raw = model.split(marker, 1)[1].split("__", 1)[0]
            values[key] = float(raw) if key in ("delay_first_byte", "idle_gap") else int(raw) if key in ("fail_status", "fail_after_n_chunks") else raw.lower() not in ("0", "false", "no")
    return values


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_args):
        return

    def send_json(self, status, payload):
        data = json.dumps(payload, separators=(",", ":")).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        if urlparse(self.path).path == "/__requests":
            self.send_json(200, {"requests": STATE.snapshot()})
            return
        self.send_json(404, {"error": "not_found"})

    def do_POST(self):
        path = urlparse(self.path).path
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length) if length else b""
        try:
            payload = json.loads(body or b"{}")
        except json.JSONDecodeError:
            self.send_json(400, {"error": "invalid_json"})
            return
        if path == "/__control":
            STATE.set_control(payload)
            self.send_json(200, {"ok": True, "control": payload})
            return

        model = str(payload.get("model", "mock-model"))
        control = model_controls(model, STATE.record({"path": path, "body": payload}),)
        status = int(control.get("fail_status", 0) or 0)
        if status:
            self.send_json(status, {"error": {"message": "injected failure", "type": "mock_error"}})
            return
        streaming = bool(payload.get("stream")) or ":streamGenerateContent" in path
        protocol = "gemini" if ":generateContent" in path else "messages" if path.endswith("/v1/messages") else "responses" if path.endswith("/v1/responses") else "chat"
        if not streaming:
            self.send_json(200, self.non_stream(protocol, model))
            return
        chunks = self.stream_chunks(protocol, model)
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "close")
        self.end_headers()
        delay = float(control.get("delay_first_byte", 0) or 0)
        if delay:
            time.sleep(delay)
        limit = int(control.get("fail_after_n_chunks", 0) or 0)
        for index, chunk in enumerate(chunks, 1):
            if limit and index > limit:
                self.close_connection = True
                return
            if index > 1 and control.get("idle_gap"):
                time.sleep(float(control["idle_gap"]))
            if protocol == "gemini":
                line = "data: " + json.dumps(chunk, separators=(",", ":")) + "\n\n"
            else:
                line = "data: " + json.dumps(chunk, separators=(",", ":")) + "\n\n"
            self.wfile.write(line.encode())
            self.wfile.flush()
        if control.get("stream_error_event"):
            self.wfile.write(b"event: error\ndata: {\"error\":{\"message\":\"injected stream error\"}}\n\n")
            self.wfile.flush()
        else:
            self.wfile.write(b"data: [DONE]\n\n")
            self.wfile.flush()
        self.close_connection = True

    @staticmethod
    def non_stream(protocol, model):
        if protocol == "gemini":
            return {"candidates": [{"content": {"parts": [{"text": "mock response"}], "role": "model"}, "finishReason": "STOP"}]}
        if protocol == "messages":
            return {"id": "mock-message", "type": "message", "role": "assistant", "model": model, "content": [{"type": "text", "text": "mock response"}], "stop_reason": "end_turn"}
        if protocol == "responses":
            return {"id": "mock-response", "object": "response", "model": model, "output": [{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "mock response"}]}], "status": "completed"}
        return {"id": "mock-chat", "object": "chat.completion", "model": model, "choices": [{"index": 0, "message": {"role": "assistant", "content": "mock response"}, "finish_reason": "stop"}]}

    @staticmethod
    def stream_chunks(protocol, model):
        if protocol == "gemini":
            return [{"candidates": [{"content": {"parts": [{"text": "mock "}], "role": "model"}}]}, {"candidates": [{"content": {"parts": [{"text": "response"}], "role": "model"}, "finishReason": "STOP"}]}]
        if protocol == "messages":
            return [{"type": "message_start", "message": {"id": "mock-message", "role": "assistant"}}, {"type": "content_block_delta", "delta": {"type": "text_delta", "text": "mock response"}}, {"type": "message_stop"}]
        if protocol == "responses":
            return [{"type": "response.output_text.delta", "delta": "mock response"}, {"type": "response.completed", "response": {"status": "completed"}}]
        return [{"id": "mock-chat", "object": "chat.completion.chunk", "model": model, "choices": [{"index": 0, "delta": {"role": "assistant", "content": "mock "}}]}, {"id": "mock-chat", "object": "chat.completion.chunk", "model": model, "choices": [{"index": 0, "delta": {"content": "response"}, "finish_reason": "stop"}]}]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=8080)
    args = parser.parse_args()
    ThreadingHTTPServer(("0.0.0.0", args.port), Handler).serve_forever()


if __name__ == "__main__":
    main()
