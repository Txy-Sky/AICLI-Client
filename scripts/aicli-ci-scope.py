#!/usr/bin/env python3
"""Select A6 coverage conservatively; never use an unverified push as baseline."""

import argparse
import json
import re
import subprocess


INFRASTRUCTURE = {
    ".github/workflows/aicli-acceptance.yml",
    "scripts/aicli-ci-scope.py",
}

# Client code exercised by the probes in the A6 environment (C1-04/06/07).
PROBED_CLIENT = ("src-tauri/src/aicli/", "src-tauri/examples/aicli-")
CLIENT_BUILD = {"src-tauri/Cargo.toml", "src-tauri/Cargo.lock"}

# Inputs of the 601-second C1-06 observation; anything else skips it.
OBSERVATION = (
    "src-tauri/src/aicli/cache",
    "src-tauri/src/aicli/newapi",
    "src-tauri/examples/aicli-cache-probe",
)


def _infrastructure(paths):
    return any(
        path.startswith(("tests/aicli-testenv/", ".github/actions/aicli-setup/"))
        or path in INFRASTRUCTURE
        or path.startswith("scripts/tests/test_aicli_ci_scope")
        for path in paths
    )


def required_coverage(paths, subjects, baseline_known=True):
    if not baseline_known:
        return True, "no successful ancestor baseline; run full A6 coverage"
    if any(re.search(r"\b[A-Z][A-Z0-9]*-GATE\b", subject) for subject in subjects):
        return True, "GATE item; run full A6 coverage"
    if _infrastructure(paths):
        return True, "test environment or its CI infrastructure changed"
    if any(path.startswith(PROBED_CLIENT) or path in CLIENT_BUILD for path in paths):
        return True, "client code exercised by A6 probes changed"
    return False, "test environment and probed client code unchanged since last successful acceptance; no GATE"


def required_observation(paths, subjects, baseline_known=True):
    """The C1-06 601-second observation is a subset of A6 coverage."""
    if not baseline_known:
        return True, "no successful ancestor baseline; run C1-06 observation"
    if any(re.search(r"\b[A-Z][A-Z0-9]*-GATE\b", subject) for subject in subjects):
        return True, "GATE item; run C1-06 observation"
    if _infrastructure(paths):
        return True, "test environment or its CI infrastructure changed"
    if any(path.startswith(OBSERVATION) or path in CLIENT_BUILD for path in paths):
        return True, "cache refresh inputs changed"
    return False, "cache refresh inputs unchanged; C1-06 observation not rerun"


def git(*args):
    return subprocess.check_output(["git", *args], text=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", default="")
    parser.add_argument("--head", required=True)
    parser.add_argument("--github-output", required=True)
    args = parser.parse_args()
    baseline_known = bool(args.base) and subprocess.run(
        ["git", "merge-base", "--is-ancestor", args.base, args.head],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    ).returncode == 0
    paths, subjects = [], []
    if baseline_known:
        try:
            paths = git("diff", "--name-only", "-z", args.base, args.head).split("\0")
            subjects = git("log", "--format=%s", f"{args.base}..{args.head}").splitlines()
            # Rechecking a successful GATE SHA must still exercise its environment.
            subjects.append(git("show", "-s", "--format=%s", args.head))
        except subprocess.CalledProcessError:
            baseline_known = False
    required, reason = required_coverage(paths, subjects, baseline_known)
    observe, observe_reason = required_observation(paths, subjects, baseline_known)
    with open(args.github_output, "a") as output:
        output.write(f"testenv={str(required).lower()}\n")
        output.write(f"c106_observe={str(observe).lower()}\n")
    print(json.dumps({"base": args.base, "head": args.head,
                      "testenv": required, "reason": reason,
                      "c106_observe": observe, "c106_reason": observe_reason},
                     sort_keys=True))


if __name__ == "__main__":
    main()
