"""Offline A5-01 checks; run in the isolated GitHub Actions job."""

import json
from pathlib import Path
import tempfile
import unittest

from line_probe import (
    Api, LINES, NoRedirect, ProbeBlocked, StreamMetrics, budget_plan,
    choose_token, decide, direct_opener, save_report, sustained,
)


class BudgetTests(unittest.TestCase):
    def setUp(self):
        self.token = {"id": 1, "group": "g", "unlimited_quota": True}
        self.model = {"quota_type": 0, "model_ratio": 1, "completion_ratio": 6,
                      "enable_groups": ["g"]}
        self.groups = {"g": {"ratio": 0.9}}

    def plan(self, **changes):
        values = dict(quota=2000000, token=self.token, model=self.model,
                      groups=self.groups, prompt="One text message", max_output_tokens=100)
        values.update(changes)
        return budget_plan(**values)

    def test_formula_and_five_percent(self):
        plan = self.plan()
        self.assertEqual(plan["upper_quota"], 65475)
        self.assertEqual(plan["five_percent_quota"], 100000)
        self.assertLessEqual(plan["upper_quota"], plan["five_percent_quota"])

    def test_over_budget_is_blocked(self):
        with self.assertRaisesRegex(ProbeBlocked, "request_exceeds_five_percent"):
            self.plan(quota=10000)

    def test_tiered_and_unknown_prices_are_blocked(self):
        for change in ({"billing_mode": "tiered_expr"}, {"model_ratio": None},
                       {"completion_ratio": "NaN"}, {"model_ratio": -1},
                       {"quota_type": 1}, {"billing_expr": "unknown"}):
            with self.subTest(change=change), self.assertRaises(ProbeBlocked):
                self.plan(model={**self.model, **change})

    def test_unknown_or_auto_group_is_blocked(self):
        for token in ({**self.token, "group": "auto"}, {**self.token, "group": "unknown"}):
            with self.subTest(token=token), self.assertRaises(ProbeBlocked):
                self.plan(token=token)
        with self.assertRaises(ProbeBlocked):
            self.plan(groups={"g": {}})

    def test_limited_token_quota_is_respected(self):
        with self.assertRaises(ProbeBlocked):
            self.plan(token={**self.token, "unlimited_quota": False, "remain_quota": 10})

    def test_output_limit_and_prompt_bounds(self):
        for limit in (0, -1, 65537):
            with self.subTest(limit=limit), self.assertRaises(ProbeBlocked):
                self.plan(max_output_tokens=limit)
        with self.assertRaises(ProbeBlocked):
            self.plan(prompt="a" * 4097)

    def test_group_must_allow_model(self):
        with self.assertRaises(ProbeBlocked):
            self.plan(model={**self.model, "enable_groups": ["different"]})


class SafetyTests(unittest.TestCase):
    def test_prohibited_operations_are_not_constructed(self):
        api = Api("fake-access-value")
        for path in ("/api/user/token", "/api/user/self/delete", "/api/token/1"):
            with self.subTest(path=path), self.assertRaises(ProbeBlocked):
                api.request("main", path)
        with self.assertRaises(ProbeBlocked):
            api.request("main", "/api/token/", payload={"name": "not-created"})
        with self.assertRaises(ProbeBlocked):
            api.request("main", "//unapproved.invalid/api/status")
        with self.assertRaises(ProbeBlocked):
            api.request("unapproved", "/api/status")

    def test_existing_key_read_is_post_and_no_user_header(self):
        request = Api("fake-access-value").request("main", "/api/token/batch/keys", payload={"ids": [1]})
        self.assertEqual(request.get_method(), "POST")
        self.assertEqual(json.loads(request.data), {"ids": [1]})
        self.assertNotIn("New-api-user", request.headers)
        self.assertEqual(request.headers["Authorization"], "Bearer fake-access-value")

    def test_redirects_refused(self):
        self.assertIsNone(NoRedirect().redirect_request(None, None, 302, "", {}, "https://other.invalid"))

    def test_direct_opener_has_no_proxies(self):
        opener = direct_opener()
        proxy_handlers = [handler for handler in opener.handlers if hasattr(handler, "proxies")]
        self.assertTrue(all(not handler.proxies for handler in proxy_handlers))

    def test_report_never_contains_credentials(self):
        api = Api("fake-access-value")
        api.model_key = "fake-model-value"
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "report.json"
            for value in (api.access_token, api.model_key):
                with self.assertRaisesRegex(ProbeBlocked, "credential_in_report"):
                    save_report(path, {"error": value}, api)
                self.assertFalse(path.exists())
            save_report(path, {"safe": True}, api)
            self.assertEqual(json.loads(path.read_text()), {"safe": True})
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)

    def test_token_pagination_and_eligibility(self):
        class FakeApi:
            def __init__(self):
                self.paths = []

            def fetch(self, path):
                self.paths.append(path)
                token = {"id": 1, "status": 1, "expired_time": -1, "group": "g"}
                if "p=1&" in path:
                    token["model_limits_enabled"] = True
                return {"data": {"items": [token], "total": 2}}

        api = FakeApi()
        token = choose_token(api, {"enable_groups": ["g"]})
        self.assertEqual(token["id"], 1)
        self.assertEqual(len(api.paths), 2)
        self.assertTrue(all(path.startswith("/api/token/?") for path in api.paths))


class MetricsTests(unittest.TestCase):
    def feed(self, metrics, event, at):
        raw = b"data: " + json.dumps(event, ensure_ascii=False).encode() + b"\r\n\r\n"
        for value in raw:
            metrics.feed(bytes([value]), at)

    def test_utf8_and_byte_gap_without_body_logging(self):
        metrics = StreamMetrics(0)
        self.feed(metrics, {"type": "response.output_text.delta", "delta": "中文"}, 2)
        self.feed(metrics, {"type": "response.output_text.delta", "delta": "another"}, 7)
        summary = metrics.summary(8)
        self.assertEqual(summary["max_adjacent_byte_gap_s"], 5)
        self.assertEqual(summary["text_span_s"], 5)
        self.assertEqual(summary["text_chars"], 9)
        self.assertNotIn("中文", json.dumps(summary, ensure_ascii=False))

    def test_heartbeat_is_not_text_output(self):
        metrics = StreamMetrics(0)
        for at in range(0, 200, 10):
            for value in b": ping\n\n":
                metrics.feed(bytes([value]), at)
        summary = metrics.summary(200)
        self.assertEqual(summary["comment_lines"], 20)
        self.assertEqual(summary["text_events"], 0)
        self.assertFalse(sustained({**summary, "http_status": 200, "tls": "verified"}))

    def test_error_and_terminal_events(self):
        metrics = StreamMetrics(0)
        self.feed(metrics, {"type": "error", "error": {"message": "fake-sensitive-value"}}, 1)
        self.assertTrue(metrics.api_error)
        self.assertNotIn("fake-sensitive-value", json.dumps(metrics.summary(2)))
        self.feed(metrics, {"type": "response.completed", "response": {"usage": {"output_tokens": 3}}}, 3)
        self.assertTrue(metrics.done)
        self.assertEqual(metrics.usage["output_tokens"], 3)

    def test_overlong_or_invalid_event_is_blocked(self):
        with self.assertRaises(ProbeBlocked):
            StreamMetrics(0).event(b"not-json", 1)
        metrics = StreamMetrics(0)
        metrics.line = bytearray(b"x" * 1048576)
        with self.assertRaises(ProbeBlocked):
            metrics.feed(b"x", 1)


class DecisionTests(unittest.TestCase):
    def rows(self):
        result = []
        for line in LINES:
            result.append({"line": line, "scenario": "idle", "http_status": 200,
                           "data_events": 2, "terminal_event": True})
            result.append({"line": line, "scenario": "continuous", "http_status": 200,
                           "tls": "verified", "text_span_s": 186, "duration_s": 190,
                           "max_text_gap_s": 2, "max_adjacent_byte_gap_s": 2,
                           "text_events": 100, "stop": "client_observed_over_180s"})
        return result

    def test_all_three_sustained(self):
        self.assertEqual(decide(self.rows()), "idle_limit_use_600s")

    def test_cf_disconnect_with_apac_control(self):
        rows = self.rows()
        for row in rows:
            if row["scenario"] == "continuous" and row["line"] != "apac":
                row.update(stop="transport_error", text_span_s=115, duration_s=120)
        self.assertEqual(decide(rows), "total_limit_use_110s_prefer_apac")

    def test_short_completion_and_mixed_results_are_inconclusive(self):
        rows = self.rows()
        rows[1].update(stop="eof", terminal_event=True, text_span_s=80, duration_s=120)
        self.assertEqual(decide(rows), "inconclusive")
        self.assertEqual(decide(rows[:-1]), "inconclusive")

    def test_api_error_or_idle_failure_never_passes(self):
        rows = self.rows()
        rows[0]["api_error"] = True
        self.assertEqual(decide(rows), "inconclusive")
        rows = self.rows()
        rows[1]["api_error"] = True
        self.assertEqual(decide(rows), "inconclusive")


if __name__ == "__main__":
    unittest.main()
