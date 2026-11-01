#!/usr/bin/env python3
"""A5-01: read-only discovery and budgeted direct HTTPS/SSE measurements.

Credentials are read only from AICLI_TEST_ACCESS_TOKEN; existing model keys stay
in memory. No response bodies, account names, keys or exception text are logged.
Run online probes locally with an isolated HOME, never in GitHub Actions.
"""

import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
from decimal import Decimal, InvalidOperation, ROUND_CEILING
import http.client
import json
import os
from pathlib import Path
import re
import ssl
import time
import urllib.error
import urllib.parse
import urllib.request


LINES = {
    "main": "https://aicliapi.com",
    "global": "https://aicli.qzz.io",
    "apac": "https://api.aicli.qzz.io",
}
USER_AGENT = "AICLI-Client/line-probe"
PROMPTS = {
    "idle": (
        "Independently solve this difficult counting problem, checking all cases "
        "carefully before giving only the final answer: how many binary 12 by 12 "
        "matrices have exactly six ones in every row and column, with the main "
        "diagonal all zero? Do not use tools. Do not give a quick approximation. "
        "Take the time needed for rigorous internal verification."
    ),
    "continuous": (
        "Generate a very long English language streaming test. Write lines numbered "
        "1 through 20000. Each line must contain its number followed by a new, "
        "original sentence of at least twelve words describing an imaginary walk "
        "through a different landscape. Start line 1 immediately. Keep emitting "
        "consecutive lines; do not summarize, skip numbers, explain limitations, "
        "write code, use tools, or conclude early. Continue until the output limit."
    ),
}


class ProbeBlocked(Exception):
    """Only fixed, credential-free error codes may be attached."""


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        # An account Authorization header must never follow a redirect.
        return None


def direct_opener():
    context = ssl.create_default_context()
    assert context.check_hostname and context.verify_mode == ssl.CERT_REQUIRED
    return urllib.request.build_opener(
        urllib.request.ProxyHandler({}), NoRedirect(),
        urllib.request.HTTPSHandler(context=context),
    )


class Api:
    GET_PATHS = {"/api/status", "/api/user/self", "/api/user/self/groups",
                 "/api/pricing", "/api/token/", "/api/log/self"}

    def __init__(self, access_token=""):
        self.access_token = access_token
        self.model_key = ""
        self.opener = direct_opener()

    def request(self, line, path, *, payload=None, model=False):
        route = urllib.parse.urlsplit(path).path
        if line not in LINES or not path.startswith("/") or path.startswith("//"):
            raise ProbeBlocked("unapproved_destination")
        if payload is None:
            allowed = route in self.GET_PATHS and not model
        else:
            allowed = (route == "/api/token/batch/keys" and not model) or (
                route == "/v1/responses" and model)
        if not allowed:
            raise ProbeBlocked("unapproved_operation")
        headers = {"User-Agent": USER_AGENT, "Accept-Encoding": "identity"}
        key = self.model_key if model else self.access_token
        if route != "/api/status":
            if not key:
                raise ProbeBlocked("missing_credential")
            headers["Authorization"] = "Bearer " + key
        data = None
        if payload is not None:
            data = json.dumps(payload, ensure_ascii=True).encode("ascii")
            headers["Content-Type"] = "application/json"
        return urllib.request.Request(LINES[line] + path, headers=headers, data=data)

    def fetch(self, path, payload=None):
        request = self.request("main", path, payload=payload)
        with self.opener.open(request, timeout=30) as response:
            body = json.load(response)
        if not isinstance(body, dict) or body.get("success") is not True:
            raise ProbeBlocked("api_unsuccessful")
        return body


def health_line(line):
    api = Api()
    rows = []
    for attempt in range(1, 11):
        start = time.monotonic()
        row = {"line": line, "attempt": attempt, "tls": "not_completed"}
        try:
            with api.opener.open(api.request(line, "/api/status"), timeout=30) as response:
                row.update(tls="verified", http_status=response.status,
                           application_success=json.load(response).get("success") is True)
        except urllib.error.HTTPError as error:
            row.update(tls="verified", http_status=error.code, error="HTTPError")
            error.close()
        except (OSError, urllib.error.URLError, ValueError) as error:
            row["error"] = type(error).__name__
        row["latency_ms"] = round((time.monotonic() - start) * 1000, 2)
        row["passed"] = (row.get("tls") == "verified"
                         and row.get("http_status") == 200
                         and row.get("application_success") is True)
        rows.append(row)
    return rows


def number(value):
    try:
        result = Decimal(str(value))
    except (InvalidOperation, ValueError):
        raise ProbeBlocked("unknown_price") from None
    if not result.is_finite() or result < 0:
        raise ProbeBlocked("unknown_price")
    return result


def budget_plan(quota, token, model, groups, prompt, max_output_tokens):
    """rc.34 service/text_quota.go: (input + output*completion)*model*group.

    Only ordinary token-priced, single-group, tool-free text requests qualify.
    Reserve 65,536 input tokens for a fixed prompt <=4KiB, and add a 10% margin.
    This is a conservative preflight bound, not a substitute for server billing.
    Unknown/tiered pricing is blocked; an existing Token is never edited to cap it.
    """
    if (not isinstance(quota, int) or isinstance(quota, bool) or quota <= 0
            or not isinstance(max_output_tokens, int) or not 1 <= max_output_tokens <= 65536):
        raise ProbeBlocked("invalid_budget")
    if len(prompt.encode("utf-8")) > 4096:
        raise ProbeBlocked("prompt_too_large")
    group = token.get("group")
    if group == "auto" or group not in groups:
        raise ProbeBlocked("unbounded_token_group")
    if (model.get("quota_type") != 0 or model.get("billing_mode") not in (None, "", "token")
            or model.get("billing_expr")):
        raise ProbeBlocked("unsupported_pricing")
    if group not in model.get("enable_groups", []) and "all" not in model.get("enable_groups", []):
        raise ProbeBlocked("model_not_in_token_group")
    ratio = number(groups[group].get("ratio"))
    input_ratio, completion_ratio = number(model.get("model_ratio")), number(model.get("completion_ratio"))
    limit = quota // 20
    if not token.get("unlimited_quota"):
        remaining = token.get("remain_quota")
        if not isinstance(remaining, int) or remaining <= 0:
            raise ProbeBlocked("token_quota_unavailable")
        limit = min(limit, remaining)
    upper = int(((Decimal(65536) + max_output_tokens * completion_ratio)
                 * input_ratio * ratio * Decimal("1.1")).to_integral_value(rounding=ROUND_CEILING))
    if upper > limit:
        raise ProbeBlocked("request_exceeds_five_percent")
    return {"account_quota_before": quota, "five_percent_quota": quota // 20,
            "reserved_input_tokens": 65536, "max_output_tokens": max_output_tokens,
            "model_ratio": str(input_ratio), "completion_ratio": str(completion_ratio),
            "group_ratio": str(ratio), "margin": "1.1", "upper_quota": upper}


def choose_token(api, model):
    tokens = []
    for page in range(1, 101):
        data = api.fetch(f"/api/token/?p={page}&page_size=100")["data"]
        items = data["items"]
        tokens.extend(items)
        if len(tokens) >= data["total"]:
            break
    else:
        raise ProbeBlocked("token_pagination_limit")
    now = time.time()
    for token in tokens:
        if (token.get("status") == 1 and token.get("group") != "auto"
                and token.get("group") in model.get("enable_groups", [])
                and (token.get("expired_time") == -1 or token.get("expired_time", 0) > now)
                and not token.get("model_limits_enabled") and not token.get("allow_ips")):
            return token
    raise ProbeBlocked("no_eligible_existing_token")


class StreamMetrics:
    def __init__(self, start):
        self.start = start
        self.first = self.last = self.first_text = self.last_text = None
        self.byte_count = self.text_events = self.text_chars = self.comment_lines = 0
        self.max_byte_gap = self.max_text_gap = 0.0
        self.data_events = 0
        self.done = self.api_error = False
        self.usage = {}
        self.line = bytearray()
        self.data = []

    def feed(self, value, now):
        if self.first is None:
            self.first = now
        if self.last is not None:
            self.max_byte_gap = max(self.max_byte_gap, now - self.last)
        self.last = now
        self.byte_count += len(value)
        self.line.extend(value)
        if len(self.line) > 1048576:
            raise ProbeBlocked("sse_line_too_large")
        if value == b"\n":
            line = bytes(self.line).rstrip(b"\r\n")
            self.line.clear()
            if line.startswith(b":"):
                self.comment_lines += 1
            elif line.startswith(b"data:"):
                self.data.append(line[5:].lstrip(b" "))
            elif not line and self.data:
                raw = b"\n".join(self.data)
                self.data.clear()
                self.event(raw, now)

    def event(self, raw, now):
        self.data_events += 1
        if raw == b"[DONE]":
            self.done = True
            return
        try:
            data = json.loads(raw)
        except (ValueError, UnicodeError):
            raise ProbeBlocked("invalid_sse_json") from None
        if not isinstance(data, dict):
            raise ProbeBlocked("invalid_sse_event")
        kind = data.get("type")
        if kind in ("error", "response.failed") or data.get("error"):
            self.api_error = True
        if kind == "response.completed":
            self.done = True
        if kind == "response.incomplete":
            self.done = True
            if data.get("response", {}).get("incomplete_details", {}).get("reason") != "max_output_tokens":
                self.api_error = True
        if kind == "response.output_text.delta" and isinstance(data.get("delta"), str) and data["delta"]:
            self.first_text = now if self.first_text is None else self.first_text
            if self.last_text is not None:
                self.max_text_gap = max(self.max_text_gap, now - self.last_text)
            self.last_text = now
            self.text_events += 1
            self.text_chars += len(data["delta"])
        usage = data.get("response", {}).get("usage", {})
        if isinstance(usage, dict):
            for name in ("input_tokens", "output_tokens", "total_tokens"):
                if isinstance(usage.get(name), int) and usage[name] >= 0:
                    self.usage[name] = usage[name]

    def summary(self, end):
        span = 0 if self.first_text is None else self.last_text - self.first_text
        return {"duration_s": round(end - self.start, 3),
                "first_byte_s": None if self.first is None else round(self.first - self.start, 3),
                "max_adjacent_byte_gap_s": round(self.max_byte_gap, 3),
                "silence_before_end_s": None if self.last is None else round(end - self.last, 3),
                "text_span_s": round(span, 3), "max_text_gap_s": round(self.max_text_gap, 3),
                "body_bytes": self.byte_count, "text_events": self.text_events,
                "text_chars": self.text_chars, "comment_lines": self.comment_lines,
                "data_events": self.data_events, "terminal_event": self.done,
                "api_error": self.api_error, "usage": self.usage}


def stream_probe(api, line, model, scenario, max_output_tokens):
    payload = {"model": model, "stream": True, "input": PROMPTS[scenario],
               "reasoning": {"effort": "high" if scenario == "idle" else "low"},
               "max_output_tokens": max_output_tokens}
    start = time.monotonic()
    metrics = StreamMetrics(start)
    row = {"line": line, "scenario": scenario, "model": model,
           "byte_timing": "application-visible one-byte reads; not packet capture",
           "stop": "eof", "request_id": None}
    try:
        with api.opener.open(api.request(line, "/v1/responses", payload=payload, model=True), timeout=300) as response:
            row.update(http_status=response.status, tls="verified")
            request_id = response.headers.get("X-Oneapi-Request-Id", "")
            if re.fullmatch(r"[A-Za-z0-9_-]{1,128}", request_id):
                row["request_id"] = request_id
            if not response.headers.get("Content-Type", "").lower().startswith("text/event-stream"):
                raise ProbeBlocked("non_sse_response")
            sock = response.fp.raw._sock
            next_timeout_update = 0
            while True:
                now = time.monotonic()
                if now - start >= 600:
                    row["stop"] = "client_total_limit"
                    break
                if now >= next_timeout_update:
                    sock.settimeout(min(300, max(0.1, 600 - (now - start))))
                    next_timeout_update = now + 1
                value = response.read1(1)
                if not value:
                    break
                now = time.monotonic()
                metrics.feed(value, now)
                if metrics.api_error:
                    row["stop"] = "api_error_event"
                    break
                if (scenario == "continuous" and metrics.first_text is not None
                        and metrics.last_text - metrics.first_text > 185):
                    row["stop"] = "client_observed_over_180s"
                    break
    except urllib.error.HTTPError as error:
        row.update(stop="http_error", http_status=error.code, error="HTTPError")
        error.close()
    except ProbeBlocked as error:
        row.update(stop="probe_blocked", error=str(error))
    except (OSError, urllib.error.URLError, http.client.HTTPException) as error:
        row.update(stop="transport_error", error=type(error).__name__)
    row.update(metrics.summary(time.monotonic()))
    return row


def bill(api, request_id):
    if not request_id:
        return {"found": False}
    query = urllib.parse.urlencode({"request_id": request_id, "type": 2, "p": 1, "page_size": 10})
    for delay in (1, 3, 10):
        time.sleep(delay)
        data = api.fetch("/api/log/self?" + query)["data"]
        items = [row for row in data["items"] if row.get("request_id") == request_id and row.get("type") == 2]
        if items and all(isinstance(row.get("quota"), int) and row["quota"] >= 0 for row in items):
            return {"found": True, "quota": sum(row["quota"] for row in items)}
    return {"found": False}


def sustained(row):
    return (row.get("http_status") == 200 and row.get("tls") == "verified"
            and not row.get("api_error") and row.get("text_span_s", 0) > 180
            and row.get("max_text_gap_s", 999) <= 30
            and row.get("max_adjacent_byte_gap_s", 999) <= 30)


def decide(rows):
    continuous = {row["line"]: row for row in rows if row["scenario"] == "continuous"}
    idle = {row["line"]: row for row in rows if row["scenario"] == "idle"}
    if set(continuous) != set(LINES) or set(idle) != set(LINES):
        return "inconclusive"
    if not all(row.get("http_status") == 200 and row.get("data_events", 0) > 0
               and not row.get("api_error") and row.get("terminal_event") for row in idle.values()):
        return "inconclusive"
    if all(sustained(row) for row in continuous.values()):
        return "idle_limit_use_600s"
    broken = {"eof", "transport_error"}
    if (sustained(continuous["apac"]) and all(
        continuous[line].get("stop") in broken
        and not continuous[line].get("terminal_event")
        and not continuous[line].get("api_error")
        and continuous[line].get("http_status") == 200
        and continuous[line].get("text_events", 0) > 1
        and 110 <= continuous[line].get("duration_s", 0) <= 135
        and continuous[line].get("max_adjacent_byte_gap_s", 999) <= 30
        for line in ("main", "global")
    )):
        return "total_limit_use_110s_prefer_apac"
    return "inconclusive"


def save_report(path, report, api):
    serialized = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    for secret in (api.access_token, api.model_key):
        if secret and secret in serialized:
            raise ProbeBlocked("credential_in_report")
    path.parent.mkdir(parents=True, exist_ok=True)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as output:
        output.write(serialized)


def run(args):
    access = os.environ.get("AICLI_TEST_ACCESS_TOKEN", "").strip()
    if not args.health_only and (not access or access.startswith("sk-")):
        raise ProbeBlocked("missing_or_wrong_access_token")
    if not os.environ.get("CC_SWITCH_TEST_HOME") or os.environ.get("HOME") != os.environ["CC_SWITCH_TEST_HOME"]:
        raise ProbeBlocked("isolated_home_required")
    api = Api(access if not args.health_only else "")
    report = {"item": "A5-01", "started_at": datetime.now(timezone.utc).isoformat(),
              "direct": True, "health": [], "streams": [], "decision": "inconclusive"}
    try:
        with ThreadPoolExecutor(max_workers=3) as pool:
            for rows in pool.map(health_line, LINES):
                report["health"].extend(rows)
                print(json.dumps({"line": rows[0]["line"], "health_passed": sum(r["passed"] for r in rows), "total": 10}), flush=True)
        save_report(args.output, report, api)
        if not all(row["passed"] for row in report["health"]):
            raise ProbeBlocked("health_preflight_failed")
        if args.health_only:
            return 0
        pricing = api.fetch("/api/pricing")
        matches = [model for model in pricing["data"] if model["model_name"] == args.model]
        if len(matches) != 1 or "openai-response" not in matches[0].get("supported_endpoint_types", []):
            raise ProbeBlocked("model_not_available_for_responses")
        model = matches[0]
        groups = api.fetch("/api/user/self/groups")["data"]
        token = choose_token(api, model)
        # Confirm budget before reading a plaintext key. No create/update/delete API exists here.
        quota = api.fetch("/api/user/self")["data"]["quota"]
        budget_plan(quota, token, model, groups, PROMPTS["continuous"], args.max_output_tokens)
        keys = api.fetch("/api/token/batch/keys", {"ids": [token["id"]]})["data"]["keys"]
        key = keys.get(str(token["id"]), "")
        if not isinstance(key, str) or not key or key.strip() != key:
            raise ProbeBlocked("existing_key_unavailable")
        api.model_key = key if key.startswith("sk-") else "sk-" + key
        report["existing_token_only"] = True
        for scenario in ("idle", "continuous"):
            for line in LINES:
                # Refetch pricing and quota before each chargeable request.
                quota = api.fetch("/api/user/self")["data"]["quota"]
                current = api.fetch("/api/pricing")
                models = [m for m in current["data"] if m["model_name"] == args.model]
                if len(models) != 1:
                    raise ProbeBlocked("model_catalog_changed")
                current_groups = api.fetch("/api/user/self/groups")["data"]
                plan = budget_plan(quota, token, models[0], current_groups, PROMPTS[scenario], args.max_output_tokens)
                print(json.dumps({"line": line, "scenario": scenario, "state": "starting", "budget": plan}), flush=True)
                row = stream_probe(api, line, args.model, scenario, args.max_output_tokens)
                row["budget"] = plan
                report["streams"].append(row)
                save_report(args.output, report, api)
                row["billing"] = bill(api, row["request_id"])
                if row["billing"].get("found"):
                    row["billing"]["within_five_percent"] = row["billing"]["quota"] <= plan["five_percent_quota"]
                    if not row["billing"]["within_five_percent"]:
                        raise ProbeBlocked("actual_charge_exceeds_five_percent")
                save_report(args.output, report, api)
                print(json.dumps(row, ensure_ascii=False), flush=True)
                if row["stop"] in ("http_error", "api_error_event", "probe_blocked"):
                    raise ProbeBlocked("model_probe_failed")
        report["decision"] = decide(report["streams"])
        if not all(row["billing"].get("within_five_percent") for row in report["streams"]):
            raise ProbeBlocked("billing_unconfirmed")
        save_report(args.output, report, api)
        print(json.dumps({"decision": report["decision"]}), flush=True)
        return 0 if report["decision"] != "inconclusive" else 2
    except ProbeBlocked as error:
        report["blocked"] = str(error)
        save_report(args.output, report, api)
        print(json.dumps({"blocked": str(error)}), flush=True)
        return 2


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--health-only", action="store_true", help="Only 30 unauthenticated status probes")
    parser.add_argument("--model", help="An existing Token must already allow this Responses model")
    parser.add_argument("--max-output-tokens", type=int, default=65536)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not args.health_only and not args.model:
        parser.error("--model is required for chargeable probes")
    try:
        return run(args)
    except Exception as error:
        # Never stringify library errors; HTTP/auth exceptions may contain credentials.
        print(json.dumps({"blocked": "unhandled_failure", "error_type": type(error).__name__}), flush=True)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
