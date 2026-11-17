# -*- coding: utf-8 -*-
"""E-07 新增规则的验收用例（含误报对照）生成后自检。

写入 src-tauri/tests/fixtures/privacy/e07_rules_cases.json：
[{"text": ..., "must_mask": [...]}, {"text": ..., "must_not_mask": [...]}]
值运行时拼接；输出自检无 {{。
"""

import io
import json
import os

OUT = os.path.join(
    os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))),
    "src-tauri", "tests", "fixtures", "privacy", "e07_rules_cases.json",
)


def main():
    pw_at = "s3cr" + "@t7"          # 口令含 @
    pw_hash = "Pg9x" + "#2Km4"      # 口令含 #
    ltai = "LTAI" + "4tGh9Kq2Wx7ZnD3P"  # 阿里云 AK（LTAI+16）
    sk = "aK9wQ2mLpX4nR7tY" + "uB3cE6fHs1Jd"  # SK 30 位
    pw_lead = "P" + "@ssw0rd"       # @ 在第二位（最常见写法）
    pw_lead_hash = "x" + "#9Kq2"    # # 在第二位
    pw_redis = "P" + "@ss1234"      # 用户名为空
    word24 = "Xk9aB3mQ7pL2zR8tW4nV6cY1"  # 24 位，前面是普通词尾的 SK

    cases = [
        # 连接串口令含 @/#：必须遮
        {"text": f"备用连接：postgres://app:{pw_at}@db1.internal:5432/orders", "must_mask": [pw_at]},
        {"text": f"mysql://root:{pw_hash}@db2.lan:3306/shop 连不上", "must_mask": [pw_hash]},
        # 阿里云 AK / SK：必须遮
        {"text": f"阿里云 AK：{ltai}，SK：{sk}", "must_mask": [ltai, sk]},
        {"text": f"access_key_secret = {sk}", "must_mask": [sk]},
        # 验收复核补充（2026-11-08）：@/# 在口令第二位、用户名为空、无端口、
        # IP 主机；须整段遮成 PASSWORD
        {"text": f"postgres://admin:{pw_lead}@db.example.com:5432/app", "must_mask": [pw_lead], "kind": "PASSWORD"},
        {"text": f"mysql://root:{pw_lead_hash}@10.0.0.5:3306/db", "must_mask": [pw_lead_hash], "kind": "PASSWORD"},
        {"text": f"redis://:{pw_redis}@cache.internal:6379/0", "must_mask": [pw_redis], "kind": "PASSWORD"},
        {"text": f"DATABASE_URL=postgres://admin:{pw_lead}@db.example.com/app", "must_mask": [pw_lead], "kind": "PASSWORD"},
        {"text": f"AWS_SK={sk}", "must_mask": [sk], "kind": "SECRET"},
        # 误报对照：不遮
        {"text": "LTAI 后面不足 16 位的不算：LTAI4tGh9", "must_not_mask": ["LTAI4tGh9"]},
        {"text": "postgres://app:nopass@host:5432/db 口令无 @/# 走原规则", "must_not_mask": ["无 @"]},
        {"text": "订单号 LTAI202605180009 参与促销", "must_not_mask": ["LTAI202605180009"]},
        {"text": "SK 后太短不遮：SK：aK9wQ2", "must_not_mask": ["aK9wQ2"]},
        # SK 须是独立的名字：TASK/DISK/RISK/MASK 结尾不算
        {"text": f"TASK: {word24}", "must_not_mask": [word24]},
        {"text": f"DISK={word24}", "must_not_mask": [word24]},
        {"text": f"RISK: {word24}", "must_not_mask": [word24]},
        {"text": f"MASK={word24}", "must_not_mask": [word24]},
        # URL 里普通端口/片段不是口令
        {"text": "https://example.com:8443/path?q=1", "must_not_mask": ["example.com:8443"]},
    ]
    with io.open(OUT, "w", encoding="utf-8", newline="\n") as f:
        json.dump(cases, f, ensure_ascii=False, indent=1)
    content = io.open(OUT, encoding="utf-8").read()
    assert "{{" not in content and "TAG_" not in content
    print(f"wrote {len(cases)} cases to {OUT}")


if __name__ == "__main__":
    main()
