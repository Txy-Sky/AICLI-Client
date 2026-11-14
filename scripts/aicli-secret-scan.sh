#!/usr/bin/env bash
# K-02: look for the canary values an end-to-end run planted, under the
# throwaway HOME it ran in. Values are never printed, only where they occur.
#
#   scripts/aicli-secret-scan.sh <home> <canaries.json>
#
# canaries.json (written by the aicli-e2e-probe, outside HOME):
#   access_token   the system access token used to bind
#   managed_keys   the sk- keys of the aicli: tokens the run created
#   local_key      the client's local proxy key (reported, not restricted)
#   privacy_secret a fake sk- key sent in a prompt with privacy on
#
# Pass: the access token and managed keys occur only in
# ~/.aicli-client/secrets.json (the file fallback; CI has no keyring), and the
# privacy secret occurs nowhere. SQLite and other binary files are read as
# bytes (what `strings` would show), so a value inside the database counts.
set -euo pipefail

home=${1:?throwaway HOME the run used}
canaries=${2:?canaries.json written by the probe}

case "$(cd -- "$home" && pwd -P)" in
  "$(cd -- ~ && pwd -P)") echo 'K-02 refuses to scan the real HOME' >&2; exit 2 ;;
esac

python3 - "$home" "$canaries" <<'PY'
import json
import os
import sys

home, canaries_path = sys.argv[1], sys.argv[2]
with open(canaries_path, encoding="utf-8") as handle:
    canaries = json.load(handle)

values = {
    "access_token": [canaries["access_token"]],
    "managed_keys": list(canaries.get("managed_keys") or []),
    "local_key": [canaries["local_key"]],
    "privacy_secret": [canaries["privacy_secret"]],
}
for name, items in values.items():
    if not items or any(not isinstance(v, str) or len(v) < 12 for v in items):
        print(f"K-02: FAIL canary {name} is missing or too short to scan for")
        sys.exit(1)
    # A managed key may be stored with or without its sk- prefix.
    if name == "managed_keys":
        values[name] = items + [v[3:] for v in items if v.startswith("sk-")]

fallback = os.path.join(home, ".aicli-client", "secrets.json")
hits = {name: {} for name in values}
scanned = 0
for root, dirs, files in os.walk(home):
    dirs[:] = [d for d in dirs if not os.path.islink(os.path.join(root, d))]
    for file in files:
        path = os.path.join(root, file)
        if os.path.islink(path) or not os.path.isfile(path):
            continue
        try:
            with open(path, "rb") as handle:
                data = handle.read()
        except OSError as error:
            print(f"K-02: FAIL cannot read {os.path.relpath(path, home)}: {error.strerror}")
            sys.exit(1)
        scanned += 1
        for name, items in values.items():
            count = sum(data.count(v.encode()) for v in items)
            if count:
                hits[name][os.path.relpath(path, home)] = count

def outside_fallback(found):
    return {p: n for p, n in found.items() if os.path.join(home, p) != fallback}

problems = []
for name in ("access_token", "managed_keys"):
    stray = outside_fallback(hits[name])
    if stray:
        problems.append(f"{name} outside secrets.json: {sorted(stray)}")
if hits["privacy_secret"]:
    problems.append(f"privacy_secret stored: {sorted(hits['privacy_secret'])}")

print(f"K-02: scanned {scanned} files under the run's HOME (binary files as bytes)")
for name in values:
    where = ", ".join(f"{p}×{n}" for p, n in sorted(hits[name].items())) or "nowhere"
    print(f"K-02: {name}: {where}")
if problems:
    for problem in problems:
        print("K-02: FAIL " + problem)
    sys.exit(1)
print("K-02: PASS server credentials only in secrets.json; privacy canary stored nowhere")
PY
