"""C1-09: TokenService on a fresh loopback rc.34 environment, default limits."""

import os
from pathlib import Path
import subprocess

from test_access_token import BASE, disposable_user, local_admin, require


def test_rust_managed_tokens_and_independent_devices(disposable_user):
    api, admin, username, user_id, access = disposable_user
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
