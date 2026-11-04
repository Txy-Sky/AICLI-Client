import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    "aicli_ci_scope", Path(__file__).resolve().parents[1] / "aicli-ci-scope.py"
)
scope = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scope)


class CoverageTests(unittest.TestCase):
    def test_ordinary_client_change_skips_a6(self):
        self.assertFalse(scope.required_coverage(["src/App.tsx"], ["feat(aicli): B-07 copy"])[0])

    def test_test_environment_change_requires_a6(self):
        for path in ("tests/aicli-testenv/docker-compose.yml", "tests/aicli-testenv/seed.py",
                     "tests/aicli-testenv/tests/test_access_token.py"):
            with self.subTest(path=path):
                self.assertTrue(scope.required_coverage([path], ["fix(aicli): A6-01 fixture"])[0])

    def test_gate_requires_a6_without_testenv_changes(self):
        for item in ("A-GATE", "B-GATE", "C5-GATE"):
            with self.subTest(item=item):
                self.assertTrue(scope.required_coverage(["scripts/gate.sh"], [f"test(aicli): {item} verify"])[0])

    def test_gate_in_earlier_commit_requires_a6(self):
        self.assertTrue(scope.required_coverage(["src/App.tsx"],
                        ["test(aicli): B-GATE verify", "fix(aicli): B-GATE formatting"])[0])

    def test_cancelled_intermediate_environment_change_is_not_lost(self):
        # Input is the entire diff from last SUCCESS, including a superseded push.
        self.assertTrue(scope.required_coverage(["tests/aicli-testenv/seed.py", "src/App.tsx"],
                        ["fix(aicli): A6-01 seed", "feat(aicli): B-07 copy"])[0])

    def test_ci_infrastructure_change_requires_a6(self):
        for path in (".github/workflows/aicli-acceptance.yml",
                     ".github/actions/aicli-setup/action.yml", "scripts/aicli-ci-scope.py"):
            with self.subTest(path=path):
                self.assertTrue(scope.required_coverage([path], ["ci(aicli): CI-OPT cache"])[0])

    def test_unknown_or_nonancestor_baseline_requires_a6(self):
        self.assertTrue(scope.required_coverage([], [], baseline_known=False)[0])

    def test_non_gate_text_does_not_force_a6(self):
        self.assertFalse(scope.required_coverage(["src/App.tsx"], ["fix(aicli): B-07 investigate"])[0])


class ProbedClientTests(unittest.TestCase):
    def test_probed_client_code_requires_a6(self):
        for path in ("src-tauri/src/aicli/profiles.rs", "src-tauri/src/aicli/newapi.rs",
                     "src-tauri/examples/aicli-catalog-probe.rs", "src-tauri/Cargo.lock"):
            with self.subTest(path=path):
                self.assertTrue(scope.required_coverage([path], ["feat(aicli): C1-09 tokens"])[0])

    def test_item_test_list_change_alone_skips_a6(self):
        self.assertFalse(scope.required_coverage(["scripts/aicli-item-tests.sh"],
                         ["test(aicli): C1-09 add item tests"])[0])

    def test_unrelated_rust_change_skips_a6(self):
        self.assertFalse(scope.required_coverage(["src-tauri/src/proxy/server.rs"],
                         ["feat(aicli): C3-04 switch"])[0])


class ObservationTests(unittest.TestCase):
    def test_other_aicli_module_skips_observation(self):
        paths = ["src-tauri/src/aicli/profiles.rs", "scripts/aicli-item-tests.sh"]
        subjects = ["feat(aicli): C1-08 profiles"]
        self.assertTrue(scope.required_coverage(paths, subjects)[0])
        self.assertFalse(scope.required_observation(paths, subjects)[0])

    def test_cache_inputs_require_observation(self):
        for path in ("src-tauri/src/aicli/cache.rs", "src-tauri/src/aicli/newapi.rs",
                     "src-tauri/examples/aicli-cache-probe.rs", "src-tauri/Cargo.toml",
                     "tests/aicli-testenv/tests/test_cache_refresh.py"):
            with self.subTest(path=path):
                self.assertTrue(scope.required_observation([path], ["fix(aicli): C1-06 cache"])[0])

    def test_gate_infrastructure_and_unknown_baseline_require_observation(self):
        self.assertTrue(scope.required_observation(["src/App.tsx"], ["test(aicli): C5-GATE run"])[0])
        self.assertTrue(scope.required_observation([".github/workflows/aicli-acceptance.yml"],
                        ["ci(aicli): CI-OPT2 split"])[0])
        self.assertTrue(scope.required_observation([], [], baseline_known=False)[0])

    def test_observation_implies_a6(self):
        cases = [["src-tauri/src/aicli/cache.rs"], ["src-tauri/Cargo.lock"],
                 ["tests/aicli-testenv/seed.py"], ["src/App.tsx"], ["src-tauri/src/aicli/x.rs"]]
        for paths in cases:
            with self.subTest(paths=paths):
                if scope.required_observation(paths, ["feat(aicli): C2-01"])[0]:
                    self.assertTrue(scope.required_coverage(paths, ["feat(aicli): C2-01"])[0])
