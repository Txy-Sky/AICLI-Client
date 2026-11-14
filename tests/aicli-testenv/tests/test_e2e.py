"""K-01/K-02/K-03: the whole client flow against a disposable loopback rc.34 user.

The Rust probe binds, syncs the catalog, saves a route profile, takes over
OpenCode (which creates the managed sk- token), sends one request through the
local proxy, waits for the server bill and unbinds. It runs under a throwaway
HOME that already holds a ~/.cc-switch folder, which must come out unchanged.
"""

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

from test_access_token import BASE, disposable_user, local_admin, require

ROOT = Path(__file__).resolve().parents[3]
PROBE = ROOT / "src-tauri/target/debug/examples/aicli-e2e-probe"
SCAN = ROOT / "scripts/aicli-secret-scan.sh"


def tree_digest(folder):
    digest = hashlib.sha256()
    for path in sorted(folder.rglob("*")):
        digest.update(str(path.relative_to(folder)).encode() + b"\0")
        if path.is_file() and not path.is_symlink():
            digest.update(path.read_bytes())
        digest.update(oct(path.lstat().st_mode).encode() + b"\0")
    return digest.hexdigest()


def plant_cc_switch(home):
    folder = home / ".cc-switch"
    (folder / "backups").mkdir(parents=True)
    (folder / "settings.json").write_text('{"language":"zh","proxyPort":15721}\n')
    (folder / "cc-switch.db").write_bytes(b"SQLite format 3\0" + bytes(range(256)) * 8)
    (folder / "backups" / "note.txt").write_text("K-03 untouched\n")
    return folder


def test_whole_client_flow_and_secret_scan(disposable_user):
    api, admin, username, user_id, access = disposable_user
    api.success("POST", "/api/user/manage", {
        "id": user_id, "action": "add_quota", "mode": "override", "value": 1000000,
    }, token=admin)
    require(PROBE.is_file(), "K-01 Rust probe is not built")

    scratch = Path(tempfile.mkdtemp(prefix="aicli-k01-"))
    home = scratch / "home"
    workdir = scratch / "work"
    home.mkdir()
    workdir.mkdir()
    cc_switch = plant_cc_switch(home)
    before = tree_digest(cc_switch)

    environment = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": str(home),
        "XDG_CONFIG_HOME": str(home / ".config"),
        "XDG_DATA_HOME": str(home / ".local/share"),
        "XDG_CACHE_HOME": str(home / ".cache"),
        "XDG_STATE_HOME": str(home / ".local/state"),
        # No keyring on the runner: the file fallback is the expected store.
        "DBUS_SESSION_BUS_ADDRESS": "unix:path=/nonexistent/aicli-k01",
        "AICLI_TEST_LINE_ORIGIN": BASE,
        "AICLI_TEST_ACCESS_TOKEN": access,
        "RUST_LOG": "info",
    }
    result = subprocess.run([str(PROBE), str(workdir)], env=environment,
                            capture_output=True, text=True, timeout=300)
    canaries_path = workdir / "canaries.json"
    secrets = list(api.secrets)
    if canaries_path.is_file():
        canaries = json.loads(canaries_path.read_text())
        secrets += [canaries["access_token"], canaries["privacy_secret"], canaries["local_key"]]
        secrets += list(canaries.get("managed_keys") or [])
    output = result.stdout + result.stderr
    leaked = [value for value in secrets if value and value in output]
    # The public job log gets the probe's output with every known secret cut out.
    shown = output
    for value in sorted({v for v in secrets if v}, key=len, reverse=True):
        shown = shown.replace(value, "<redacted>")
    for line in shown.splitlines()[-80:]:
        print("probe| " + line, flush=True)
    require(not leaked, "K-02: a secret reached the probe's output")
    require(result.returncode == 0, "K-01 end-to-end probe failed")
    steps = json.loads((workdir / "e2e.json").read_text())["steps"]
    names = [step["step"] for step in steps if step["result"] == "pass"]
    require(names == ["bind", "catalog", "profile", "takeover", "token", "request", "bill", "unbind"],
            f"K-01 steps: {names}")
    print("K-01: PASS bind -> catalog -> profile -> token -> request -> bill -> unbind", flush=True)

    scan = subprocess.run(["bash", str(SCAN), str(home), str(canaries_path)],
                          capture_output=True, text=True, timeout=120)
    for line in (scan.stdout + scan.stderr).splitlines():
        require(all(value not in line for value in secrets), "K-02 scan printed a secret")
        print(line, flush=True)
    require(scan.returncode == 0, "K-02 secret scan failed")

    after = tree_digest(cc_switch)
    require(before == after, "K-03: ~/.cc-switch changed during the run")
    print(f"K-03: PASS ~/.cc-switch digest unchanged ({before[:12]}…)", flush=True)
    shutil.rmtree(scratch, ignore_errors=True)
