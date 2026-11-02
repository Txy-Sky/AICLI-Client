"""Offline target guard regression: no API calls and no credential values."""

import os
from pathlib import Path
import subprocess
import sys
import unittest


class LocalOnlyTest(unittest.TestCase):
    def test_nonlocal_targets_exit_before_network_or_pytest(self):
        script = Path(__file__).with_name("test_access_token.py")
        for target in ("https://aicliapi.com", "http://192.0.2.1:13000", "http://127.0.0.1.evil.test", "http://localhost@evil.test", "http://[::1]:13000", "http://127.0.0.1:13000/path", "http://localhost:bad"):
            with self.subTest(target=target):
                result = subprocess.run([sys.executable, "-S", str(script)],
                                        env={**os.environ, "AICLI_TESTENV_BASE_URL": target},
                                        capture_output=True, text=True, timeout=5)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("only accepts 127.0.0.1 or localhost", result.stderr)
                self.assertEqual(result.stdout, "")


if __name__ == "__main__":
    unittest.main()
