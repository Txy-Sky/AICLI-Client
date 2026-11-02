"""A6-03 routing checks against the isolated rc.34 HTTP API."""

import json
import math
import os
import time
import urllib.parse

import pytest

from test_access_token import LocalApi, login as browser_login, require


API = LocalApi()
USERNAME = os.environ.get("AICLI_TESTENV_ALICE_USERNAME", "alice")
PASSWORD = os.environ.get("AICLI_TESTENV_ALICE_PASSWORD", "ChangeMe_for_local_tests_123!")


def call(method, path, body=None, token=None):
    status, headers, result = API.call(method, path, body, token)
    require(status == 200, method + " " + path + " returned a non-200 response")
    if path.startswith("/api/"):
        require(result.get("success") is True, method + " " + path + " failed")
    return status, headers, result


def login():
    # Login's JWT is only a bootstrap credential. All acceptance requests below
    # use the real local system access token issued after a scoped proof.
    password = API.secret(PASSWORD)
    session = browser_login(API, USERNAME, password)
    _, verified = API.success("POST", "/api/verify", {
        "method": "password", "scope": "access_token.generate", "password": password,
    }, token=session)
    proof = API.secret(verified["data"]["proof_token"])
    _, generated = API.success("POST", "/api/user/token", token=session, proof=proof)
    access = API.secret(generated["data"])
    require(access.count(".") != 2, "expected a system access token, not a login JWT")
    _, account = API.success("GET", "/api/user/self", token=access)
    require(account.get("data", {}).get("username") == USERNAME, "wrong routing test user")
    print("A6-03 credential_kind=system_access_token", flush=True)
    return access


def token_id_and_key(access_token, name):
    _, _, result = call("GET", "/api/token/?p=1&page_size=100", token=access_token)
    item = next(item for item in result.get("data", {}).get("items", []) if item.get("name") == name)
    _, _, keys = call("POST", "/api/token/batch/keys", {"ids": [item["id"]]}, token=access_token)
    key = keys.get("data", {}).get("keys", {}).get(str(item["id"]))
    raw = API.secret(key)
    key = API.secret(raw if raw.startswith("sk-") else "sk-" + raw)
    return item["id"], key


def request_model(key, model):
    # Equal sufficiently long prompts keep integer quota rounding below the
    # explicit 2% comparison tolerance for these small local mock requests.
    _, headers, body = call("POST", "/v1/chat/completions", {
        "model": model,
        "messages": [{"role": "user", "content": "route " * 128}],
        "max_tokens": 8,
    }, token=key)
    return headers, body


def log_for(access_token, request_id):
    for _ in range(10):
        _, _, result = call("GET", "/api/log/self?request_id=" + urllib.parse.quote(request_id), token=access_token)
        rows = result.get("data", {}).get("items", [])
        if rows:
            matching = [row for row in rows if row.get("request_id") == request_id]
            require(len(matching) == 1, "request ID did not identify one billing row")
            return matching[0]
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
            require(result.get("success") is True, "model token creation failed")
            token_id, key = token_id_and_key(access_token, name)
            created.append(token_id)
            keys[name] = key

        billed = {}
        for name, model, group in (
            ("aicli:a6-t1", "mock-model-a", "g-cheap"),
            ("aicli:a6-t2", "mock-model-a", "g-mid"),
            ("aicli:a6-t3", "mock-model-b", "g-pro"),
        ):
            headers, _ = request_model(keys[name], model)
            request_id = headers.get("X-Oneapi-Request-Id")
            require(bool(request_id), "missing X-Oneapi-Request-Id")
            row = log_for(access_token, request_id)
            require(row.get("group") == group, "model routed to the wrong group")
            quota = float(row.get("quota", 0))
            require(math.isfinite(quota) and quota > 0, "missing positive billed quota")
            billed[name] = quota
            print("A6-03 " + json.dumps({
                "token_name": name, "model": model, "request_id": request_id,
                "group": row["group"], "quota": quota,
            }), flush=True)
        ratio = billed["aicli:a6-t2"] / billed["aicli:a6-t1"]
        print(f"A6-03 quota_ratio_mid_over_cheap={ratio:.6f}", flush=True)
        require(math.isclose(ratio, 2.0, rel_tol=0.02), "quota ratio is not approximately 1:2")
    finally:
        for token_id in created:
            call("DELETE", "/api/token/" + str(token_id), token=access_token)
