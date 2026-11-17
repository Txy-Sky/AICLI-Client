"""C1-09: TokenService on a fresh loopback rc.34 environment, default limits."""

import os
from pathlib import Path
import subprocess

from test_access_token import BASE, disposable_user, local_admin, require


def initialize_seed_channel_cache(api, admin):
    # rc.34 channel creation leaves the in-memory routing cache unchanged.
    # Deleting a disposable initializer calls InitChannelCache synchronously.
    # Do not restart the server or consume the tested model/key retry budget.
    _, before = api.success("GET", "/api/channel/?p=1&page_size=100", token=admin)
    rows = before.get("data", {}).get("items", [])
    require(len(rows) == 3 and {row.get("name") for row in rows} == {
        "aicli-g-cheap", "aicli-g-mid", "aicli-g-pro",
    } and all(row.get("status") == 1 for row in rows), "unexpected seed environment")
    name = "aicli-c109-cache-initializer"
    require(all(row.get("name") != name for row in rows), "initializer already exists")
    api.success("POST", "/api/channel/", {"mode": "single", "channel": {
        "type": 1, "name": name, "key": "not-a-live-credential",
        "base_url": "http://mock-upstream:8080", "models": "mock-model-b",
        "group": "g-pro", "status": 1,
    }}, token=admin)
    _, created = api.success("GET", "/api/channel/?p=1&page_size=100", token=admin)
    initializers = [row for row in created.get("data", {}).get("items", []) if row.get("name") == name]
    require(len(initializers) == 1, "initializer creation failed or ambiguous")
    channel_id = initializers[0].get("id")
    require(isinstance(channel_id, int) and channel_id > 0, "initializer id invalid")
    api.success("DELETE", f"/api/channel/{channel_id}", token=admin)
    _, after = api.success("GET", "/api/channel/?p=1&page_size=100", token=admin)
    states = [(row.get("id"), row.get("name"), row.get("group"), row.get("models"), row.get("status"))
              for row in after.get("data", {}).get("items", [])]
    original = [(row.get("id"), row.get("name"), row.get("group"), row.get("models"), row.get("status"))
                for row in rows]
    require(sorted(states) == sorted(original), "seed channel routing configuration changed")
    print("C1-09 setup: seed channel cache initialized by local initializer create/delete HTTP APIs; original routing state preserved", flush=True)


def test_rust_managed_tokens_and_independent_devices(disposable_user):
    api, admin, username, user_id, access = disposable_user
    initialize_seed_channel_cache(api, admin)
    api.success("POST", "/api/user/manage", {
        "id": user_id, "action": "add_quota", "mode": "override", "value": 1000000,
    }, token=admin)
    root = Path(__file__).resolve().parents[3]
    probe = root / "src-tauri/target/debug/examples/aicli-newapi-probe"
    require(probe.is_file(), "C1-09 Rust probe is not built")
    environment = os.environ.copy()
    environment.pop("AICLI_TEST_ACCESS_TOKEN", None)
    environment.update({
        "AICLI_TESTENV_BASE_URL": BASE,
        "AICLI_LOCAL_TEST_ACCESS_TOKEN": access,
        "AICLI_LOCAL_TEST_USERNAME": username,
        "AICLI_LOCAL_TEST_TOKEN_ONLY": "1",
    })
    result = subprocess.run([str(probe)], env=environment, capture_output=True, text=True, timeout=90)
    expected = "C1-09: PASS; real rc.34 reuse/restrictions; 69 models one key batch; two isolated device HOMEs; both model requests; unbind(false) leaves second device usable; deleted key recovered\n"
    # Do not forward arbitrary subprocess diagnostics to a public job log.
    if result.returncode:
        for line in result.stderr.splitlines():
            if line.startswith("C1-04: FAIL; ") and all(value not in line for value in api.secrets):
                print(line, flush=True)
    require(result.returncode == 0, "C1-09 Rust probe failed")
    require(result.stdout == expected and result.stderr == "", "C1-09 probe returned unexpected output")
    print(expected.strip(), flush=True)
