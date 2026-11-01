#!/usr/bin/env python3
"""A5-02 read-only account capability probe.

The access token is read only from AICLI_TEST_ACCESS_TOKEN. Responses are
reduced to capability metadata; account names, keys, and response bodies are
never written to the report.
"""

import argparse
import json
import os
from pathlib import Path
import ssl
import urllib.error
import urllib.parse
import urllib.request

LINES = {
    "main": "https://aicliapi.com",
    "global": "https://aicli.qzz.io",
    "apac": "https://api.aicli.qzz.io",
}
READ_PATHS = {
    "/api/user/self",
    "/api/user/self/groups",
    "/api/token/auto-groups",
    "/api/log/self",
    "/api/pricing",
}


class ProbeBlocked(Exception):
    pass


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def opener():
    context = ssl.create_default_context()
    return urllib.request.build_opener(
        urllib.request.ProxyHandler({}),
        NoRedirect(),
        urllib.request.HTTPSHandler(context=context),
    )


def request(line, path, token):
    if line not in LINES or not path.startswith("/"):
        raise ProbeBlocked("unapproved_destination")
    route = urllib.parse.urlsplit(path).path
    if route not in READ_PATHS:
        raise ProbeBlocked("unapproved_operation")
    return urllib.request.Request(
        LINES[line] + path,
        headers={
            "Authorization": "Bearer " + token,
            "User-Agent": "AICLI-Client/account-probe",
            "Accept-Encoding": "identity",
        },
    )


def fetch(line, path, token):
    try:
        with opener().open(request(line, path, token), timeout=30) as response:
            body = json.load(response)
            return response.status, body
    except urllib.error.HTTPError as error:
        error.close()
        return error.code, None
    except (OSError, urllib.error.URLError, ValueError):
        return None, None


def group_names(data):
    if not isinstance(data, dict):
        return set()
    return {str(name) for name in data}


def model_group_counts(pricing, available):
    if not isinstance(pricing, list):
        return []
    rows = []
    for model in pricing:
        if not isinstance(model, dict) or not isinstance(model.get("model_name"), str):
            continue
        enabled = model.get("enable_groups")
        if not isinstance(enabled, list):
            continue
        usable = sorted({str(group) for group in enabled} & available)
        if usable:
            rows.append({"model": model["model_name"], "usable_group_count": len(usable)})
    return rows


def run(output):
    token = os.environ.get("AICLI_TEST_ACCESS_TOKEN", "").strip()
    if not token or token.startswith("sk-"):
        raise ProbeBlocked("missing_or_wrong_access_token")
    if os.environ.get("HOME") != os.environ.get("CC_SWITCH_TEST_HOME"):
        raise ProbeBlocked("isolated_home_required")

    result = {"item": "A5-02", "lines": {}, "conclusion": "inconclusive"}
    status, account = fetch("main", "/api/user/self", token)
    status_groups, groups_data = fetch("main", "/api/user/self/groups", token)
    status_auto, auto_data = fetch("main", "/api/token/auto-groups", token)
    status_logs, logs_data = fetch("main", "/api/log/self?p=1&page_size=1", token)
    status_pricing, pricing_data = fetch("main", "/api/pricing", token)

    result["lines"]["main"] = {
        "user_self_http": status,
        "groups_http": status_groups,
        "auto_groups_http": status_auto,
        "logs_http": status_logs,
        "pricing_http": status_pricing,
    }
    available = group_names(groups_data.get("data") if isinstance(groups_data, dict) else None)
    pricing = pricing_data.get("data") if isinstance(pricing_data, dict) else None
    auto = auto_data.get("data") if isinstance(auto_data, dict) else None
    max_count = auto.get("max_count") if isinstance(auto, dict) else None
    result["account"] = {
        "quota_present": isinstance(account, dict) and isinstance(account.get("data", {}).get("quota"), int),
        "available_groups": sorted(available),
        "auto_available": "auto" in available,
        "auto_max_count": max_count if isinstance(max_count, int) else None,
        "model_group_counts": model_group_counts(pricing, available),
        "max_model_group_count": max(
            (row["usable_group_count"] for row in model_group_counts(pricing, available)),
            default=0,
        ),
        "logs_shape_valid": isinstance(logs_data, dict) and isinstance(logs_data.get("data"), dict),
    }
    all_200 = all(value == 200 for value in result["lines"]["main"].values())
    max_ok = isinstance(max_count, int) and max_count >= result["account"]["max_model_group_count"]
    if all_200 and result["account"]["auto_available"] and max_ok:
        result["conclusion"] = "pass"
    elif all_200:
        result["conclusion"] = "owner_decision_required"

    serialized = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    if token in serialized or "sk-" in serialized:
        raise ProbeBlocked("credential_in_report")
    output.parent.mkdir(parents=True, exist_ok=True)
    fd = os.open(output, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as stream:
        stream.write(serialized)
    print(json.dumps({"item": "A5-02", "conclusion": result["conclusion"], "all_http_200": all_200}))
    return 0 if result["conclusion"] == "pass" else 2


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        return run(args.output)
    except ProbeBlocked as error:
        print(json.dumps({"item": "A5-02", "blocked": str(error)}))
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
