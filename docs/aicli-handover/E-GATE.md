# E-GATE 续接交接（新会话从这里开始）

基线：`feat/aicli-client-v1` @ `8bf621ac`（E-01～E-08 全部验收通过，含审查方补修的已知值记忆）。工作区干净。
本文件 + `egate-frontend-failures.log`（Windows 前端单测失败全量日志，2570 行）已入库随此提交。

任务：清单 `### E-GATE 阶段 E 门禁`，三条验收：
1. `scripts/aicli-check.sh` 全部通过；
2. 真实 Claude Code 经本地代理（H-02 = 本机 Windows）走一轮，隐私开启：
   - 消息带假密钥 + 假邮箱（值脚本运行时生成，自检无 `{{` / `TAG_`），上游收占位符、Claude Code 见原值；
   - 下一轮同值不带"密钥/密码"字样单独出现 → 仍被遮（已知值记忆，见 E-03.txt 第三部分）；
   - 流式回复中途 Esc 中断一次 → 不残留占位符、代理不报错；
3. 证据 `docs/aicli-evidence/E-GATE.txt`，值一律打码。

## 当前进度

### 第 1 项：卡在 pnpm test:unit（Windows 本机特有）

- `aicli-check.sh` 步骤 1–3 过（install/typecheck/format:check）；**步骤 4 `pnpm test:unit` 失败**：
  3360 个前端测试 43 个失败（重跑 98 个 → 数量不稳定，含时序抖动）
- **与 E-07/E-08 无关**：在未含我改动的已验收提交 `dca1815a` 上同样失败；CI 的
  B stage desktop checks（Linux）全绿 → Windows 特有，从未被 CI 拦过
- 失败集中在 10 个文件：AppsPage、SettingsDialog（integration）、ProviderForm 多个、
  IconPicker、KimiProviderForm、PiProviderForm、ClaudeDesktopProviderForm、
  SessionManagerPage、SwitchModePanel、UsageDashboard.smoke
- 已见根因样本（详见 egate-frontend-failures.log）：
  1. **路径拼接**：`Expected /home/mock/.claude, Received /home/mock/.claude/picked`
     （SettingsDialog 集成测试；疑 Windows 下 path join/选择器行为差）
  2. **CodeMirror**：`textRange(...).getClientRects is not a function`（jsdom 缺 API；
     ProviderForm.stackModels 等）
  3. **MSW 未拦截**：`get_codex_session_compression` / `get_codex_sessions_disk_usage` /
     `dsh_account_status` / `get_openclaw_env` / `get_claude_default_plan` 等 handler 缺失
  4. **React Query**：`Query data cannot be undefined`（managed-auth-status 等）
  5. 大量 `act(...)` 警告（时序类，可能只是噪声也可能连锁超时）
- **修法建议**：先按根因分组修（1/2/3 大概率各修一处解决一批）；修不了的
  平台差异测试可加 `describe.skipIf(process.platform === 'win32')` 但要在 E-GATE.txt
  写明跳过理由与数量。目标 = `bash scripts/aicli-check.sh` 全绿（后续步骤
  build:renderer / cargo fmt / clippy / nextest 尚未在本轮跑到，预期无碍）
- 注意：`tests/components/__snapshots__/ProviderForm.presetRows.golden.test.tsx.snap`
  会被测试改写（运行后出现 M），提交前 `git checkout --` 还原，别把快照差异带进提交

### 第 2 项：未开始（真实 Claude Code 隐私轮）

准备要点：
- 本机已装 Claude Code（`D:\Local_large_model\` 下还有用户自己的 Maskit 桌面版占着
  5801 端口——别动它；我们自己的服务避让端口）
- 代理：仓库 `proxy/server.rs`（ProxyServer），隐私中间件 E-06 已接（local_auth 之内）。
  需要绑定账号（aicli_account 有行 + privacy_enabled=1）+ mock/真实上游。可参考
  `src-tauri/tests/aicli_privacy_e2e.rs` 的夹具思路，但 E-GATE 要**真实 Claude Code**：
  最省事的形态 = 捕获上游（照抄 `scripts/aicli-poc/maskit_diff.py` 的 CaptureHandler）
  作为 AICLI 线路的上游，Claude Code 的 ANTHROPIC_BASE_URL 指向本地代理
- 假密钥/假邮箱：**运行时拼接生成**（如 `''.join(['sk-','Zx9Q…'])`），写完自检无
  `{{` / `TAG_`（本地网关会把命令里的完整敏感形态替换成占位符——E-03/E-07 都踩过，
  拼接可避让）
- 断言：捕获上游收到的 body 里是 `{{API_KEY_…}}`/`{{EMAIL_…}}` 占位符且无原值；
  Claude Code 界面回显原值（响应路径 restore）；第二轮裸值仍遮（已知值记忆，
  `known.rs`：≥8 字节的值遮一次后进程内任何上下文都遮）；Esc 中断流式后再发一条
  正常消息，确认无占位符残留、代理日志无 error
- 证据打码：日志/截图里的假值也按密钥形态打码后入库

### 第 3 项：E-GATE.txt（未开始）

格式照 E-06.txt：状态、逐条验收记录、失败历史（若有）、CI。注意清单要求 GATE 项
提交标题带项目编号（第 0.2 节 / 第 12 节：**GATE 实现和修复提交的标题必须带相应
`*-GATE` 项目编号**）→ 提交格式如 `feat(aicli): E-GATE …` / `fix(aicli): E-GATE …`。

## 自查清单（每轮提交前，来自 E-07-E-08 交接第三部分第 2 条）

- 故障注入点在真实失败处；夹具不让被测条件自动成立；去掉修复测试必须失败
- 声称"既有问题"要带最小复现（Windows 前端失败这条已带 log + 在 dca1815a 复现）
- 本地先过：`cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` +
  相关测试，再推；每次 CI 约 15–20 分钟
- E-GATE 涉及真实客户端的人工步骤（H-02），截图/抓包打码后入库

## 环境备忘

- Windows + git bash；本地代理 127.0.0.1:7897（曾宕机数小时，push 失败时先查它）
- `gh` CLI 已登录（Txy-Sky），`gh run list --repo Txy-Sky/AICLI-Client --branch feat/aicli-client-v1`
- 前端全量单测一次约 16–20 分钟（jsdom 很慢），跑的时候耐心
- E-07 用过的隔离 Maskit 实例已停；`source-reference/maskit` 黑盒边界继续有效（AGPL）
