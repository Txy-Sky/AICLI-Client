"""C1-06: exercise cache loading and request budget against isolated rc.34."""

import os
from pathlib import Path
import subprocess

from test_access_token import BASE, disposable_user, require


def test_cache_first_refresh_stays_within_cold_start_request_budget(disposable_user):
    api, _, username, _, access = disposable_user
    root = Path(__file__).resolve().parents[3]
    probe = root / "src-tauri/target/debug/examples/aicli-cache-probe"
    require(probe.is_file(), "C1-06 Rust cache probe is not built")
    environment = os.environ.copy()
    environment.pop("AICLI_TEST_ACCESS_TOKEN", None)
    environment.update({
        "AICLI_TESTENV_BASE_URL": BASE,
        "AICLI_LOCAL_TEST_ACCESS_TOKEN": access,
        "AICLI_LOCAL_TEST_USERNAME": username,
    })
    result = subprocess.run([str(probe)], env=environment, capture_output=True, text=True, timeout=120)
    if result.returncode:
        for line in result.stderr.splitlines():
            if line.startswith("C1-06: FAIL;"):
                print(line, flush=True)
    require(result.returncode == 0, "C1-06 cache probe failed")
    require(result.stderr == "", "C1-06 probe wrote unexpected diagnostics")
    require(result.stdout.startswith("C1-06: PASS; cold-start audited /api/* request count="),
            "C1-06 probe returned unexpected output")
    print(result.stdout.strip(), flush=True)
