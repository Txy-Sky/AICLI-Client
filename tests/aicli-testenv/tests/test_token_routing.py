"""A6-03 routing checks against the isolated rc.34 HTTP API."""

import json
import os
import time
import urllib.error
import urllib.request

import pytest


BASE = os.environ.get("AICLI_TESTENV_BASE_URL", "http://127.0.0.1:13000").rstrip("/")
USERNAME = os.environ.get("AICLI_TESTENV_ALICE_USERNAME", "alice")
PASSWORD = os.environ.get("AICLI_TESTENV_ALICE_PASSWORD", "ChangeMe_for_local_tests_123!")


def call(method, path, body=None, token=None):
    data = None if body is None else json.dumps(body).encode()
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = "Bearer " + token
    request = urllib.request.Request(BASE + path, data=data, headers=headers, method=method)
    with urllib.request.urlopen(request, timeout=15) as response:
        return response.status, dict(response.headers), json.loads(response.read())


def login():
    _, _, result = call("POST", "/api/user/login", {"username": USERNAME, "password": PASSWORD})
    return result["data"]["access_token"]


def token_id_and_key(access_token, name):
    _, _, result = call("GET", "/api/token/?p=1&page_size=100", token=access_token)
    item = next(item for item in result.get("data", {}).get("items", []) if item.get("name") == name)
    _, _, keys = call("POST", "/api/token/batch/keys", {"ids": [item["id"]]}, token=access_token)
    key = keys.get("data", {}).get("keys", {}).get(str(item["id"]))
    assert key, keys
    return item["id"], key


def request_model(key, model):
    request = urllib.request.Request(
        BASE + "/v1/chat/completions",
        data=json.dumps({"model": model, "messages": [{"role": "user", "content": "route"}], "max_tokens": 8}).encode(),
        headers={"Authorization": "Bearer " + key, "Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=15) as response:
        body = json.loads(response.read())
        return dict(response.headers), body


def log_for(access_token, request_id):
    for _ in range(10):
        _, _, result = call("GET", "/api/log/self?request_id=" + request_id, token=access_token)
        rows = result.get("data", {}).get("items", [])
        if rows:
            return rows[0]
        time.sleep(0.5)
    pytest.fail("request log was not available")


def test_auto_group_order_and_fallback():
    access_token = login()
    created = []
    try:
        specs = {
            "aicli:a6-t1": ["g-cheap", "g-mid"],
            "aicli:a6-t2": ["g-mid", "g-cheap"],
            "aicli:a6-t3": ["g-cheap", "g-pro"],
        }
        keys = {}
        for name, groups in specs.items():
            _, _, result = call("POST", "/api/token/", {"name": name, "group": "auto", "auto_groups": groups, "cross_group_retry": True, "unlimited_quota": True, "expired_time": -1}, token=access_token)
            assert result.get("success") is not False, result
            token_id, key = token_id_and_key(access_token, name)
            created.append(token_id)
            keys[name] = key

        headers, _ = request_model(keys["aicli:a6-t1"], "mock-model-a")
        request_id = headers.get("X-Oneapi-Request-Id")
        assert request_id
        row = log_for(access_token, request_id)
        assert row.get("group") == "g-cheap", row

        headers, _ = request_model(keys["aicli:a6-t2"], "mock-model-a")
        row = log_for(access_token, headers["X-Oneapi-Request-Id"])
        assert row.get("group") == "g-mid", row

        headers, _ = request_model(keys["aicli:a6-t3"], "mock-model-b")
        row = log_for(access_token, headers["X-Oneapi-Request-Id"])
        assert row.get("group") == "g-pro", row
        assert float(row.get("quota", 0)) > 0, row
    finally:
        for token_id in created:
            try:
                call("DELETE", "/api/token/" + str(token_id), token=access_token)
            except (OSError, urllib.error.HTTPError):
                pass
