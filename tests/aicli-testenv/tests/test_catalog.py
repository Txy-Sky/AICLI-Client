"""C1-07: compare the Rust catalog with actual loopback rc.34 permissions."""

import json
import os
from pathlib import Path
import subprocess
import urllib.parse

import pytest

from test_access_token import BASE, disposable_user, local_admin, require


def project(groups, pricing, api):
    root = Path(__file__).resolve().parents[3]
    probe = root / "src-tauri/target/debug/examples/aicli-catalog-probe"
    require(probe.is_file(), "C1-07 Rust probe is not built")
    environment = os.environ.copy()
    environment.pop("AICLI_TEST_ACCESS_TOKEN", None)
    result = subprocess.run(
        [str(probe)], input=json.dumps({"groups": groups, "pricing": pricing}),
        env=environment, capture_output=True, text=True, timeout=30,
    )
    require(result.returncode == 0 and not result.stderr, "C1-07 Rust projection failed")
    require(all(secret not in result.stdout for secret in api.secrets), "C1-07 credential in output")
    try:
        return json.loads(result.stdout)
    except ValueError:
        raise AssertionError("C1-07 invalid probe output") from None


@pytest.mark.parametrize("special", [False, True])
def test_catalog_matches_group_models_and_pricing(disposable_user, special):
    api, admin, _, user_id, access = disposable_user
    _, account = api.success("GET", "/api/user/self", token=access)
    option = "group_ratio_setting.group_special_usable_group"
    _, options = api.success("GET", "/api/option/", token=admin)
    previous = next((item["value"] for item in options["data"] if item["key"] == option), "{}")
    usable_before = next(item["value"] for item in options["data"] if item["key"] == "UserUsableGroups")
    try:
        if special:
            usable = json.loads(usable_before)
            usable.pop("g-pro", None)
            api.success("PUT", "/api/option/", {"key": "UserUsableGroups", "value": json.dumps(usable)}, token=admin)
            _, before_special = api.success("GET", "/api/user/self/groups", token=access)
            require("g-pro" not in before_special["data"], "C1-07 added-group fixture already available")
            settings = json.loads(previous)
            settings[account["data"]["group"]] = {"-:g-cheap": "removed", "+:g-pro": "added"}
            api.success("PUT", "/api/option/", {"key": option, "value": json.dumps(settings)}, token=admin)
        _, groups = api.success("GET", "/api/user/self/groups", token=access)
        _, pricing = api.success("GET", "/api/pricing", token=access)
        catalog = project(groups, pricing, api)
        expected_groups = set(groups["data"]) - {"auto"}
        require(set(catalog["groups"]) == expected_groups, "C1-07 groups differ")
        require(catalog["auto_available"] == ("auto" in groups["data"]), "C1-07 auto differs")
        require("svip" not in catalog["groups"], "C1-07 unavailable group leaked")
        require(bool(catalog["models"]), "C1-07 fixture has no models")
        pricing_models = {row["model_name"] for row in pricing["data"]}
        for group in sorted(expected_groups):
            _, models = api.success("GET", "/api/user/models?group=" + urllib.parse.quote(group, safe=""), token=access)
            expected = set(models["data"]) & pricing_models
            actual = {name for name, model in catalog["models"].items() if group in model["groups"]}
            require(actual == expected, "C1-07 per-group model set differs")
        if special:
            require("g-cheap" not in catalog["groups"] and "g-pro" in catalog["groups"], "C1-07 special rules not applied")
        # Simulate forbidden pricing rows even if the server normally filters them.
        forbidden = dict(pricing["data"][0], model_name="c107-forbidden", enable_groups=["svip"])
        synthetic = dict(pricing, data=pricing["data"] + [forbidden])
        require("c107-forbidden" not in project(groups, synthetic, api)["models"], "C1-07 forbidden model leaked")
        print("C1-07: PASS; " + ("special -:/+: permissions" if special else "default permissions") + "; every group matches HTTP models intersect pricing; svip excluded", flush=True)
    finally:
        if special:
            api.success("PUT", "/api/option/", {"key": option, "value": previous}, token=admin)
            api.success("PUT", "/api/option/", {"key": "UserUsableGroups", "value": usable_before}, token=admin)
