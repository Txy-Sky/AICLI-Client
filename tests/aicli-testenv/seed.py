#!/usr/bin/env python3
"""Idempotently seed the local rc.34 test environment through HTTP APIs."""

import json
import os
import sys
import time
import urllib.error
import urllib.request

BASE = os.environ.get("AICLI_TESTENV_BASE_URL", "http://127.0.0.1:13000").rstrip("/")
ADMIN = os.environ.get("AICLI_TESTENV_ADMIN_USERNAME", "admin")
ADMIN_PASSWORD = os.environ.get("AICLI_TESTENV_ADMIN_PASSWORD", "ChangeMe_for_local_tests_123!")
USER_PASSWORD = os.environ.get("AICLI_TESTENV_ALICE_PASSWORD", "ChangeMe_for_local_tests_123!")


def call(method, path, body=None, token=None):
    data = None if body is None else json.dumps(body).encode()
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = "Bearer " + token
    request = urllib.request.Request(BASE + path, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=15) as response:
            result = json.loads(response.read())
            if result.get("success") is False:
                raise RuntimeError(f"{method} {path}: {result.get('message', 'request failed')}")
            return result
    except urllib.error.HTTPError as error:
        raise RuntimeError(f"{method} {path}: HTTP {error.code}") from error


def wait_ready():
    for _ in range(60):
        try:
            if call("GET", "/api/status").get("success"):
                return
        except (OSError, RuntimeError, ValueError):
            time.sleep(2)
    raise RuntimeError("new-api did not become ready")


def login(username, password):
    result = call("POST", "/api/user/login", {"username": username, "password": password})
    return result["data"]["access_token"]


def find_user(token, username):
    result = call("GET", "/api/user/?keyword=" + username + "&p=1&page_size=20", token=token)
    for user in result.get("data", {}).get("items", []):
        if user.get("username") == username:
            return user
    return None


def ensure_user(token, username):
    user = find_user(token, username)
    if user:
        return user["id"]
    call("POST", "/api/user/", {"username": username, "password": USER_PASSWORD, "display_name": username, "role": 1}, token=token)
    user = find_user(token, username)
    if not user:
        raise RuntimeError(f"created user {username} was not returned by the admin API")
    return user["id"]


def ensure_option(token, key, value):
    call("PUT", "/api/option/", {"key": key, "value": value}, token=token)


def ensure_channel(token, name, group, models):
    channels = call("GET", "/api/channel/?p=1&page_size=100", token=token).get("data", {}).get("items", [])
    if any(channel.get("name") == name for channel in channels):
        return
    call("POST", "/api/channel/", {"mode": "single", "channel": {"type": 1, "name": name, "key": "aicli-test-key", "base_url": "http://mock-upstream:8080", "models": ",".join(models), "group": group, "status": 1}}, token=token)


def main():
    wait_ready()
    try:
        admin_token = login(ADMIN, ADMIN_PASSWORD)
    except RuntimeError:
        call("POST", "/api/setup", {"username": ADMIN, "password": ADMIN_PASSWORD, "confirmPassword": ADMIN_PASSWORD, "SelfUseModeEnabled": False, "DemoSiteEnabled": False})
        admin_token = login(ADMIN, ADMIN_PASSWORD)
    ensure_user(admin_token, "alice")
    ensure_user(admin_token, "bob")
    ensure_option(admin_token, "GroupRatio", json.dumps({"g-cheap": 0.5, "g-mid": 1, "g-pro": 2, "auto": 1}))
    ensure_option(admin_token, "ModelRatio", json.dumps({"mock-model-a": 1, "mock-model-b": 1}))
    ensure_option(admin_token, "UserUsableGroups", json.dumps({"auto": "自动分组", "g-cheap": "低成本", "g-mid": "标准", "g-pro": "高性能"}, ensure_ascii=False))
    ensure_option(admin_token, "MaxTokenAutoGroups", "10")
    ensure_option(admin_token, "LogConsumeEnabled", "true")
    ensure_channel(admin_token, "aicli-g-cheap", "g-cheap", ["mock-model-a"])
    ensure_channel(admin_token, "aicli-g-mid", "g-mid", ["mock-model-a"])
    ensure_channel(admin_token, "aicli-g-pro", "g-pro", ["mock-model-b"])
    print("A6-01 seed: PASS (admin, alice, bob, options, and channels are idempotently present)")


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, ValueError, KeyError) as error:
        print(f"A6-01 seed: FAIL: {error}", file=sys.stderr)
        raise SystemExit(1)
