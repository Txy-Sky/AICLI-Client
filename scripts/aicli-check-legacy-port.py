#!/usr/bin/env python3
"""B-02/C3-10: exactly two detection-only legacy literals, nowhere else."""
from pathlib import Path

allowed_file = Path("src-tauri/src/aicli/takeover.rs")
expected = [
    (str(allowed_file), "const LEGACY_CC_SWITCH_PORT: u16 = 15721;"),
    (str(allowed_file), 'const LEGACY_CC_SWITCH_DESKTOP_PROFILE_ID: &str = "00000000-0000-4000-8000-000000157210";'),
]
actual = []
for root in (Path("src-tauri/src"), Path("src")):
    for path in sorted(root.rglob("*")):
        if path.is_file():
            for line in path.read_bytes().splitlines():
                if b"15721" in line:
                    actual.append((str(path), line.decode("utf-8", errors="replace").strip()))
if actual != expected:
    raise SystemExit(f"B-02/C3-10: unexpected legacy references: {actual!r}")
print("B-02/C3-10: exactly the two authorized detection constants; PASS")
