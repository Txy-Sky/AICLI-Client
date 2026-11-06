# C5-GATE 真实客户端验收记录（2026-11-06）

环境（H-01/H-02 已提供）：
- 主机：负责人本机（Windows）；H-01 令牌经 Actions secret 与环境变量提供，值不落任何记录。
- 客户端：Claude Code 2.1.293、Codex CLI 0.160.0、Gemini CLI 0.38.2（node v24.11.0）。
- 载体：`src-tauri/examples/aicli-c5-runner`（无头 runner：Database::init + seed::init +
  AicliServices::bind（含 C4-02 自动接管）→ 驻留 → stop 文件触发 unbind(true) 还原+删 Token）。
  与应用启动对齐的两处装配（seed::init、rustls CryptoProvider）由 runner 自带，产品无此问题。
- 线路：main（aicliapi.com）。测试前后对 ~/.claude/settings.json、~/.codex/config.toml、
  ~/.codex/cc-switch-model-catalog.json、~/.gemini/.env、~/.gemini/settings.json、
  ~/.grok/config.toml 做了带 SHA256 的全量备份；结束时 sha256sum -c 全部 OK（逐字节还原）。
- 日志：全部客户端原始输出存负责人本机 `~/c5-logs/`；proxy_request_logs 共 73 行。

## 结果矩阵

| 组合 | 别名可见 | 文本(流/非流) | 工具(并行/回传) | 多轮 | 图片 | 长上下文 | 错误格式 | 取消 |
|---|---|---|---|---|---|---|---|---|
| Claude Code → gpt-6.1-sol（服务端转换 wire=anthropic，默认） | ✅ `aicli-claude-gpt-6.1-sol`（/v1/models 带 anthropic-version 65 项别名） | ✅ SMOKE-OK-1；API 直连 stream 32 行 SSE / 非 stream end_turn | ✅ 两命令并行 echo AAA-111/BBB-222 均执行并回报 | ✅ PINEAPPLE-77 跨会话续接 | ✅ red_box.png → Red | ✅ 1.1MB/2 万行定位 UNICORN-42 第 13778 行 | ✅ 400 Model is not in the selected profile，客户端干净报错 | ✅ SIGINT 正常退出，服务端行正常落库 |
| Claude Code → claude-sonnet-5-5（原生 anthropic） | ✅ | ✅ NATIVE-OK | ✅ NATIVE-TOOL-99 | ✅ MANGO-33 | ✅ 红色 | ✅ 13778 行 | ✅ 同上 | ✅（同机制） |
| Codex → gpt-6.1-sol（原生 openai-response） | ✅（/v1/models 裸名） | ✅ CODEX-OK | ✅ CODEX-TOOL-77（shell 工具） | ✅ resume --last ORCHID-55 | ✅ Red | ✅ rg 定位 13778 行 | ✅ invalid_request_error/model_not_in_profile 干净显示 | ✅（进程可中断，行落库） |
| Gemini CLI → gemini-3.1-pro / 3-pro-preview / 3.6-flash | — | ❌ 上游 503「分组 Gemini-Ultra 下模型 … 无可用渠道(distributor)」 | — | — | — | — | — | — |

Gemini 组合：代理侧行为正确（别名解析、转发、错误干净回显给 CLI），但直接用编译出的
auto 组 sk- 令牌以 openai 与 gemini 两种 wire 直连线路复测，上游均返回同一 distributor
错误——目录里的 gemini 模型在服务端当前无可用渠道，属服务端可用性问题，客户端侧无法
继续验证。按清单规则，Tier 1 原生协议组合 Gemini→gemini 必须 full，C5-GATE 标 `[!]`，
已在第 12 节登记，待负责人开通渠道后仅补测该组合。

额度不足场景未注入（账户 quota 充足）；错误格式以「模型不存在」为代表验证。

## 转换方式比较（Claude Code → gpt-6.1-sol）

同一用例集分别跑默认 wire=anthropic（服务端转换）与 compat.json 覆盖
`anthropic|gpt-6.1-sol: {"wire":"openai-response"}`（客户端转换，重新编译 runner 后验证）：

| 用例 | 服务端转换(anthropic) | 客户端转换(openai-response) |
|---|---|---|
| 文本 | ✅ CONV 对拍通过 | ✅ CONV-CLIENT-OK |
| 工具 | ✅ | ✅ WIRE2-TOOL-11 |
| 多轮 | ✅ PINEAPPLE-77 | ✅ BANANA-88 |
| 图片 | ✅ Red | ✅ 红色 |
| 长上下文 | ✅ 13778 | ✅ 13,778 |
| 错误 | ✅ 400 干净 | ✅ 400 干净 |

通过项持平（6:6）→ 按定案规则保持默认（服务端转换），compat.json 不写 wire 覆盖。

## compat.json 定案（本轮已验证项）

- `anthropic|gpt-6.1-sol`：full（双 wire 均全过）
- `anthropic|claude-sonnet-5-5`：full
- `openai-response|gpt-6.1-sol`：full
- `gemini|gemini-*`：无条目（服务端无渠道，未验证，unverified 默认）

## 清理与还原

- 解绑两轮（首轮因 Cloudflare 对本机临时 403 失败一次，退避后成功）：远端 `aicli:` Token
  全部删除（本地表 0）、aicli_account/aicli_takeover/aicli_local_token 清空。
- 六个客户端配置 sha256sum -c 全部 OK；proxy_request_logs 保留 73 行作为请求证据。
- claude -p 运行会回写 ~/.claude/settings.json（env 合并），还原以备份为准。

## 结论

- Claude Code（两个组合）、Codex 全项通过；Gemini 组合因服务端无渠道未验证。
- C5-GATE 状态：`[!]`（仅剩 Gemini→gemini 原生组合，等服务端渠道），其余全部通过。
