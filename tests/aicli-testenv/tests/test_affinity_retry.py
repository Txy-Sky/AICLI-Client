"""A6-05: HTTP-only affinity and streaming observations on disposable rc.34."""

import collections
import copy
import hashlib
import http.client
import json
import os
import re
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request

import pytest

from test_access_token import BASE, LocalApi, login, require


RULE = "aicli-session-test"
MODELS = ["mock-model-a", "gpt-aicli-affinity", "claude-aicli-affinity"]
PREFIX = "channel_affinity_setting."


def evidence(kind, **values):
    # Callers supply only fixture labels, counts, booleans and public rule data.
    # Never serialize an API response, credentials, headers or raw stream errors.
    print("A6-05 " + json.dumps({"kind": kind, **values}, ensure_ascii=False), flush=True)


def mock_call(method, path, body=None):
    project = os.environ.get("AICLI_TESTENV_PROJECT", "")
    require(re.fullmatch(r"aicli-a605-[0-9]+", project) is not None,
            "A6-05 requires its disposable CI compose project")
    require(path in ("/__control", "/__requests"), "invalid local mock operation")
    # The mock stays unexposed on the host. Its control API is reached inside its
    # own container, never through a production address, proxy or SQL client.
    script = """import json,sys,urllib.request
method,path,body=json.load(sys.stdin)
opener=urllib.request.build_opener(urllib.request.ProxyHandler({}))
req=urllib.request.Request('http://127.0.0.1:8080'+path,method=method,
    data=None if body is None else json.dumps(body).encode(),
    headers={'Content-Type':'application/json'})
with opener.open(req,timeout=10) as response:
    sys.stdout.write(response.read().decode())
"""
    result = subprocess.run(
        ["docker", "compose", "-p", project, "-f", "tests/aicli-testenv/docker-compose.yml",
         "exec", "-T", "mock-upstream", "python", "-c", script],
        input=json.dumps([method, path, body]), capture_output=True, text=True, timeout=20,
    )
    require(result.returncode == 0, "local mock control failed")
    try:
        return json.loads(result.stdout)
    except ValueError:
        raise AssertionError("local mock returned invalid JSON") from None


class Environment:
    def __init__(self):
        self.api = LocalApi()  # Enforces literal loopback and disables redirects/proxies.
        self.admin = login(self.api, "admin", self.api.secret(os.environ.get(
            "AICLI_TESTENV_ADMIN_PASSWORD", "ChangeMe_for_local_tests_123!")))
        password = self.api.secret(os.environ.get(
            "AICLI_TESTENV_ALICE_PASSWORD", "ChangeMe_for_local_tests_123!"))
        browser = login(self.api, "alice", password)
        _, result = self.api.success("POST", "/api/verify", {
            "method": "password", "scope": "access_token.generate", "password": password,
        }, token=browser)
        proof = self.api.secret(result["data"]["proof_token"])
        _, result = self.api.success("POST", "/api/user/token", token=browser, proof=proof)
        self.access = self.api.secret(result["data"])
        require(self.access.count(".") != 2, "A6-05 requires a real local system access token")
        self.channel_ids = []
        self.token_id = None
        self.original = self.options()

    def options(self):
        _, result = self.api.success("GET", "/api/option/", token=self.admin)
        return {item["key"]: item["value"] for item in result["data"]}

    def option(self, key, value):
        self.api.success("PUT", "/api/option/", {"key": key, "value": value}, token=self.admin)

    def channels(self):
        _, result = self.api.success("GET", "/api/channel/?p=1&page_size=100", token=self.admin)
        return result["data"]["items"]

    def priority(self, channel_id, value):
        self.api.success("PUT", "/api/channel/", {"id": channel_id, "priority": value}, token=self.admin)
        channel = next(item for item in self.channels() if item["id"] == channel_id)
        require(channel.get("priority") == value, "channel priority did not update")

    def setup_channels_and_token(self):
        ratios = json.loads(self.original["ModelRatio"])
        ratios.update({model: 1 for model in MODELS})
        self.option("ModelRatio", json.dumps(ratios))
        for suffix, priority in (("a", 100), ("b", 1)):
            name = "aicli-a605-" + suffix
            self.api.success("POST", "/api/channel/", {"mode": "single", "channel": {
                "type": 1, "name": name, "key": "aicli-local-mock-only",
                "base_url": "http://mock-upstream:8080", "models": ",".join(MODELS),
                "group": "g-cheap", "status": 1, "priority": priority, "auto_ban": 0,
            }}, token=self.admin)
            matches = [item for item in self.channels() if item.get("name") == name]
            require(len(matches) == 1, "missing or ambiguous local affinity channel")
            self.channel_ids.append(matches[0]["id"])
            # rc.34 AddChannel does not rebuild the routing cache; the normal
            # update endpoint does. Confirm the configured priority through HTTP
            # so this newly created fixture channel is immediately selectable.
            self.priority(matches[0]["id"], priority)
        name = "aicli:a605"
        self.api.success("POST", "/api/token/", {"name": name, "group": "g-cheap",
                         "unlimited_quota": True, "expired_time": -1}, token=self.access)
        _, listing = self.api.success("GET", "/api/token/?p=1&page_size=100", token=self.access)
        matches = [item for item in listing["data"]["items"] if item.get("name") == name]
        require(len(matches) == 1, "missing or ambiguous local affinity token")
        self.token_id = matches[0]["id"]
        _, result = self.api.success("POST", "/api/token/batch/keys", {"ids": [self.token_id]}, token=self.access)
        raw = self.api.secret(result["data"]["keys"][str(self.token_id)])
        self.key = self.api.secret(raw if raw.startswith("sk-") else "sk-" + raw)
        self.api.success("DELETE", "/api/option/channel_affinity_cache?all=true", token=self.admin)

    def log(self, request_id):
        require(bool(request_id), "missing affinity request ID")
        path = "/api/log/?type=2&request_id=" + urllib.parse.quote(request_id)
        for _ in range(10):
            _, result = self.api.success("GET", path, token=self.admin)
            rows = [row for row in result["data"]["items"] if row.get("request_id") == request_id]
            if rows:
                require(len(rows) == 1, "ambiguous affinity billing row")
                return rows[0]
            time.sleep(0.05)  # Local asynchronous billing visibility, not startup readiness.
        raise AssertionError("affinity billing row was not available")

    def request(self, path="/v1/chat/completions", model="mock-model-a", session=None):
        body = {"model": model, "messages": [{"role": "user", "content": "affinity fixture"}], "max_tokens": 8}
        if path == "/v1/responses":
            body = {"model": model, "input": "affinity fixture", "prompt_cache_key": "default-responses-session"}
        elif path == "/v1/messages":
            body["metadata"] = {"user_id": "default-messages-session"}
        headers = {"Content-Type": "application/json", "Authorization": "Bearer " + self.key}
        if session is not None:
            headers["X-AICLI-Session"] = session
        req = urllib.request.Request(BASE + path, data=json.dumps(body).encode(), headers=headers)
        try:
            with self.api.opener.open(req, timeout=20) as response:
                require(response.status == 200, "affinity fixture request failed")
                request_id = response.headers.get("X-Oneapi-Request-Id")
                response.read()
        except urllib.error.HTTPError as error:
            # Keep fixture failures diagnosable without exposing response data,
            # request headers, credentials or exception representations.
            with error:
                try:
                    result = json.loads(error.read(16384))
                except (ValueError, UnicodeError):
                    result = {}
            detail = result.get("error", {}) if isinstance(result, dict) else {}
            detail = detail if isinstance(detail, dict) else {}
            message = str(detail.get("message", ""))[:1000]
            for secret in sorted(self.api.secrets, key=len, reverse=True):
                message = message.replace(secret, "[REDACTED]")
            message = re.sub(r"(?i)(?:sk-|Bearer\s+)\S+|[A-Za-z0-9_/-]{24,}", "[REDACTED]", message)
            evidence("fixture_http_error", path=path, model=model,
                     http_status=error.code, message=message)
            raise AssertionError("affinity fixture request failed") from None
        except OSError:
            raise AssertionError("affinity fixture request failed") from None
        return self.log(request_id)

    @staticmethod
    def affinity(row):
        other = row.get("other") or "{}"
        other = json.loads(other) if isinstance(other, str) else other
        return other.get("admin_info", {}).get("channel_affinity") or {}

    def default_snapshot(self):
        snapshot = []
        for path, model, rule in (("/v1/responses", MODELS[1], "codex cli trace"),
                                  ("/v1/messages", MODELS[2], "claude cli trace")):
            self.request(path, model)  # Populate the existing default cache entry.
            row = self.request(path, model)
            affinity = self.affinity(row)
            require(affinity.get("rule_name") == rule, "default rule was not used; A6-05 BLOCKED")
            snapshot.append({"path": path, "model": model, "channel": row["channel"],
                             **{key: affinity.get(key) for key in (
                                 "rule_name", "key_source", "key_key", "key_path", "key_fp", "using_group")}})
        return snapshot

    def restore(self):
        mock_call("POST", "/__control", {})
        for key in ("ModelRatio", "RetryTimes", PREFIX + "rules"):
            self.option(key, self.original[key])
        if self.token_id is not None:
            self.api.success("DELETE", "/api/token/" + str(self.token_id), token=self.access)
        for channel_id in self.channel_ids:
            self.api.success("DELETE", "/api/channel/" + str(channel_id), token=self.admin)


@pytest.fixture(scope="module")
def environment():
    env = Environment()
    try:
        env.setup_channels_and_token()
        env.before = env.default_snapshot()
        original_rules = json.loads(env.original[PREFIX + "rules"])
        env.rule = {"name": RULE, "model_regex": [".*"], "path_regex": [],
                    "key_sources": [{"type": "request_header", "key": "X-AICLI-Session"}],
                    "ttl_seconds": 3600, "skip_retry_on_failure": False,
                    "include_using_group": True, "include_rule_name": True,
                    "include_model_name": False}
        env.option(PREFIX + "rules", json.dumps([env.rule, *copy.deepcopy(original_rules)]))
        actual = json.loads(env.options()[PREFIX + "rules"])
        require(actual[0] == env.rule and actual[1:] == original_rules,
                "new rule did not preserve the original rule list")
        yield env
    finally:
        env.restore()


def test_default_rules_and_independent_profile_pins(environment):
    env = environment
    after = env.default_snapshot()
    require(after == env.before, "no-header default behavior changed; A6-05 BLOCKED")
    _, stats = env.api.success("GET", "/api/option/channel_affinity_cache", token=env.admin)
    require(stats["data"]["by_rule_name"].get(RULE, 0) == 0,
            "new rule captured requests without its header; A6-05 BLOCKED")
    evidence("no_header_snapshot", before=env.before, after=after, identical=True)

    a, b = env.channel_ids
    sessions = ("profile-a:shared-session", "profile-b:shared-session")
    histories = {session: [] for session in sessions}
    histories[sessions[0]].append(env.request(session=sessions[0])["channel"])
    env.priority(a, 1)
    env.priority(b, 100)
    histories[sessions[1]].append(env.request(session=sessions[1])["channel"])
    for _ in range(4):
        for session, expected in zip(sessions, (a, b)):
            row = env.request(session=session)
            histories[session].append(row["channel"])
            affinity = env.affinity(row)
            require(affinity.get("rule_name") == RULE and affinity.get("key_key") == "X-AICLI-Session",
                    "session request did not use the AICLI header rule")
            require(affinity.get("key_fp") == hashlib.sha1(session.encode()).hexdigest()[:8],
                    "profile prefix was not part of the affinity key")
            require(row["channel"] == expected, "profile affinity crossed to another channel")
    require(histories[sessions[0]] == [a] * 5 and histories[sessions[1]] == [b] * 5,
            "five-request session affinity was not stable and independent")
    _, stats = env.api.success("GET", "/api/option/channel_affinity_cache", token=env.admin)
    require(stats["data"]["by_rule_name"].get(RULE) == 2, "expected two independent AICLI cache entries")
    evidence("profile_pins", channels=histories, requests_per_profile=5, independent=True)


def stream(env, session, label):
    body = {"model": "mock-model-a", "messages": [{"role": "user", "content": label}],
            "stream": True, "max_tokens": 8}
    req = urllib.request.Request(BASE + "/v1/chat/completions", data=json.dumps(body).encode(), headers={
        "Content-Type": "application/json", "Authorization": "Bearer " + env.key,
        "X-AICLI-Session": session,
    })
    try:
        response = env.api.opener.open(req, timeout=20)
    except urllib.error.HTTPError as error:
        response = error
    raw = bytearray()
    client_error = None
    with response:
        status = response.status
        content_type = response.headers.get("Content-Type", "")
        try:
            while True:
                part = response.readline()
                if not part:
                    break
                raw.extend(part)
                require(len(raw) <= 1024 * 1024, "local stream exceeded evidence bound")
        except (OSError, http.client.IncompleteRead) as error:
            client_error = type(error).__name__
    parts = []
    errors = done = frames = 0
    for line in raw.decode("utf-8", errors="replace").splitlines():
        if not line.startswith("data:"):
            continue
        value = line[5:].strip()
        if value == "[DONE]":
            done += 1
            continue
        frames += 1
        try:
            event = json.loads(value)
        except ValueError:
            continue
        errors += int(bool(event.get("error")))
        for choice in event.get("choices") or []:
            content = (choice.get("delta") or {}).get("content")
            if isinstance(content, str) and content:
                parts.append(content)
    counts = collections.Counter(parts)
    return {"http_status": status, "is_sse": "text/event-stream" in content_type,
            "client_error": client_error, "data_frames": frames, "done_frames": done,
            "error_frames": errors, "duplicate_content": any(count > 1 for count in counts.values()),
            "mock_prefix_occurrences": "".join(parts).count("mock "),
            "response_fragment_occurrences": "".join(parts).count("response"),
            "text_equals_fixture": "".join(parts) == "mock response"}


@pytest.mark.parametrize("retry_times", [0, 2])
@pytest.mark.parametrize("failure", ["stream_error_event", "fail_after_n_chunks"])
def test_observe_streaming_retry(environment, retry_times, failure):
    env = environment
    label = f"a605-{failure}-{retry_times}"
    session = "stream-profile:" + label
    env.option("RetryTimes", str(retry_times))
    require(env.options()["RetryTimes"] == str(retry_times), "RetryTimes did not update")
    _, runtime = env.api.success("GET", "/api/channel/ops", token=env.admin)
    require(runtime["data"].get("retry_times") == retry_times,
            "runtime RetryTimes does not match the stored option")
    mock_call("POST", "/__control", {})
    healthy = stream(env, session, label)
    require(healthy["http_status"] == 200 and healthy["is_sse"] and healthy["text_equals_fixture"]
            and not healthy["duplicate_content"], "healthy stream calibration failed")
    control = {failure: True if failure == "stream_error_event" else 1}
    controlled = mock_call("POST", "/__control", control)
    require(controlled.get("control") == control, "stream error injection did not take effect")
    before = len(mock_call("GET", "/__requests")["requests"])
    try:
        observed = stream(env, session, label)
        requests = mock_call("GET", "/__requests")["requests"][before:]
        require(bool(requests) and all(request["body"].get("model") == "mock-model-a"
                and request["body"].get("stream") is True for request in requests),
                "injected streaming request never reached the local upstream")
        evidence("stream_retry_observation", retry_times=retry_times, failure=failure,
                 upstream_attempts=len(requests), **observed)
        # This is an observation for H-08, not an assertion that server-side
        # retries are safe or that an incomplete stream must be replayed.
    finally:
        mock_call("POST", "/__control", {})
