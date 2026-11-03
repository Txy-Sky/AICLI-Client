"""C1-06: exercise cache loading and request budget against isolated rc.34."""

import os
from pathlib import Path
import subprocess
import tempfile

from test_access_token import BASE, disposable_user, local_admin, require


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
    with tempfile.TemporaryDirectory(prefix="aicli-cache-home-") as test_home:
        environment.update({"HOME": test_home, "CC_SWITCH_TEST_HOME": test_home,
                            "XDG_CONFIG_HOME": test_home + "/.config",
                            "XDG_DATA_HOME": test_home + "/.local/share",
                            "XDG_CACHE_HOME": test_home + "/.cache"})
        result = subprocess.run([str(probe)], env=environment, capture_output=True, text=True, timeout=750)
    if result.returncode:
        for line in result.stderr.splitlines():
            if line.startswith("C1-06: FAIL;") and all(value not in line for value in api.secrets):
                print(line, flush=True)
    require(result.returncode == 0, "C1-06 cache probe failed")
    require(result.stderr == "", "C1-06 probe wrote unexpected diagnostics")
    require(result.stdout.startswith("C1-06: PASS; observed 601s including periodic refresh; audited /api/* request count="),
            "C1-06 probe returned unexpected output")
    print(result.stdout.strip(), flush=True)
