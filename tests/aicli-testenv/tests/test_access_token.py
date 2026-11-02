"""A6-04: real system-access-token checks, exclusively on a disposable local API."""

import json
import os
import secrets
import sys
import time
import urllib.error
import urllib.parse
import urllib.request


def local_base(value):
    try:
        url = urllib.parse.urlsplit(value)
        valid = (
            url.scheme in ("http", "https")
            and url.hostname in ("127.0.0.1", "localhost")
            and url.username is None
            and url.password is None
            and url.path in ("", "/")
            and not url.query
            and not url.fragment
            and (url.port is None or 0 < url.port < 65536)
        )
    except ValueError:
        valid = False
    if not valid:
        raise ValueError("A6-04 only accepts 127.0.0.1 or localhost API origins")
    return value.rstrip("/")


try:
    BASE = local_base(os.environ.get("AICLI_TESTENV_BASE_URL", "http://127.0.0.1:13000"))
except ValueError as error:
    raise SystemExit(str(error)) from None

# Validate the target before loading pytest, including when invoked as a script.
import pytest


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise ValueError("A6-04 refuses HTTP redirects")


class LocalApi:
    def __init__(self):
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
        self.secrets = []

    def secret(self, value):
        require(isinstance(value, str) and bool(value), "missing local credential")
        self.secrets.append(value)
        return value

    def call(self, method, path, body=None, token=None, proof=None):
        local_base(BASE)
        require(path.startswith("/") and not path.startswith("//"), "invalid API path")
        headers = {"Content-Type": "application/json", "Accept-Language": "zh-CN"}
        if token:
            headers["Authorization"] = "Bearer " + token
        if proof:
            headers["X-Security-Proof"] = proof
        request = urllib.request.Request(
            BASE + path, method=method, headers=headers,
            data=None if body is None else json.dumps(body).encode(),
        )
        try:
            response = self.opener.open(request, timeout=15)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            status, response_headers = response.code, response.headers
            try:
                result = json.loads(response.read())
            except (ValueError, UnicodeError):
                raise AssertionError("API returned a non-JSON response") from None
        require(isinstance(result, dict), "API response is not an object")
        return status, response_headers, result

    def success(self, method, path, body=None, token=None, proof=None):
        status, headers, result = self.call(method, path, body, token, proof)
        require(status == 200 and result.get("success") is True, method + " " + path + " failed")
        return headers, result

    def report(self, method, path, status, result, **checks):
        # Never serialize data: it can contain a newly issued credential on failure.
        record = {"method": method, "path": path, "status": status,
                  "success": result.get("success"), "message": result.get("message"),
                  "code": result.get("code"), **checks}
        output = json.dumps(record, ensure_ascii=False)
        for value in sorted(self.secrets, key=len, reverse=True):
            output = output.replace(value, "[REDACTED]")
        print("A6-04 " + output, flush=True)


def require(condition, message):
    # Fixed diagnostic messages keep credentials out of pytest assertion rendering.
    if not condition:
        raise AssertionError(message)


def login(api, username, password):
    _, result = api.success("POST", "/api/user/login", {"username": username, "password": password})
    return api.secret(result["data"]["access_token"])


def find_user(api, admin, username):
    _, result = api.success("GET", "/api/user/?keyword=" + urllib.parse.quote(username) + "&p=1&page_size=100", token=admin)
    return next((user for user in result["data"]["items"] if user.get("username") == username), None)


@pytest.fixture(scope="module")
def local_admin():
    api = LocalApi()
    admin = login(api, os.environ.get("AICLI_TESTENV_ADMIN_USERNAME", "admin"),
                  api.secret(os.environ.get("AICLI_TESTENV_ADMIN_PASSWORD", "ChangeMe_for_local_tests_123!")))
    return api, admin


@pytest.fixture
def disposable_user(local_admin):
    api, admin = local_admin
    username = "a604_" + secrets.token_hex(5)
    password = api.secret(secrets.token_urlsafe(24))
    api.success("POST", "/api/user/", {"username": username, "password": password, "display_name": username, "role": 1}, token=admin)
    user = find_user(api, admin, username)
    require(user is not None, "disposable user was not created")
    session = login(api, username, password)
    _, verified = api.success("POST", "/api/verify", {"method": "password", "scope": "access_token.generate", "password": password}, token=session)
    proof = api.secret(verified["data"]["proof_token"])
    _, generated = api.success("POST", "/api/user/token", token=session, proof=proof)
    access = api.secret(generated["data"])
    require(access.count(".") != 2, "expected a system access token, not a login JWT")
    # Each test gets a new user, never alice or bob. Compose destroys all users.
    return api, admin, username, user["id"], access


@pytest.mark.parametrize("method,path", [("POST", "/api/user/token"), ("DELETE", "/api/user/self")])
def test_security_operations_are_rejected(disposable_user, method, path):
    api, admin, username, user_id, access = disposable_user
    _, before = api.success("GET", "/api/user/self", token=access)
    require(before["data"]["id"] == user_id, "wrong disposable user before operation")
    status, _, result = api.call(method, path, token=access)
    # Always probe both postconditions, even if an operation unexpectedly succeeded.
    self_status, _, after = api.call("GET", "/api/user/self", token=access)
    remaining = find_user(api, admin, username)
    valid = self_status == 200 and after.get("success") is True and after.get("data", {}).get("id") == user_id
    exists = remaining is not None and remaining.get("id") == user_id
    api.report(method, path, status, result, original_token_valid=valid, user_exists=exists)
    require(result.get("success") is not True, "security operation unexpectedly succeeded; A6-04 BLOCKED")
    require(valid, "original system access token no longer works; A6-04 BLOCKED")
    require(exists, "disposable user disappeared; A6-04 BLOCKED")
    require(status in (200, 401, 403) and result.get("code") in ("SECURITY_PROOF_INVALID", "SECURITY_PROOF_REQUIRED"), "operation was not rejected for Security Proof")


def test_account_token_lifecycle_and_credential_boundaries(disposable_user):
    api, admin, _, user_id, access = disposable_user
    api.success("POST", "/api/user/manage", {"id": user_id, "action": "add_quota", "mode": "override", "value": 1000000}, token=admin)
    for path in ("/api/user/self", "/api/user/self/groups", "/api/token/", "/api/token/auto-groups", "/api/pricing", "/api/log/self?request_id=", "/api/log/self/stat"):
        api.success("GET", path, token=access)
    status, _, _ = api.call("GET", "/v1/models", token=access)
    require(status == 401, "system access token must be rejected by model endpoint")
    _, created = api.success("POST", "/api/token/", {"name": "aicli:test", "group": "auto", "auto_groups": ["g-cheap", "g-mid"], "cross_group_retry": True, "unlimited_quota": True, "expired_time": -1}, token=access)
    require(not any(key in created for key in ("key", "id")), "create response unexpectedly includes id/key")
    require(not isinstance(created.get("data"), dict) or not any(key in created["data"] for key in ("key", "id")), "create response data unexpectedly includes id/key")
    _, listing = api.success("GET", "/api/token/?p=1&page_size=100", token=access)
    items = [item for item in listing["data"]["items"] if item.get("name") == "aicli:test"]
    require(len(items) == 1, "token name is missing or ambiguous")
    item = items[0]
    token_id = item["id"]
    try:
        groups = item.get("auto_groups")
        if isinstance(groups, str):
            groups = json.loads(groups)
        require(groups == ["g-cheap", "g-mid"], "auto_groups order was not preserved")
        _, keys = api.success("POST", "/api/token/batch/keys", {"ids": [token_id]}, token=access)
        raw = api.secret(keys["data"]["keys"][str(token_id)])
        key = api.secret(raw if raw.startswith("sk-") else "sk-" + raw)
        print("A6-04 batch_keys_has_sk_prefix=" + str(raw.startswith("sk-")).lower(), flush=True)
        status, _, _ = api.call("GET", "/api/user/self", token=key)
        require(status == 401, "model key must be rejected by account endpoint")
        status, _, _ = api.call("GET", "/v1/models", token=key)
        require(status == 200, "new model key is not usable")
        status, headers, _ = api.call("POST", "/v1/chat/completions", {"model": "mock-model-a", "messages": [{"role": "user", "content": "access boundary"}], "max_tokens": 8}, token=key)
        require(status == 200, "model request failed")
        request_id = headers.get("X-Oneapi-Request-Id")
        require(bool(request_id), "missing upstream request id")
        for _ in range(10):
            _, logs = api.success("GET", "/api/log/self?request_id=" + urllib.parse.quote(request_id), token=access)
            if logs.get("data", {}).get("items"):
                break
            time.sleep(0.5)
        require(bool(logs.get("data", {}).get("items")), "request log was not returned")
    finally:
        api.success("DELETE", "/api/token/" + str(token_id), token=access)
    started = time.monotonic()
    for attempt in range(1, 21):
        status, _, _ = api.call("GET", "/v1/models", token=key)
        if status == 401:
            break
        require(status == 200, "unexpected status during token revocation check")
        time.sleep(0.25)
    elapsed = time.monotonic() - started
    print(f"A6-04 deleted_token_status={status} attempts={attempt} delay_seconds={elapsed:.6f}", flush=True)
    require(status == 401, "deleted token remains usable; A6-04 BLOCKED")


if __name__ == "__main__":
    sys.exit(pytest.main(["-q", "-s", "--tb=short", __file__]))
