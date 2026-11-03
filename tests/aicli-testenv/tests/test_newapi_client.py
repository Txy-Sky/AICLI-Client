"""C1-04: execute the Rust client against a disposable loopback rc.34 user."""

import os
from pathlib import Path
import subprocess

from test_access_token import BASE, disposable_user, local_admin, require


def test_rust_newapi_client_all_wrappers(disposable_user):
    api, admin, username, user_id, access = disposable_user
    api.success("POST", "/api/user/manage", {
        "id": user_id, "action": "add_quota", "mode": "override", "value": 1000000,
    }, token=admin)
    root = Path(__file__).resolve().parents[3]
    probe = root / "src-tauri/target/debug/examples/aicli-newapi-probe"
    require(probe.is_file(), "C1-04 Rust probe is not built")
    environment = os.environ.copy()
    environment.pop("AICLI_TEST_ACCESS_TOKEN", None)
    environment.update({
        "AICLI_TESTENV_BASE_URL": BASE,
        "AICLI_LOCAL_TEST_ACCESS_TOKEN": access,
        "AICLI_LOCAL_TEST_USERNAME": username,
    })
    result = subprocess.run([str(probe)], env=environment, capture_output=True, text=True, timeout=90)
    expected = "C1-04: PASS; all 11 wrappers; disposable rc.34 user; mock bill/stat/count match; token deleted; invalid access mapped\n"
    # Do not forward arbitrary subprocess diagnostics to a public job log.
    if result.returncode:
        for line in result.stderr.splitlines():
            if line.startswith("C1-04: FAIL; ") and all(value not in line for value in api.secrets):
                print(line, flush=True)
    require(result.returncode == 0, "C1-04 Rust probe failed")
    require(result.stdout == expected and result.stderr == "", "C1-04 probe returned unexpected output")
    print(expected.strip(), flush=True)
