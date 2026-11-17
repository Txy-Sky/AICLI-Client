# -*- coding: utf-8 -*-
"""E-07 补充语料生成器：覆盖 Maskit 宣称而 E-03 语料未覆盖的类别。

假密钥/假证件全部运行时拼接生成，不手写；生成后自检无 {{ / TAG_。
不得从 Maskit 的测试或文档复制样例（交接说明第 4 条）。

用法：python scripts/aicli-poc/gen_corpus_e07.py
输出：scripts/aicli-poc/corpus_e07.jsonl
"""

import io
import json
import os

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "corpus_e07.jsonl")


def db_conn(module, user, pw, host, db):
    if module == "postgres":
        return f"postgres://{user}:{pw}@{host}:5432/{db}"
    if module == "mongodb":
        return f"mongodb+srv://{user}:{pw}@{host}.mongodb.net/{db}?retryWrites=true"
    if module == "mysql":
        return f"mysql://{user}:{pw}@{host}:3306/{db}"
    if module == "redis":
        return f"redis://{user}:{pw}@{host}:6379/0"
    # JDBC
    return f"jdbc:mysql://{host}:3306/{db}?user={user}&password={pw}"


def main():
    lines = []

    def add(text):
        lines.append({"text": text})

    # 数据库连接串（含密码）
    for module, user, pw, host, db in [
        ("postgres", "app_user", "Pg9x2Km4", "db1.internal", "orders"),
        ("mongodb", "svc_reader", "Mn7p!q3Z", "cluster0", "analytics"),
        ("mysql", "root", "My5ql#8", "db2.lan", "shop"),
        ("redis", "default", "Rd6s2Fj9", "cache.internal", ""),
        ("jdbc", "report", "Jd3c8Kw2", "replica.internal", "metrics"),
    ]:
        add(f"数据库连不上，备用连接串：{db_conn(module, user, pw, host, db)}")

    # 座机（含区号、分机）
    add("前台座机 010-66889570，转分机 8021")
    add("售后热线：021-52384761 转 3104")
    add("办公室电话 0755-83981234-678")

    # PEM 私钥块
    key_body = "MIIEowIBAAKCAQEA1X9mQv4LtR7pW2nS8uYh3GkD5JfB0aM6cZxE4NvH2qL9sTbU8wOiP5rC7dE3fGhJ6kV1yX0zAqB4nM8sW2tE9rC5uH7jL3pO6iK1mN4bV8cX0zSqW3eR7tY2uF9dA5gHjK4lM"
    add("下面是服务证书的私钥，帮我看看配置对不对：-----BEGIN RSA PRIVATE KEY-----\n"
        + key_body + "\n-----END RSA PRIVATE KEY-----")
    add("-----BEGIN EC PRIVATE KEY-----\nMHcCAQEEII" + key_body[:48] + "\n-----END EC PRIVATE KEY-----")

    # 云 AccessKey
    add(f"阿里云 AK：LTAI{'4tGh9Kq2Wx'}{'7ZnD3Pm'}，SK：{'aK9wQ2mLpX4nR7tY'}{'uB3cE6fHs1Jd'}")
    add(f"AWS 凭证 aws_access_key_id = AKIA{'J6QM3SLZ8'}{'PVK2RT4N'}, aws_secret_access_key = {'wJalrXUtnFEMI'}{'K7MDENGbPxRfiCY'}EXAMPLEKEY")
    add(f"华为云 AK: {'HWPUB8Zk3Lm'}{'9Xq2Rt'}, SK: {'Fg5Yh7Jm3Qp'}{'6Ws8Ed2Vc0Xz9Bn4Mr'}")

    # 各类 IP
    add("办公网出口是 203.0.198.5，内网跳板 10.20.30.40，网关 192.168.8.1")
    add("对方接口白名单要加 172.31.240.9 和 198.51.100.23")
    add("IPv6 调试地址 2408:8207:1234:5678:9abc:def0:1234:5678")

    # JWT
    add(f"抓到的登录态：eyJ{'hbGciOiJIUzI1NiJ9'}.eyJ{'zdWIiOiIxMjM0NTY3ODkwIn0'}.Sfl{'KxwRJSMeKKF2QT4fwpMeJf36POk6yJV'}")

    # .env 片段
    add("\n".join([
        "# .env 生产配置（勿外传）",
        "DATABASE_PASSWORD=" + "Pr0d#Xk7" + "Qm2L",
        "STRIPE_SECRET_KEY=sk_live_" + "9Za8Qw3Er5Ty1Ui6",
        "JWT_SIGNING_SECRET=" + "j8sN2k4R6t" + "Vb7cXz9",
        "SMTP_PASS=" + "Sm5tp!8Kq3",
    ]))

    # 代码里的硬编码凭证
    add(f"const client = new Client({{ token: 'ghp_' + 'Xk7Qm2Lp9Rt4Vb' + 'N8cZ3wY6Jd1Hf' }});")
    add(f"os.environ['OPENAI_API_KEY'] = 'sk-proj-' + 'Wq8Rx5Ty2Uz'")
    add(f"password = '{'Py9!ssw0rd'}'  # TODO 上线前改掉")
    add(f"curl -H 'Authorization: Bearer {'xoxb-' + '6626-8813' + '-Tq2Wv9Xz'}' https://api.example.com")

    # 误报对照（不应遮）
    add("订单号 202605180009876543 已经发货")
    add("提交 hash 3f5a8c90e2b1d47f6a09c88e77b5d1aa 是本次回滚的基线")
    add("用户 ID 10086 和批次 8640021 对不上")
    add("版本号 2.11.3-rc.4 修复了这个 bug")
    add("路径 /usr/local/share/dotnet/sdk/8.0.401 存在")

    with io.open(OUT, "w", encoding="utf-8", newline="\n") as f:
        for line in lines:
            f.write(json.dumps(line, ensure_ascii=False) + "\n")

    content = io.open(OUT, encoding="utf-8").read()
    assert "{{" not in content, "placeholder leaked"
    assert "TAG_" not in content, "gateway tag leaked"
    print(f"wrote {len(lines)} lines to {OUT}")


if __name__ == "__main__":
    main()
