import json
from pathlib import Path
import tempfile
import unittest

import account_probe


class AccountProbeTests(unittest.TestCase):
    def test_only_read_paths_are_allowed(self):
        request = account_probe.request("main", "/api/user/self", "fake-access")
        self.assertEqual(request.get_method(), "GET")
        self.assertEqual(request.headers["Authorization"], "Bearer fake-access")
        for path in (
            "/api/token/",
            "/api/token/1",
            "/api/user/token",
            # W-00：钱包的写接口一个都不能探测
            "/api/user/pay",
            "/api/user/amount",
            "/api/user/topup",
            "/api/user/aff_transfer",
            "/api/subscription/epay/pay",
            "/api/subscription/balance/pay",
        ):
            with self.assertRaises(account_probe.ProbeBlocked):
                account_probe.request("main", path, "fake-access")

    def test_group_intersection_does_not_guess(self):
        rows = account_probe.model_group_counts(
            [{"model_name": "a", "enable_groups": ["g", "missing"]}], {"g"}
        )
        self.assertEqual(rows, [{"model": "a", "usable_group_count": 1}])
        self.assertEqual(account_probe.model_group_counts([{"model_name": "b"}], {"g"}), [])

    def test_report_can_be_written_without_credentials(self):
        with tempfile.TemporaryDirectory() as home:
            old = {key: __import__("os").environ.get(key) for key in ("HOME", "CC_SWITCH_TEST_HOME", "AICLI_TEST_ACCESS_TOKEN")}
            __import__("os").environ["HOME"] = home
            __import__("os").environ["CC_SWITCH_TEST_HOME"] = home
            __import__("os").environ["AICLI_TEST_ACCESS_TOKEN"] = "fake-access-value"
            original = account_probe.fetch
            account_probe.fetch = lambda line, path, token: (
                200,
                {"data": {"quota": 10}} if path == "/api/user/self" else
                {"data": {"max_count": 2}} if path.endswith("auto-groups") else
                {"data": {"g": {"ratio": 1}, "auto": {"ratio": 1}}} if path.endswith("groups") else
                {"data": {"items": [], "total": 0}} if path.startswith("/api/log") else
                {"data": [{"model_name": "a", "enable_groups": ["g"]}]},
            )
            try:
                path = Path(home) / "report.json"
                self.assertEqual(account_probe.run(path), 0)
                self.assertEqual(json.loads(path.read_text())["conclusion"], "pass")
            finally:
                account_probe.fetch = original
                for key, value in old.items():
                    if value is None:
                        __import__("os").environ.pop(key, None)
                    else:
                        __import__("os").environ[key] = value


    def test_wallet_capabilities_keep_metadata_only(self):
        replies = {
            "/api/user/topup/info": {"data": {
                "enable_online_topup": True,
                "enable_redemption": True,
                "min_topup": 10,
                "amount_options": [10, 50],
                "pay_methods": [
                    {"name": "A", "type": "alipay"},
                    {"name": "S", "type": "stripe"},
                    "broken",
                ],
            }},
            "/api/user/topup/self": {"data": {"items": [{"trade_no": "USR7NOprivate"}], "total": 1}},
            "/api/subscription/plans": {"data": [{"plan": {"id": 1, "title": "private title"}}]},
            "/api/subscription/self": {"data": {"subscriptions": []}},
        }
        original = account_probe.fetch
        account_probe.fetch = lambda line, path, token: (200, replies[path.split("?")[0]])
        try:
            wallet = account_probe.wallet_capabilities("fake-access")
        finally:
            account_probe.fetch = original
        self.assertEqual(wallet["pay_method_types"], ["alipay", "stripe"])
        self.assertEqual(wallet["epay_method_types"], ["alipay"])
        self.assertEqual((wallet["plan_count"], wallet["amount_option_count"]), (1, 2))
        self.assertTrue(wallet["history_shape_valid"] and wallet["subscriptions_shape_valid"])
        serialized = json.dumps(wallet)
        self.assertNotIn("private", serialized)

        account_probe.fetch = lambda line, path, token: (None, None)
        try:
            unreachable = account_probe.wallet_capabilities("fake-access")
        finally:
            account_probe.fetch = original
        self.assertEqual(unreachable["topup_info_http"], None)
        self.assertEqual(unreachable["plan_count"], None)
        self.assertFalse(unreachable["enable_online_topup"])


if __name__ == "__main__":
    unittest.main()
