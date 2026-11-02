# AICLI Client 实施清单

- 依据方案：`docs/aicli-client-redesign-plan-v2-zh.md`（v2.2）。**全程不改 new-api、不新增服务端、不直接读写任何数据库。**清单与方案冲突时，以本清单为准，并在"偏差记录"写明。
- 代码基线：`Txy-Sky/AICLI-Client@8ff0ac25`。文中行号都基于这个提交，行号漂移时按函数名定位。
- 编写依据：只读了源码，下列命令和测试都没有实际运行过。发现清单有误时按第 0.3 节处理，不要猜。

---

## 0. 执行规则（必读）

### 0.1 标记方法

每一项都有一个复选框和一行证据。**只有验收全部通过后才能标记。**

| 标记 | 含义 |
|---|---|
| `- [ ]` | 未开始或未通过 |
| `- [x]` | 验收全部通过，证据已填写 |
| `- [!]` | 被阻塞。必须写明原因，并在第 12 节"阻塞记录"登记 |
| `- [-]` | 按清单写明的条件跳过，必须写明依据 |

证据格式固定，直接替换 `证据：` 后面的内容：

```text
证据：<提交短哈希> | <实际运行的命令> | <结果，如 "12 passed"> | <日志路径或"无">
```

需要构建或运行 Rust、前端检查的项目，证据改用第 0.6 节的格式。

- 输出较长时存到 `docs/aicli-evidence/<项目编号>.txt`，存之前要去除密钥。
- 标记时立刻提交清单本身，不要攒到最后。
- 我最终验收时会抽查重跑证据里的命令，填不实的证据视为整项未完成。

### 0.2 硬性规则

1. **阶段顺序固定为 A → B → C → E → F → G → J → K → L**。阶段内按编号执行。依赖没完成不能开始；阶段门禁（编号含 `GATE`）没过不能进下一阶段。
2. **`[HUMAN]` 项不得由模型执行**，包括任何生产环境操作。模型做到准备好材料为止，然后标 `[!]` 并等待。
3. **严禁触碰生产**：不连生产 MySQL、不改线上设置、不部署、不执行 `wrangler deploy`。**任何阶段都不得用 SQL 写 new-api 的表，Token 只能通过 new-api 的 HTTP 接口操作。**允许的线上操作只有第 2 节列出的只读 GET 和用测试账号发起的测试请求。
4. **严禁密钥入库**：真实密钥只能通过环境变量读取，不得写入代码、日志、证据、提交信息。
5. **`git add` 只能写明确路径，禁止 `git add -A` 和 `git add .`**。根目录的 `AICLI-Client/` 是未跟踪的完整副本，不得修改、提交或删除。
6. **只改本项要求的内容**。发现其他问题记入第 12 节，不要顺手修。
7. **不得为了通过验收而放宽测试、删除断言或加 `#[ignore]`**。删除功能时可以连带删除其测试，必须在证据中列出删除的测试文件。
8. **`cargo` 命令一律带隔离环境**：`CC_SWITCH_TEST_HOME` 或临时 `HOME`。开发机上真实的 `~/.claude`、`~/.codex` 等配置不得被测试改动。
9. 每项的实现和证据分开提交（见第 0.6 节），提交信息格式 `<type>(<scope>): <项目编号> <说明>`，如 `feat(aicli): C3-04 add line registry`。

### 0.3 遇到清单有误或有歧义

1. 不要自行发明方案。先查方案文档和代码。
2. 如果只是定位点漂移，或者字段名与清单不一致但含义明确，照实现并在第 13 节"偏差记录"写明。
3. 如果会改变行为、接口或数据结构，标 `[!]`，记入第 12 节，等待决定。

### 0.4 固定取值（全文唯一来源，不得改动）

| 项 | 取值 |
|---|---|
| 产品名 | `AICLI Client` |
| Bundle Identifier | `com.aicliapi.com` |
| 深链 Scheme | `aicliapi` |
| 配置目录 | `~/.aicli-client` |
| 数据库文件 | `aicli-client.db` |
| 本地代理默认端口 | `15821` |
| Cargo 包名 / npm 包名 | `aicli-client` |
| 保留不改的内部名 | Rust lib 名 `cc_switch_lib`、`CC_SWITCH_*` 环境变量、`TRAY_ID` 常量、`cc-switch` 字样的内部哈希值 `CODEX_STACK_COMP_HASH`。原因是减少与上游 cherry-pick 的冲突 |
| Keychain service | `com.aicliapi.client` |
| Keychain account | `access-token`、`token:<new-api token id>`、`privacy-hmac-key`、`takeover-backup:<app>` |
| 本地令牌格式 | `aicli-local-<32 位小写十六进制>` |
| 自动创建的 Token 名称 | `aicli:<groups_hash 前 8 位>`；`unlimited_quota=true`，`expired_time=-1` |
| 自动创建的 Token 上限 | 32 个，超过时报错提示用户清理，不得删除用户的 Token |
| 网页个人设置地址 | `https://aicliapi.com/profile` |
| 绑定界面文案（中文，一字不改，其他语言按此翻译） | 标题：`系统访问令牌`<br>输入框占位：`粘贴系统访问令牌（不是 sk- 开头的 API 密钥）`<br>说明：`在网页「个人设置」中生成。令牌只显示一次，请妥善保存；绑定其他设备时需要粘贴同一个令牌。`<br>警告：`重新生成后旧令牌立即失效，所有已绑定的设备都要粘贴新令牌。`<br>误填 sk- 时的错误：`这是 API 密钥，请粘贴网页「个人设置」中的系统访问令牌。` |
| 多设备规则 | 所有设备粘贴**同一个系统访问令牌**（网页生成，不是 `sk-` 密钥）。`sk-` 密钥由客户端用该令牌自动取得，用户不接触。客户端**不调用**任何生成或撤销系统访问令牌的接口 |
| 后台刷新周期 | 打开相关页面时刷新一次；另外每 10 分钟刷新账户和目录 |
| 请求头 | `X-AICLI-Session: <profile_id>:<session_id>`，只发给 new-api |

线路表：

| id | baseUrl | 显示名 | transport | 优先级 | 首字节超时 | 静默超时 | 流式总时长 | 非流式总时长 |
|---|---|---|---|---|---|---|---|---|
| `main` | `https://aicliapi.com` | 主线路 | cloudflare | 100 | 100s | 110s | 600s | 110s |
| `global` | `https://aicli.qzz.io` | 全球备用 | cloudflare | 90 | 100s | 110s | 600s | 110s |
| `apac` | `https://api.aicli.qzz.io` | 亚太加速 | cdn | 80 | 300s | 300s | 600s | 600s |

`main`、`global` 的流式总时长 `600s` 由负责人于 2026-11-02 决定，**未经实测**，不代表已确认 120 秒是空闲上限或总时长上限。A5-01 保持 `[-]`；C4-05 按同一决定不启用并标 `[-]`。其余超时值不变。

### 0.5 涉及的仓库与分支

| 仓库 | 分支 | 放什么 |
|---|---|---|
| `Txy-Sky/AICLI-Client` | `feat/aicli-client-v1`（从 `8ff0ac25` 切出） | 客户端、测试环境、本清单 |
| `QuantumNous/new-api` | 不改 | 只读参考，固定 tag `v1.0.0-rc.34` |
| `Txy-Sky/newapi-cost`、`Txy-Sky/aicli-homepage` | 不改 | 无需访问 |

不合并到任一仓库的 `main`，不发布 Release。

### 0.6 云端构建（GitHub Actions）

本机磁盘不足以承担完整构建，构建和测试一律在 GitHub Actions 上运行。清单中的 `cargo`、`pnpm`、`scripts/aicli-check.sh` 命令都按本节执行，第 0.2 节的其他规则不变。

**提交和标记**

1. 每项先推送实现提交，CI 通过后再追加一个只改清单和证据的提交。修复失败时继续追加提交，提交信息带同一项目编号。禁止强推。
2. 只有 CI 在该项最终实现提交上通过，才能标 `[x]`。当前项未通过前不开始下一项。
3. 证据格式：
   ```text
   证据：<实现提交短哈希> | <job 名和实际命令> | <结果> | <Actions run 链接>
   ```

**工作流**

4. 新增独立工作流，用 `on.push` 只监听 `feat/aicli-client-v1`，并用 `paths-ignore` 排除 `docs/**`，避免证据提交触发构建。现有 `ci.yml` 的 push 只监听 `main`，不会在本分支自动运行。
5. 不要依赖 `workflow_dispatch`：工作流文件不在默认分支上时无法手动触发，而本清单禁止合并到 `main`。
6. 禁止运行 `release.yml` 和 `sync-r2.yml`，不新增或使用签名、发布类 secrets。
7. 日常只跑 Linux。Windows 和 macOS 只在阶段门禁或清单明确要求时运行。

**只在本机运行的内容**

8. 不要把 `AICLI_TEST_ACCESS_TOKEN` 放进 Actions secrets。A5-01、A5-02 这类线上测试在本机用环境变量运行。
9. C5-GATE 和阶段 J 由 CI 构建产物（保留 7 天以内），下载到有真实客户端的机器上测试。
10. A2-01：构建工具链的版本以 CI 日志为证据。本机只安装线上测试和对照工具需要的部分。
11. Maskit 对照（E-07）留在本机运行，不进入工作流和产物。

**验收**：验收人查看或重跑证据中对应提交的 Actions 记录，不在本地编译。

---

## 1. `[HUMAN]` 开工前由负责人提供

模型不得代办。缺哪一项，只阻塞依赖它的项目，其余继续做。

- [ ] **H-01** 测试账号：一个 AICLI 账号，至少能用两个倍率不同且都含同一模型的分组，账户有小额额度。通过环境变量 `AICLI_TEST_ACCESS_TOKEN`（网页个人设置生成的系统访问令牌）提供。依赖项：A5-01、C5-GATE。
  证据：
- [ ] **H-02** 真实客户端：可运行 Claude Code、Codex CLI、Gemini CLI 的环境及其版本号。依赖项：C5-GATE、J。
  证据：
- [ ] **H-03** macOS、Windows 测试机，可操作图形界面的人。依赖项：L。
  证据：
- [x] **H-04** 后台设置：确认 `auto` 在用户可用分组内，`MaxTokenAutoGroups` 不小于单个模型的分组数。判断依据见 A5-02。依赖项：L-05。
  证据：负责人告知已开启 `auto`；A5-02 只读复测确认 `auto_available=true`、`auto_max_count=15`，大于等于最大模型分组数 3；`docs/aicli-evidence/A5-02.txt`。
- [ ] **H-05** 应用图标源文件。没有提供时沿用占位图标（B-08 生成）。依赖项：L-01。
  证据：
- [ ] **H-06** 更新器：签名密钥对、更新清单的公开地址。没有提供时禁用更新器（B-05）。依赖项：L-02。
  证据：
- [ ] **H-07** 法务确认：Magpie/jieba 声明、Maskit 仅作对照的做法。依赖项：L-07。
  证据：
- [ ] **H-08** 运营决定：线上 `RetryTimes` 的取值。依据是 A6-05 的实测结果。依赖项：L-05。
  证据：

---

## 2. 允许的线上只读操作

```text
GET https://{aicliapi.com,aicli.qzz.io,api.aicli.qzz.io}/api/status
GET https://{三条线路}/api/pricing
用 H-01 账号的请求：方案第 3.3 节列出的 /api/* 接口、/v1/models、以及 A5-01、C5-GATE 明确列出的小额模型请求。
只有 C5-GATE 允许在该账号下创建 Token，名称必须符合第 0.4 节，测试后由该流程删除
```

A5-01 另允许 `GET /api/token/` 和 `POST /api/token/batch/keys`，仅用于读取测试账号已有 Token（负责人于 2026-11-01 明确授权）；不得创建、修改或删除 Token，不得生成或撤销系统访问令牌。

**A6-04 本地测试范围（负责人于 2026-11-02 明确授权）**：禁止生成或撤销系统访问令牌的约束适用于线上账号和产品客户端代码，不适用于 A6-04 的隔离 rc.34 测试环境。`test_access_token.py` 仅接受 `127.0.0.1` 或 `localhost`，其他地址直接退出，不读取 `AICLI_TEST_ACCESS_TOKEN`，不使用代理或跟随重定向。通过管理 HTTP API 新建一次性用户，在该用户中通过登录和 Security Proof 取得本地系统访问令牌，然后仅携带该访问令牌调用 `POST /api/user/token`、`DELETE /api/user/self` 验证拒绝行为；不使用 alice、bob 或任何线上账号。两个调用都必须断言响应没有 `success:true`，且调用后原令牌仍可访问 `/api/user/self`、管理 API 确认用户仍存在；实际状态码和 message 写入证据。任一调用意外成功则 A6-04 标 `[!]`，实际响应去除令牌后记入第 12 节，不放宽断言。

---

## A. 阶段 A：环境和基线

### A1-01 建分支并提交方案文档
- [x] 完成（已由负责人安排提交，执行者不需要重做）
- 内容：在分支 `feat/aicli-client-v1` 上删除第一版方案，提交方案 v2.2 和本清单。
- 执行者开工时：`git switch feat/aicli-client-v1`，确认 `git status --short` 中只有 `?? AICLI-Client/`。
- 证据：`git log -1 -- docs/aicli-client-implementation-checklist-zh.md`

### A2-01 安装工具链
- [x] 完成
- 修改：不改仓库。按 `.node-version`（22.12.0）、`rust-toolchain.toml`（1.95，含 rustfmt、clippy）安装 Node、pnpm、Rust、`cargo-nextest`；按 `.github/workflows/ci.yml:143-151` 安装 Linux 依赖；安装 Docker 和 Docker Compose、Go 1.25+（只用于 E-02、E-03 的对照工具）、Python 3.12。
- 验收：`node -v` 为 22.12.0；`rustc -V` 含 1.95；`docker compose version`、`go version`、`python3 --version` 都能输出版本。
- 证据：67dc7e45 | Checklist acceptance (Linux): `node -v; pnpm --version; rustc -V; cargo fmt --version; cargo clippy --version; cargo nextest --version; docker --version; docker compose version; docker info --format '{{.ServerVersion}}'; go version; python3 --version; pkg-config --modversion openssl gtk+-3.0 librsvg-2.0 ayatana-appindicator3-0.1 webkit2gtk-4.1 libsoup-3.0` | CI success（57s）；Node 22.12.0、pnpm 10.12.3、Rust 1.95.0（含 rustfmt/clippy）、nextest 0.9.146、Docker 28.0.4、Compose 2.38.2、Go 1.25.14、Python 3.12.15；Linux 依赖及测试 HOME 隔离断言通过；完整输出见 `docs/aicli-evidence/A2-01.txt` | https://github.com/Txy-Sky/AICLI-Client/actions/runs/41287034200

### A3-01 客户端基线全绿
- [x] 完成
- 修改：新增 `scripts/aicli-check.sh`，依次运行下列命令，任一失败即退出：
  ```bash
  pnpm install --frozen-lockfile
  pnpm typecheck
  pnpm format:check
  pnpm test:unit
  pnpm build:renderer
  cargo fmt --check --manifest-path src-tauri/Cargo.toml
  cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
  cargo nextest run --manifest-path src-tauri/Cargo.toml
  ```
- 验收：退出码 0。把通过的 Rust、前端测试数量写入证据，作为后续"不退化"的基数。**如果基线本身有失败**，把失败用例写入第 12 节并标 `[!]`，不要修。
- 证据：efbd24ef | Checklist acceptance (Linux): `bash scripts/aicli-check.sh（依次执行 pnpm install --frozen-lockfile; pnpm typecheck; pnpm format:check; pnpm test:unit; pnpm build:renderer; cargo fmt --check --manifest-path src-tauri/Cargo.toml; cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings; cargo nextest run --manifest-path src-tauri/Cargo.toml）` | CI success（16m03s），脚本退出码 0；前端 266 个文件、3415 tests passed；Rust 4443 passed、10 skipped（原有基线，未修改测试或新增 ignore）；完整日志见 `docs/aicli-evidence/A3-01.txt` | https://github.com/Txy-Sky/AICLI-Client/actions/runs/41287292774

### A5-01 实测 120 秒语义和三条线路 `[需 H-01]`
- [-] 按负责人决定跳过 180 秒持续输出语义测量：直连、鉴权、预算守卫、离线测试和 CI 均已有证据；120 秒语义仍未实测定案。负责人于 2026-11-02 另行决定 `main`、`global` 流式总时长为 `600s`，已记入第 0.4 节；该值不是测量结论。C4-05 按同一决定不启用。`gpt-image-2` 只支持 `openai`/`image-generation`、属于 `quota_type=1`，未发送不匹配请求。历史测量详见 `docs/aicli-evidence/A5-01-streaming.txt`，本次决定见 `docs/aicli-evidence/A5-01-owner-decision.txt`。
- 修改：新增 `scripts/aicli-poc/line_probe.py`，读 `AICLI_TEST_ACCESS_TOKEN`。
- 步骤：
  1. 三条线路各请求 `/api/status`，记录 TLS 结果、HTTP 状态、延迟，连续 10 次。
  2. **空闲场景**：对每条线路发一个流式请求，提示词要求模型先长时间思考（或使用最慢模型），记录相邻两个字节的最大间隔，以及连接被断开时的累计时长。
  3. **持续输出场景**：要求模型输出足够长的文本，使总时长超过 180 秒，记录是否在约 120 秒被断开。
- 单次请求预算不超过测试账号额度的 5%。
- 原测量判定规则（保留供历史核对；测量已按负责人决定跳过，不据此推断当前采用值）：
  - 持续输出能超过 180 秒 → 120 秒是**空闲上限**。把 `main`、`global` 的流式总时长定为 `600s`。
  - 持续输出在约 120 秒被断开 → 120 秒是**总时长上限**。把 `main`、`global` 的流式总时长定为 `110s`，并在 C4-05 启用"流式请求默认首选 `apac`"。
  - 结果不明确 → 标 `[!]`，不要猜。
- 当前采用值：按负责人于 2026-11-02 的决定，第 0.4 节 `main`、`global` 均为 `600s`，未经实测，不形成 120 秒语义结论；C4-05 不启用。
- 证据：40d1ca64 | Checklist acceptance (Linux): `python3 -m unittest discover -s scripts/aicli-poc -p 'test_line_probe.py' -v; bash scripts/aicli-check.sh`；本机隔离 HOME、直连执行 `python3 -B -u scripts/aicli-poc/line_probe.py --model codex-auto-review --max-output-tokens 65536 --output /tmp/aicli-A5-01-online-bounded.json` 等实际启动命令见 `docs/aicli-evidence/A5-01-streaming.txt` | CI success：探测离线测试 21 passed；前端 266 文件/3415 passed；Rust 4443 passed/原有 10 skipped。线上验收未通过：持续输出时长不足且有一次补测读超时，120 秒语义未定；只读核对的 `gpt-image-2` 不支持所需 SSE 文本端点，未发送不匹配请求；该次测量未回填第 0.4 节、当时未开始 A5-02。当前 600s 采用值来自负责人 2026-11-02 的另行决定，不是本次历史测量的结论。历史网络失败保留在 A5-01.txt，直连恢复证据在 A5-01-direct-network.txt | https://github.com/Txy-Sky/AICLI-Client/actions/runs/41297271375

### A5-02 真实账号能力核对 `[需 H-01]`
- [x] 完成：只读复测确认五个接口均返回 200，`auto` 已在可用分组内，`auto_max_count=15`，单模型最大可用分组数为 3。
- 修改：`scripts/aicli-poc/account_probe.py`，只读。
- 验收，把结论写入证据：
  - 只带 `Authorization: Bearer <令牌>`（不带 `New-Api-User`）调用 `/api/user/self`、`/api/user/self/groups`、`/api/token/auto-groups`、`/api/log/self?p=1&page_size=1`，都返回 200。
  - 记录 `/api/user/self/groups` 是否含 `auto`、`max_count`，以及每个模型在该账号可用的分组数最大值（`/api/pricing` 的 `enable_groups` 与可用分组取交集）。
  - 没有 `auto` 或 `max_count` 小于最大分组数 → 标 `[!]`，交 H-04 决定，不要绕过。
- 证据：bf8e5f88 | Checklist acceptance (Linux): `python3 -m unittest discover -s scripts/aicli-poc -p 'test_account_probe.py' -v; bash scripts/aicli-check.sh`；本机隔离 HOME：`python3 /tmp/aicli-a5-run-with-env.py /home/txy/.local/bin/aicli-direct python3 -B -u scripts/aicli-poc/account_probe.py --output /tmp/aicli-A5-02-account-rerun.json` | CI success（A5-02 离线安全测试、A3-01 基线通过：Rust 4443 passed/10 skipped，前端 266 文件/3415 passed）；线上复测五个接口均 HTTP 200、`auto_available=true`、`max_count=15`、最大模型分组数 3、结论 `pass` | https://github.com/Txy-Sky/AICLI-Client/actions/runs/41301708173

### A6 测试环境（未修改的 new-api rc.34）

#### A6-01 编写 compose 环境
- [x] 完成
- 修改：新增 `tests/aicli-testenv/`：
  - `docker-compose.yml`：`new-api` 固定为 `calciumion/new-api@sha256:d9956dc8bde83cb1378932802670350b3c4c1ee7611d2e0d7c63af111da2b989`（线上在用的 rc.34 镜像）；`mysql:5.7.44`；`redis`；`mock-upstream`；端口全部绑定 `127.0.0.1`。
  - `seed.py`：通过 new-api 管理 API 创建管理员、两个测试用户（`alice`、`bob`）、三个分组（`g-cheap` 倍率 0.5、`g-mid` 倍率 1、`g-pro` 倍率 2），把 `auto` 加入可用分组，`MaxTokenAutoGroups=10`，并给每个分组配一个指向 `mock-upstream` 的渠道（`g-cheap`、`g-mid` 都含 `mock-model-a`，`g-pro` 只含 `mock-model-b`）。
  - `.env.example`：只放变量名和假值。
- 验收：`docker compose up -d` 后 `curl -s 127.0.0.1:<port>/api/status` 版本为 `v1.0.0-rc.34`；`seed.py` 重复运行结果不变。
- 证据：75f6e4d7 | job `Checklist acceptance (Linux)`，step `A6-01 compose rc.34 environment and idempotent seed`：`docker compose -p "$project" -f tests/aicli-testenv/docker-compose.yml up -d`; `curl --fail --silent --show-error http://127.0.0.1:13000/api/status`; `python3 tests/aicli-testenv/seed.py`（两次）；取证 step 执行 `docker compose logs --timestamps mysql new-api` 及 `docker inspect --format '{{.State.StartedAt}}'`，输出前过滤；`bash scripts/aicli-check.sh` | 同一 SHA 首次运行及 `gh run rerun 41379889841` 各完整通过（attempt 1 / 2）；两次均确认 MySQL 的 port 3306 就绪早于 new-api StartedAt，new-api 无 connection refused；基线两次均前端 266 文件/3415 passed，Rust 4443 passed/原有 10 skipped。时间点和原始 run 链接见 `docs/aicli-evidence/A6-01.txt`；日志只在 job 日志中，未写入证据或 artifact | https://github.com/Txy-Sky/AICLI-Client/actions/runs/41379889841
- 历史证据（保留；原环境验收未暴露初始化竞争，不作为本次修正的两次验收）：4945b896 | Checklist acceptance (Linux), job `A6-01 compose rc.34 environment and idempotent seed`: `docker compose -p "$project" -f tests/aicli-testenv/docker-compose.yml up -d`; `curl --fail --silent --show-error http://127.0.0.1:13000/api/status`; `python3 tests/aicli-testenv/seed.py`（连续运行两次）；`bash scripts/aicli-check.sh` | CI run success；rc.34 `/api/status` 版本断言通过，seed 重复运行通过，A3-01 基线通过（Rust 4443 passed/10 skipped，前端 266 文件/3415 passed） | https://github.com/Txy-Sky/AICLI-Client/actions/runs/41307378844

#### A6-02 编写 mock-upstream
- [x] 完成
- 修改：`tests/aicli-testenv/mock_upstream/`，Python 实现。支持 `/v1/chat/completions`、`/v1/responses`、`/v1/messages`、Gemini `:generateContent`/`:streamGenerateContent`，流式和非流式都支持。通过请求里的模型名后缀或测试控制接口 `POST /__control` 注入行为：`delay_first_byte`、`idle_gap`、`fail_status`、`fail_after_n_chunks`、`stream_error_event`。记录每个请求的完整请求体，可经 `GET /__requests` 读出。
- 验收：pytest 用例覆盖每种协议的流式和非流式，覆盖每种注入行为。
- 证据：e45c2cde | Checklist acceptance (Linux), job `A6-02 mock upstream protocol and injection tests`: `python3 -m pip install --user pytest`; `PYTHONPATH=tests/aicli-testenv python3 -m pytest -q tests/aicli-testenv/tests/test_mock_upstream.py` | CI run success；协议流式/非流式与行为注入 pytest 通过，A3-01 基线同 run 通过 | https://github.com/Txy-Sky/AICLI-Client/actions/runs/41311104301

#### A6-03 实测：受管 Token 路由（方案 PoC B 前半）
- [x] 完成：A6-01 前置环境修正通过后，已用本项独立实现提交 `53567f1c` 及其完整 CI 重新验收；真正系统访问令牌、精确 request_id、等量用量和约 1:2 quota 比例全部通过，不沿用 `c66defc2` 的旧证据。
- 修改：`tests/aicli-testenv/tests/test_token_routing.py`。用 alice 的系统访问令牌调用 `POST /api/token/` 创建 Token：T1 `Group=auto, AutoGroups=[g-cheap,g-mid]`；T2 `Group=auto, AutoGroups=[g-mid,g-cheap]`；T3 `Group=auto, AutoGroups=[g-cheap,g-pro]`。
- 验收（断言 `logs` 表的 `group`、`quota`）：
  - T1 请求 `mock-model-a` → 分组 `g-cheap`；T2 → `g-mid`；quota 比例约为 1:2。
  - T3 请求 `mock-model-b` → 分组 `g-pro`（第一组没有该模型时跳组）。
  - 响应头含 `X-Oneapi-Request-Id`，用它在 `/api/log/self?request_id=` 能查到对应行。
- 证据：53567f1c | job `Checklist acceptance (Linux)`，step `A6-03 managed token routing integration test`：`docker compose -p "$project" -f tests/aicli-testenv/docker-compose.yml up -d`; `python3 tests/aicli-testenv/seed.py`; `docker compose -p "$project" -f tests/aicli-testenv/docker-compose.yml restart new-api`（原有 seed 后刷新配置，未新增重启逻辑）；`PYTHONPATH=tests/aicli-testenv python3 -m pytest -q -s --tb=short tests/aicli-testenv/tests/test_token_routing.py`; `bash scripts/aicli-check.sh` | CI success；真实本地系统访问令牌；1 passed in 0.63s；T1 g-cheap/quota=98，T2 g-mid/quota=195，均 prompt_tokens=192、completion_tokens=3，T2/T1=1.989796（2% 容差通过）；T3 g-pro/quota=390；三条响应 request_id 均精确查到对应账单行；前端 266 文件/3415 passed，Rust 4443 passed/原有 10 skipped。详见 `docs/aicli-evidence/A6-03.txt` | https://github.com/Txy-Sky/AICLI-Client/actions/runs/41383467961
- 历史证据（旧测试缺少系统访问令牌和比例断言，不作为重新验收依据）：c66defc2 | job `Checklist acceptance (Linux)`，命令：`docker compose -p "$project" -f tests/aicli-testenv/docker-compose.yml up -d`; `python3 tests/aicli-testenv/seed.py`; `docker compose -p "$project" -f tests/aicli-testenv/docker-compose.yml restart new-api`; `PYTHONPATH=tests/aicli-testenv python3 -m pytest -q tests/aicli-testenv/tests/test_token_routing.py`; `bash scripts/aicli-check.sh` | CI success；A6-01 种子两次 PASS，A6-03 路由断言 1 passed，基线 Rust 4443 passed/10 skipped、前端 3415 passed | https://github.com/Txy-Sky/AICLI-Client/actions/runs/41317794458

#### A6-04 实测：系统访问令牌的能力边界
- [x] 完成：负责人已明确本地测试授权范围；两个敏感接口均被拒绝，原令牌仍可用且用户仍存在。仅使用隔离容器中新建的一次性用户。
- 修改：同目录 `test_access_token.py`。
- 验收（只带 `Authorization`，不带 `New-Api-User`）：
  - 返回成功：`GET /api/user/self`、`/api/user/self/groups`、`/api/token/`、`/api/token/auto-groups`、`/api/pricing`、`/api/log/self?request_id=`、`/api/log/self/stat`；`POST /api/token/`；`POST /api/token/batch/keys`；`DELETE /api/token/:id`。
  - `POST /api/token/` 响应不含 key 和 id，要用 `GET /api/token/` 按名称取 id，再用 `batch/keys` 取明文。把 `batch/keys` 返回值是否带 `sk-` 前缀写入证据，C1-09 按实际处理。
  - 必须失败：访问令牌调用 `/v1/models` 返回 401；`sk-` 调用 `/api/user/self` 返回 401；访问令牌调用 `POST /api/user/token`、`DELETE /api/user/self` 被拒绝（需要 Security Proof）。
  - `POST /api/token/` 带 `{"name":"aicli:test","group":"auto","auto_groups":["g-cheap","g-mid"],"cross_group_retry":true,"unlimited_quota":true,"expired_time":-1}`，从列表读回的 `auto_groups` 顺序一致。
  - 测试结束时删除的 Token 立即不可用（带重试断言，把延迟写入证据）。
- 证据：cf9147a5 | job `Checklist acceptance (Linux)`，step `A6-04 system access token boundaries in local disposable users`：`python3 -m unittest discover -s tests/aicli-testenv/tests -p 'test_access_token_safety.py' -v`; `docker compose -p "$project" -f tests/aicli-testenv/docker-compose.yml up -d`; `python3 tests/aicli-testenv/seed.py`; `docker compose -p "$project" -f tests/aicli-testenv/docker-compose.yml restart new-api`; `python3 -m pytest -q -s --tb=short tests/aicli-testenv/tests/test_access_token.py`; `bash scripts/aicli-check.sh` | CI success；地址防护 1 test OK、接口验收 3 passed；POST `/api/user/token` 和 DELETE `/api/user/self` 均 HTTP 403、`success:false`、message=`安全验证状态无效`、code=`SECURITY_PROOF_INVALID`，两次均 `original_token_valid=true`、`user_exists=true`；`batch_keys_has_sk_prefix=false`，删除后第 1 次请求返回 401，延迟 0.001169s；基线 Rust 4443 passed/原有 10 skipped、前端 266 文件/3415 passed；详见 `docs/aicli-evidence/A6-04.txt` | https://github.com/Txy-Sky/AICLI-Client/actions/runs/41369181694

#### A6-05 实测：亲和规则和流中重试
- [ ] 完成
- 修改：同目录 `test_affinity_retry.py`。按方案 3.4 配置新亲和规则（只在测试环境，放在规则列表**最前**，`KeySources=[{type:request_header,key:X-AICLI-Session}]`，`SkipRetryOnFailure=false`，`IncludeUsingGroup=true`，`IncludeRuleName=true`）。
- 验收：
  - 同一 `X-AICLI-Session` 连续 5 次请求落在同一渠道；不同 profile 前缀互不影响。
  - 不带该请求头时，默认规则的行为与配置前一致（对比快照）。若新规则抢占了默认规则，标 `[!]`。
  - `RetryTimes=0` 和 `RetryTimes=2` 时，分别用 `stream_error_event` 和 `fail_after_n_chunks` 触发错误，**如实记录**客户端收到的 SSE 是否出现重复内容。此项不要求通过，只要求结论写入证据，供 H-08 决策。
- 证据：

### A-GATE 阶段 A 门禁
- [ ] 完成
- 验收：A1～A6 全部 `[x]`（A5-01、A5-02 可因 H-01 缺失为 `[!]`，但必须在 C5-GATE 之前解决）。
- 证据：

---

## B. 阶段 B：身份改造

目的是先把数据目录和端口隔离开，后面运行应用才不会碰到 cc-switch 的真实数据。**只改身份，不改行为。**

### B-01 配置目录
- [ ] 完成
- 修改：
  - `src-tauri/src/config.rs`：新增 `pub const APP_DIR_NAME: &str = ".aicli-client";`，`get_app_config_dir` 用它。**删除** `:320-350` 一带兼容 v3.10.3 的 HOME 回退（含 Windows 分支），测试旁路也一并去掉。
  - 下列文件中的 `.join(".cc-switch")` 全部改用该常量：`live/engine.rs:36`（`DeviceStore::for_device`）、`panic_hook.rs:28`、`settings.rs:729`，并同步修改相关测试断言（如 `panic_hook.rs:253`、`claude_desktop_config.rs:1667`）。
  - 数据库文件名改为 `aicli-client.db`：`lib.rs:601`、`database/mod.rs:101`。
  - `dsh_config.rs` 的 `.cc-switch-profile-sync.json` 改为 `.aicli-client-profile-sync.json`。
- 验收：
  - `grep -rn '"\.cc-switch"' src-tauri/src` 无结果。
  - 新增测试：临时 HOME 下初始化数据库和 `DeviceStore`，路径都在 `.aicli-client` 内，且 `.cc-switch` 不存在。
  - `scripts/aicli-check.sh` 通过。
- 证据：

### B-02 端口
- [ ] 完成
- 修改：`15721` → `15821`：`proxy/types.rs:46`、`database/dao/proxy.rs:60`、`database/schema.rs` 中的默认值，以及相关测试和字符串。`CC_SWITCH_PROXY_PORT` 环境变量名保留。
- 验收：`grep -rn 15721 src-tauri/src src` 无结果；检查通过。
- 证据：

### B-03 标识和产品名
- [ ] 完成
- 修改：
  - `src-tauri/tauri.conf.json`：`productName`=`AICLI Client`，`identifier`=`com.aicliapi.com`，`version` 改 `0.1.0`；`tauri.windows.conf.json` 的窗口标题。
  - `src-tauri/Cargo.toml`：`name`=`aicli-client`，`version`=`0.1.0`，`description`、`authors`、`repository` 改为本项目；`[lib] name` 保留。
  - `package.json`：`name`=`aicli-client`，`version`=`0.1.0`。
  - `flatpak/`：重命名为 `com.aicliapi.com.*`，内容同步。
  - `src-tauri/src/lib.rs:556` 日志文件名 `cc-switch` → `aicli-client`。
  - `src/lib/navigation.ts` 的 `VIEW_STORAGE_KEY`、`APP_STORAGE_KEY` 前缀改为 `aicli-client-`。
- 验收：`cargo check`、`pnpm typecheck` 通过；`grep -n 'ccswitch\|CC Switch' src-tauri/tauri.conf.json src-tauri/Cargo.toml package.json` 无结果。
- 证据：

### B-04 深链 Scheme 注册
- [ ] 完成
- 修改：
  - `tauri.conf.json:61` schemes 改 `["aicliapi"]`；`Info.plist:13`；`src-tauri/wix/per-user-main.wxs:225-236`。
  - `lib.rs:280`（`handle_deeplink_url` 的前缀）、`deeplink/parser.rs:22`（scheme 校验）改 `aicliapi`。
  - `lib.rs:1143` 判断文件名改为实际生成的处理器文件名（以 tauri-plugin-deep-link 对新包名的生成结果为准，把实测文件名写入证据）。
  - `lib.rs:304` 的缺陷：`PendingDeepLink::Error` 里的 `url` 改为 `url_for_log(url_str)` 的结果，不得保留 query。
- 验收：
  - `cargo nextest run deeplink` 通过。
  - 新增测试：解析失败的 URL 带 `accessKey=SECRET` 时，`PendingDeepLink::Error.url` 不含 `SECRET`。
  - `ccswitch://` 被拒绝。
- 证据：

### B-05 更新器和发布工作流
- [ ] 完成
- 修改：
  - `tauri.conf.json`：`createUpdaterArtifacts`=false，`plugins.updater.endpoints`=`[]`，`pubkey` 置空。若构建要求非空，移除 updater 插件的配置项并在 `lib.rs` 中按 `updater_enabled()`（常量，当前返回 false）跳过初始化；前端"检查更新"入口隐藏。
  - 删除 `.github/workflows/` 下的 `schema-version.yml`、`sync-r2.yml`、`stale.yml`、`labeler.yml`、`claude.yml`、`wsl2-nightly.yml`。
  - `release.yml`：保留，所有 `cc-switch`、`CC-Switch` 产物名改为 `aicli-client`、`AICLI-Client`；Apple 签名和 updater 签名步骤改为 `if: ${{ vars.AICLI_RELEASE_SIGNING == 'true' }}`。
- 验收：`grep -n 'farion1231\|ccswitch.io' src-tauri/tauri.conf.json .github/workflows/*` 无结果；检查通过。
- 证据：

### B-06 Codex 配置里的品牌标识
- [ ] 完成
- 修改：`codex_config.rs:19-25`：`CC_SWITCH_CODEX_OFFICIAL_PROXY_PROVIDER_ID` 的值改 `aicli-official`，`CC_SWITCH_CODEX_MODEL_CATALOG_FILENAME` 的值改 `aicli-model-catalog.json`。常量名保留。`CC_SWITCH_LEGACY_CODEX_MODEL_PROVIDER_IDS` 不再包含 cc-switch 旧值时，保留空实现。
- 验收：`cargo nextest run codex` 通过；断言文本里的旧值同步更新。
- 证据：

### B-07 用户可见文案
- [ ] 完成
- 修改：5 个 `src/i18n/locales/*.json` 中 `CC Switch`、`cc-switch` 的用户可见文案改 `AICLI Client`；`README*.md`、`CHANGELOG.md`、`CONTRIBUTING.md`、`SUPPORT.md`、`SECURITY.md`、`deplink.html`、`session-manager.md`：删除多语言 README 和 `deplink.html`，`README.md` 重写为一页（产品说明 + 开发命令 + 上游声明"基于 farion1231/cc-switch，MIT"），`LICENSE` 保留并追加本项目版权行，`CHANGELOG.md` 清空为新起点。
- 验收：`grep -ril 'cc switch\|cc-switch' src/i18n` 无结果；`LICENSE` 中保留原作者版权声明；检查通过。
- 证据：

### B-08 占位图标
- [ ] 完成
- 修改：用 `pnpm tauri icon` 从一张自制的纯色 `AI` 字样 PNG 生成全部图标，覆盖 `src-tauri/icons/`。
- 验收：`pnpm tauri build --debug --no-bundle` 成功；文件存在。
- 证据：

### B-GATE 阶段 B 门禁
- [ ] 完成
- 验收：
  1. `HOME=$(mktemp -d) xvfb-run pnpm tauri dev` 能启动（或 `[HUMAN]` 在桌面环境启动），临时 HOME 下只生成 `.aicli-client`。
  2. 本地代理监听 `127.0.0.1:15821`。
  3. `scripts/aicli-check.sh` 通过，测试数量相对 A3-01 的减少都能在证据中解释。
- 证据：

---

## C. 阶段 C：路由主链路（AICLI-Client 仓库）

新代码全部放在 `src-tauri/src/aicli/`。这里直接调用 new-api 现有接口，不依赖任何新服务端。`lib.rs` 增加 `pub mod aicli;`。对现有文件只做本节写明的最小改动。

### C1-01 线路注册表
- [ ] 完成
- 修改：`aicli/lines.rs`，按第 0.4 节定义静态表 `LINES`，`Line { id, base_url, label_key, transport, priority, first_byte_s, idle_s, stream_total_s, non_stream_s }`。`pub fn line(id) -> Option<&'static Line>`。没有任何增删改接口。
- 验收：单测断言三条线路的每个字段与第 0.4 节一致；URL 不含尾斜杠。
- 证据：

### C1-02 Keychain
- [ ] 完成
- 修改：`src-tauri/Cargo.toml` 增加 `keyring`（启用 apple-native、windows-native、sync-secret-service）。`aicli/secrets.rs`：`trait SecretStore { get, set, delete }`，实现 `KeyringStore`、`FileFallbackStore`（`~/.aicli-client/secrets.json`，Unix 权限 0600，Windows 使用用户目录默认 ACL）、测试用 `MemoryStore`。`secret_backend() -> {keychain|file_fallback}`。优先 Keychain，首次读写失败时退回文件并置 `file_fallback`，供设置页显示。
- 验收：单测覆盖读写删、回退；文件权限断言（Unix）；`Debug` 输出不含值；`cargo clippy` 通过。Linux 无 Secret Service 时测试走回退路径。
- 证据：

### C1-03 数据库 v25
- [ ] 完成
- 修改：`database/mod.rs` `SCHEMA_VERSION=25`；`database/schema.rs` 的建表函数和 `apply_schema_migrations_on_conn` 增加 `24 => migrate_v24_to_v25`；在 `proxy_request_logs` 的建表语句和迁移中同时加列。新增：
  ```text
  aicli_account(id INTEGER PK CHECK(id=1), user_id INTEGER, username TEXT, user_group TEXT, selected_line_id TEXT NOT NULL DEFAULT 'main', active_profile_id TEXT, privacy_enabled INTEGER NOT NULL DEFAULT 0, privacy_failure_mode TEXT NOT NULL DEFAULT 'block', allow_unscanned_media INTEGER NOT NULL DEFAULT 0, last_sync_at INTEGER)
  aicli_cache(kind TEXT PK, fingerprint TEXT, fetched_at INTEGER NOT NULL, body_json TEXT NOT NULL)   -- kind: account|groups|pricing|usage_today|desktop_alias
  aicli_profile(id TEXT PK, name TEXT NOT NULL, source TEXT NOT NULL, preset_id TEXT, revision INTEGER NOT NULL, body_json TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)
  aicli_scope_binding(scope TEXT NOT NULL, scope_key TEXT NOT NULL, profile_id TEXT NOT NULL, PRIMARY KEY(scope, scope_key))   -- scope: global|app|project|session
  aicli_managed_token(token_id INTEGER PK, groups_hash TEXT NOT NULL UNIQUE, groups_json TEXT NOT NULL, name TEXT NOT NULL, masked TEXT NOT NULL, created_by_client INTEGER NOT NULL)
  aicli_usage_daily(day TEXT PK, quota INTEGER NOT NULL, requests INTEGER NOT NULL, is_final INTEGER NOT NULL, fetched_at INTEGER NOT NULL)
  aicli_local_token(app_type TEXT PK, token TEXT NOT NULL, created_at INTEGER NOT NULL)
  aicli_line_health(line_id TEXT NOT NULL, checked_at INTEGER NOT NULL, status TEXT NOT NULL, latency_ms INTEGER, http_status INTEGER, error_code TEXT, PRIMARY KEY(line_id, checked_at))
  aicli_takeover(app_type TEXT PK, state TEXT NOT NULL, backup_ref TEXT, taken_over_at INTEGER)
  proxy_request_logs 增加: aicli_line_id TEXT, aicli_profile_id TEXT, upstream_request_id TEXT, server_group TEXT, billed_quota INTEGER, billed_at INTEGER
  ```
  数据库中不存访问令牌和 `sk-` 明文。
- 验收：单测：v24 库迁移到 v25 不丢数据且可重复；新库结构与迁移结果一致；现有 schema 测试更新版本后通过。
- 证据：

### C1-04 new-api 客户端
- [ ] 完成
- 修改：`aicli/newapi.rs`。`NewApiClient`，基于现有 `reqwest`，按所选线路请求，网络失败或 HTML 5xx 时依次尝试其余线路（HTTP 4xx 不换线路）。只带 `Authorization: Bearer <访问令牌>`。封装：`user_self`、`user_groups`、`pricing`、`auto_groups_setting`、`list_tokens`（翻页取全）、`create_token`、`delete_token`、`batch_token_keys`、`log_by_request_id`（`/api/log/self?request_id=&p=1&page_size=1`）、`log_stat(start,end)`（`type=2`）、`log_count(start,end)`（`/api/log/self` 的 `total`）。new-api 返回的 `success=false` 和 401 都映射成明确错误；401 映射为 `NewApiError::TokenInvalid`。连接超时 10s，总超时 20s。
- 验收：用 mock 覆盖线路回退、翻页、错误映射；日志和错误文本中不出现 `Authorization` 和响应中的 key（有断言）。对 A6 环境的集成测试覆盖全部封装。
- 证据：

### C1-05 绑定和解绑
- [ ] 完成
- 修改：`aicli/account.rs`：
  - `bind(access_token, line)`：去除首尾空白；以 `sk-` 开头时直接返回 `WrongCredentialType`，不发请求；按线路调用 `user_self` 校验，失败不保存任何内容；成功后令牌写 Keychain（`access-token`），写 `aicli_account`，触发 C1-06 的刷新。切换到不同 `user_id` 时清空缓存、配置集和 Token 记录。
  - `unbind(delete_created_tokens: bool)`：还原全部接管（C3-10）；清除 Keychain 中的 `access-token` 和全部 `token:*`；清空账户相关表，但**保留** `proxy_request_logs`。`delete_created_tokens=true` 时才删除 `created_by_client=1` 的 Token，默认 false，因为这些 Token 可能被该账号的其他设备共用。访问令牌无法撤销，界面要提示用户到网页处理。
  - 运行中收到 `TokenInvalid`：转为"需要重新绑定"状态，保留缓存可读，模型请求返回 `not_bound`。
- 验收：单测覆盖误填 `sk-` 被拒且没有网络请求、校验失败不落盘、换账号清空、解绑两种选项、令牌失效状态；断言令牌不进 SQLite（搜索数据库文件内容）。令牌失效后，再粘贴同账号的新令牌时，配置集、缓存和 Token 记录都保留。
- 证据：

### C1-06 缓存优先的加载
- [ ] 完成
- 修改：`aicli/cache.rs`。所有读取走同一流程：**立即返回 `aicli_cache` 中的值，并附 `fetched_at` 和 `stale` 标记；后台刷新；成功后写缓存并发事件 `aicli-data-updated`；失败时保留缓存并记录 `last_error`。** 同一 `kind` 同时只允许一个刷新任务。
  - `account`、`groups`、`pricing`：打开相关页面时和每 10 分钟刷新。刷新结果按内容 sha256 算 `fingerprint`，未变化时不发事件，也不重新编译路由。**不得使用 `pricing_version`。**
  - 用量：过去日期写入 `aicli_usage_daily`（`is_final=1`）后不再请求；今天每次打开页面刷新。首次绑定回填最近 30 天，并发不超过 2，后台进行，不阻塞界面。
- 验收：
  - 单测：有缓存时立即返回且不等待网络（用永不返回的 mock 断言）；刷新失败不覆盖缓存；无缓存且失败时返回 `None`，不是 0；并发去重；内容不变不发事件。
  - 过去日期只请求一次。
  - 集成测试统计：冷启动后 10 分钟内，对 new-api 的 `/api/*` 请求数不超过 40（含 30 天回填）。new-api 对访问令牌请求写审计，所以不能轮询。
- 证据：

### C1-07 目录
- [ ] 完成
- 修改：`aicli/catalog.rs`，从缓存的 `groups`、`pricing` 计算：
  - 分组：`/api/user/self/groups`，排除 `auto`；倍率取其中的 `ratio`。缺少倍率时 `ratio_known=false`，**不得当作 1**。`auto_available` 记录是否含 `auto`。
  - 模型：`pricing.data[]`，`groups = enable_groups ∩ 可用分组`，为空的模型排除。保留 `supported_endpoint_types`、`billing_mode`、`billing_expr`、`quota_type`、`model_price` 等，**不自行计算最终费用**。
  - `wire`：对每个客户端协议，端点含该协议就透传；否则按 `openai-response > openai > anthropic > gemini` 取第一个可用的。可被 `aicli/compat.json`（`include_str!`）覆盖，键 `<client_protocol>|<model>`。
  - `compat`：来自同一文件，取值 `full|partial|text_only|unavailable`，缺省 `unverified`。初始为空，C5-GATE 回填。
- 验收：对 A6 环境，每个分组的模型集合等于 `GET /api/user/models?group=X` 与 pricing 的交集；配置特殊可用分组（含 `-:`、`+:`）后仍一致；`svip` 这类不可用分组不出现（用测试数据模拟）。
- 证据：

### C1-08 配置集和路由编译
- [ ] 完成
- 修改：`aicli/profiles.rs`。存 `aicli_profile`，`body_json`：
  ```json
  {"selected_groups":["g-cheap","g-mid"],"selection_mode":"fixed_priority",
   "model_bindings":[{"model":"mock-model-a","source_group":"g-mid","alias":null}],
   "resolved_conflicts":["mock-model-a"],"line_policy":"inherit","allow_fallback":true}
  ```
  - 预设（内置，只读，不入库）：`recommended`：线上 `pricing.auto_groups` 与可用分组的交集，保持顺序；`low_cost`：可用分组按倍率升序，`ratio_known=false` 的排最后。用户"从预设新建"后才入库，`source=system_copy`。未绑定配置集时用 `recommended`。
  - 保存校验：分组都可用；每个多来源模型都在 `resolved_conflicts` 或 `model_bindings` 中，否则返回未解决的模型列表；alias 唯一；`revision` 不一致时报冲突。预设不校验冲突，按顺序取第一个。
  - `selection_mode`：`fixed_priority` 按 `selected_groups` 原顺序；`cost_first` 按有效倍率升序。
  - `compile(profile, catalog) -> Vec<ModelRoute>`：对配置集中的每个模型：有 `source_group` → `[source_group]`，`allow_fallback=true` 时追加其余含该模型的分组；否则 → `selected_groups` 中含该模型的分组，保持顺序。**长度超过 `max_count` 时报错，不得截断。**
  - `needs_repair`：引用的分组已不可用时为 true，附缺失分组和受影响模型。分组被撤销时不自动删除用户配置。
- 验收：单测覆盖全部校验分支、两种模式排序、截断报错、`needs_repair`。用线上形状的夹具（`gpt/gpt-max/gpt-pro`、`claude-aws/claude-code/claude-kiro`、`gemini-cli`、`xAI` 各成一簇）断言：69 个模型编译后不同的分组顺序不超过 8 种。
- 证据：

### C1-09 取得 sk- Token
- [ ] 完成
- 修改：`aicli/tokens.rs`，只调 HTTP 接口。`ensure_token(ordered_groups) -> token_id`：
  - `groups_hash = sha256("\n".join(ordered_groups))`。先查 `aicli_managed_token`，并确认 Keychain 中有明文。
  - 否则 `list_tokens` 找**可复用**的：`status=1`；`expired_time=-1` 或剩余超过 30 天；`unlimited_quota=true`；`model_limits_enabled=false`；`allow_ips` 为空；单分组时 `group` 相等，多分组时 `group=auto` 且 `auto_groups` 顺序完全相同。复用的 Token 记 `created_by_client=0`，**不得修改或删除**。
  - 仍没有才创建：单分组 `{"name","group":<分组>,"unlimited_quota":true,"expired_time":-1}`；多分组 `{"name","group":"auto","auto_groups":[...],"cross_group_retry":true,"unlimited_quota":true,"expired_time":-1}`。多分组且 `auto_available=false` → 返回 `AutoGroupUnavailable`，不得退化成只用第一组。`created_by_client=1` 的数量达 32 时返回 `TokenLimit`。
  - 创建响应没有 id，再次 `list_tokens` 按名称取回。名称已存在但配置不同（用户改过）时，名称追加序号再建，不覆盖。
  - 明文：用 `batch_token_keys` **一次取本次缺的全部**，存 Keychain（`token:<id>`），按 A6-04 的结论补 `sk-` 前缀。遇到限流（429）时记录并按 Retry-After 延后，不得循环重试。
  - 只在编译结果变化时运行。请求时如果 Token 返回 401（被用户删除），删除本地记录，触发一次重建。
- 验收：对 A6 环境集成测试：
  - 重复调用只创建一次；已有合适的用户 Token 时不创建；用户 Token 有额度限制或模型限制时不复用。
  - 69 个模型的编译结果只产生一次 `batch_token_keys` 调用。
  - 用户在网页删除 Token 后，下次请求自动恢复。
  - **多设备**：用两个独立的临时 HOME 模拟两台设备，绑定同一令牌。第二台不再创建 Token，只读取第一台创建的同一批 Token，两台都能发请求。任一台 `unbind(false)` 后，另一台不受影响。
  - `grep -rniE 'mysql|new_api\.' src-tauri/src/aicli src-tauri/Cargo.toml` 无结果：客户端不连接 new-api 的数据库。
- 证据：

### C2-01 运行时状态
- [ ] 完成
- 修改：`aicli/runtime.rs`：
  ```rust
  pub struct AicliRuntime {
      store: Arc<dyn SecretStore>, db: Arc<Database>, api: NewApiClient,
      routes: RwLock<Arc<RoutesSnapshot>>,        // catalog + 各配置集编译结果
      credentials: RwLock<HashMap<i64, String>>,  // token_id -> 明文，仅内存，来自 Keychain
      session_pins: Mutex<LruCache<String, SessionPin>>, // 上限 4096，TTL 24h；Cargo.toml 增加 lru
      cooldown: Cooldown,
  }
  ```
  `ProxyState`（`proxy/server.rs:34`）增加 `pub aicli: Arc<AicliRuntime>`。启动时先从缓存构建快照，**代理立即可用**，再后台刷新；`fingerprint` 变化时重新编译并调用 C1-09。
- 验收：单测：网络不可用时启动，缓存路由仍可解析；刷新失败保留旧快照；未绑定时为空；旧会话持有旧 `Arc<RoutesSnapshot>` 不受新快照影响。
- 证据：

### C2-02 模型别名
- [ ] 完成
- 修改：`aicli/alias.rs`。`encode(app, public_model) -> client_id`、`decode(app, client_id) -> Option<public_model>`：
  - Claude Code：`aicli-claude-<public_model>`。原因见 `mode/stack.rs:33-44`。若 `public_model` 以 `claude-` 开头，**也按此规则编码**，以保证 MAX 窗口逻辑一致。
  - Claude Desktop：`claude-fable-aicli-<6 位数字>`，数字为 `sha256(public_model)` 派生，表持久化在 `aicli_cache`（kind=`desktop_alias`），冲突时线性探测。
  - 其他客户端：不编码，原样。
  - 非别名 id 的 `decode` 返回 `None`。四档别名（`ANTHROPIC_DEFAULT_*`）指向当前配置集第一个模型的编码值。
- 验收：单测覆盖往返、Desktop 不含厂商黑名单词（复用 `stack.rs` 的约束）、id 末尾不被当成 8 位日期。
- 证据：

### C3-01 解析器
- [ ] 完成
- 修改：`aicli/resolver.rs`：
  ```rust
  pub struct RouteDecision {
      pub public_model: String, pub upstream_model: String,
      pub profile_id: String, pub token_id: i64, pub credential: String,
      pub session_key: String,          // "<profile_id>:<session_id>"
      pub wire_protocol: String,
      pub providers: Vec<Provider>,     // 每条线路一个克隆，按尝试顺序
      pub timeouts: HashMap<String, LineTimeouts>, // provider.id -> 超时
  }
  pub enum Resolved { NotBound, Plain, Hit(Box<RouteDecision>), Miss(RouteMiss) }
  ```
  顺序：
  1. 未绑定 → `NotBound`，返回 401，错误信息提示先绑定。
  2. 从请求（Gemini 从 URL）取模型，`alias::decode`，得到 `public_model`。
  3. 配置集选择：`session pin > aicli_scope_binding(session > project > app > global) > active_profile_id > 预设 recommended`。首次命中写入 pin。
  4. 在 `RoutesSnapshot` 查 `public_model`；没有 → `Miss::ModelNotInProfile`；配置集 `needs_repair` 且该模型受影响 → `Miss::NeedsRepair`。
  5. 取凭证；取 `wire[client_protocol]`；compat 为 `unavailable` → `Miss::Unsupported`。
  6. 线路顺序：`model preferred_line > 配置集 line_policy > selected_line_id`，排除冷却中线路；全部冷却时只保留恢复最早的一条（probe-one）；其余按 `priority` 降序。
  7. 克隆种子 Provider（C3-02），按 wire 协议设置 meta，把 `body.model` 改写为 `upstream_model`。
- `RouteMiss` 的错误响应按客户端协议构造（复用 `stack_miss_body`），不同原因用不同 `code`：`model_not_in_profile`、`profile_needs_repair`、`capability_unsupported`、`not_bound`。
- 验收：表驱动单测覆盖上述每个分支，含 Gemini 从 URL 取模型；pin 后目录变更不影响该会话；probe-one。
- 证据：

### C3-02 隐藏种子 Provider
- [ ] 完成
- 修改：`aicli/seed.rs`，为支持的 app 建 `aicli-main`、`aicli-global`、`aicli-apac` 三行（复用 `database/dao/providers_seed.rs` 的写法，启动时幂等执行，并以此替换 `init_default_official_providers` 的调用）。`settings_config` 按各适配器的要求写入 base URL，不写 key：Claude/Desktop `env.ANTHROPIC_BASE_URL`（`providers/claude.rs:725-745`）；Gemini `env.GOOGLE_GEMINI_BASE_URL`（`providers/gemini.rs:168`）；Codex `base_url`（`providers/codex.rs:1025`）。
  `clone_for(app, line, wire)`：按 wire 设置 Provider meta，使现有适配器选到对应转换链。Claude 的取值见 `resolve_claude_api_format`（`forwarder.rs:3167`），Codex 的见 `forwarder.rs:217` 附近的转换选择。
- 验收：对每个 `(app, wire)` 组合写一条转发测试，对 mock 上游断言**实际命中的路径**：
  | client | wire | 上游路径 |
  |---|---|---|
  | Claude | anthropic | `/v1/messages` |
  | Claude | openai | `/v1/chat/completions` |
  | Claude | openai-response | `/v1/responses` |
  | Claude | gemini | `:generateContent` 或流式对应路径 |
  | Codex | openai-response | `/v1/responses` |
  | Codex | openai | `/v1/chat/completions` |
  | Codex | anthropic | `/v1/messages` |
  | Gemini | gemini | `:generateContent` 或流式对应路径 |
- 证据：

### C3-03 接入请求入口
- [ ] 完成
- 修改：
  - `proxy/handler_context.rs:92` `RequestContext::new` 的 `stack: Option<StackTarget>` 改为 `route: RouteInput`，`enum RouteInput { None, Stack(StackTarget), Aicli(Box<RouteDecision>) }`。`Aicli` 分支：`providers=decision.providers`，`current_provider_id=providers[0].id`，`request_model=public_model`，`app_config.auto_failover_enabled=true`，`max_retries=providers.len()-1`，超时从 `decision.timeouts` 取，`is_stack=false`。
  - `proxy/handlers.rs`：`resolve_stack_target` 的 4 个调用点，以及没有它的 `handle_codex_standalone_passthrough`（`:1356`）和 `handle_gemini`（`:2576`），统一改为调用新函数 `resolve_route`：账户已绑定 → 走解析器；否则返回 `not_bound`。
  - Gemini：沿用 `with_model_from_uri`（`handler_context.rs:231`），并把 URL 中的模型替换为 `upstream_model`。
  - `handle_models`（`handlers.rs:92`）：绑定后返回解析器的列表，Claude 发现请求用 Anthropic 形状，其他用 OpenAI 形状；`handle_claude_desktop_models` 同理。
- 验收：新增 `tests/aicli_routing.rs`，起真实代理 + mock 上游：模型改写正确；未绑定返回 401；`/v1/models` 返回别名；现有 proxy 测试不退化。
- 证据：

### C3-04 转发器开关
- [ ] 完成
- 修改：`proxy/forwarder.rs`：`RequestForwarder` 增加 `aicli_request: bool` 和 builder `aicli_request(bool)`，在 `create_forwarder` 中设置。
  - `routing_state_enabled()`（`:505`）：`!stack_request && !aicli_request`，这样绕过熔断器。
  - `keeps_resolved_model()`（`:511`）：`stack_request || aicli_request`，不再做模型映射。
  - `finish_success`（`:612`）：`aicli_request` 时跳过 `:653` 的 `try_switch` 分支，**不**累加 failover 到路由状态。
  - `record_provider_failure`（`:672`）：`aicli_request` 时只写 `AicliRuntime.cooldown`，不写 `provider_health`。
- 验收：单测：备用线路成功后，`live-state.json` 和 `current_providers` 都不变；模型不被映射；熔断器状态不变。
- 证据：

### C3-05 错误分类
- [ ] 完成
- 修改：`aicli/errors.rs` 的 `categorize(&ProxyError) -> ErrorCategory`，在 `categorize_proxy_error`（`forwarder.rs:3323`）开头：`if self.aicli_request { return aicli::errors::categorize(error); }`。规则：
  | 错误 | 分类 | 冷却 |
  |---|---|---|
  | `ForwardFailed`（连接、TLS、超时）、首字节超时、`StreamIdleTimeout`（首字节前） | Retryable | 线路级 |
  | `UpstreamError` 502/503/504/520～527/530，且 body **不是** JSON | Retryable | 线路级 |
  | `UpstreamError` 429 | NonRetryable | `线路+模型`，按 Retry-After |
  | 其他 `UpstreamError`（401/403/404/408/409/451/任意带 JSON 体的 5xx） | NonRetryable | 无 |
  | `TransformError`、`ConfigError`、`AuthError` | NonRetryable | 无 |
  | 客户端断开 | ClientAbort | 无 |
- 新错误映射：按 new-api 错误体区分并返回明确信息：`insufficient_quota`（额度不足）、`model_not_found`（模型不存在）、`invalid_token`/401（`sk-` 失效，按 C1-09 重建）。
- 验收：表驱动单测覆盖每一行；集成测试：主线路返回 HTML 502 时切到备用；返回 JSON 503、401、429 时**不切线路**，mock 上游只收到一次请求。
- 证据：

### C3-06 冷却
- [ ] 完成
- 修改：`aicli/cooldown.rs`：两级表 `line`、`line+model`。退避 `min(60s << min(n-1,10), cap)`，普通失败 cap=600s，限流 cap=1800s，30 分钟无失败清零计数。Retry-After 只在长于退避时覆盖，上限 3600s。成功时清除对应条目。
- 验收：使用注入时钟的单测，断言序列 60、120、240…及封顶；Retry-After 覆盖规则；清零规则。
- 证据：

### C3-07 每次尝试的超时
- [ ] 完成
- 修改：
  - `handler_context.rs:20` `StreamingTimeoutConfig` 增加 `total_timeout: u64`。
  - `forwarder.rs`：`forward` 中按当前 provider 从 `decision.timeouts` 取值（替换 `:2772`、`:3117` 处读取），hyper 路径等响应头的超时（`:2729`、`:2803`）在 `aicli_request` 时使用首字节超时。
  - `response_processor.rs::create_logged_passthrough_stream`（`:731`）：`total_timeout>0` 时检查总时长，超时按流中断处理。响应阶段按 `ctx.provider`（`handlers.rs:383`）取最终线路配置。
- 验收：用 mock 的 `delay_first_byte`、`idle_gap` 分别测试 Cloudflare 和 CDN 线路的超时值，测试时间可通过测试专用配置缩短；首字节前超时切线路；首字节后超时只报告中断，mock 上游只收到一次请求。
- 证据：

### C3-08 本地鉴权
- [ ] 完成
- 修改：
  - `aicli/local_auth.rs`：`token_for(app)`，读写 `aicli_local_token`，格式见第 0.4 节；`is_proxy_token(s)`：以 `aicli-local-` 开头或等于 `PROXY_MANAGED`。
  - `proxy/server.rs:423` 之前加 `axum::middleware::from_fn_with_state`：`/health`、`/status` 放行；其余从 `Authorization: Bearer`、`x-api-key`、`x-goog-api-key` 或 Gemini `?key=` 取值，与 `aicli_local_token` 全部行比对（常数时间比较）。不匹配返回 401。
  - 匹配后**删除**这些入站鉴权头，后续由 `forwarder.rs:2110` 注入真实凭证。Desktop 路径原有的网关令牌校验（`handlers.rs:445-500`）改为同一套。
  - 投影：`live/project/claude.rs:264`、`codex.rs:512`、`gemini.rs`、`grok.rs`、`mode/controller.rs:433,476` 写入值改为 `local_auth::token_for(app)`。判定"是否接管态"的比较（`codex_config.rs:316`、`live/project/codex.rs:862`）改用 `is_proxy_token`。`forwarder.rs:48,186,4272` 的防泄漏守卫也改用 `is_proxy_token`，确保本地令牌绝不发往上游。
  - 拒绝带 `Origin` 头且非本应用的请求（防浏览器跨站）：`Origin` 存在且不是 `tauri://`、`http://tauri.localhost` 时返回 403。
- 验收：集成测试：无令牌 401；错令牌 401；任一 app 的令牌可通过；跨站 `Origin` 403；mock 上游收到的请求不含本地令牌（断言三种头）；`/health` 免鉴权。
- 证据：

### C3-09 注入凭证和会话头
- [ ] 完成
- 修改：`forwarder.rs:2110` 处，`aicli_request` 时用 `decision.credential` 构造 `AuthInfo`，策略按 wire 协议取：anthropic → `Anthropic`（x-api-key），openai/openai-response → `Bearer`，gemini → `x-goog-api-key` 对应策略（`providers/auth.rs:88-107`）。凭证加入 `log_secrets`。增加请求头 `X-AICLI-Session`。
- 验收：mock 上游断言每种 wire 的鉴权头；日志捕获测试中不含凭证明文。
- 证据：

### C3-10 接管和还原
- [ ] 完成
- 修改：`aicli/takeover.rs`：
  - `take_over(app)`：先备份客户端当前配置原文到 `SecretStore`（account `takeover-backup:<app>`），写 `aicli_takeover`，再调用 `mode::controller::enter_with_route`（路由模式 + `aicli-main` 种子）。**不复用** `proxy_live_backup` 存原文。
  - `release(app)`：`mode::controller::exit`，再把备份原文原样写回，与备份逐字节比较后删除备份。
  - 启动时若检测到 cc-switch 正在接管（客户端配置的 base URL 指向 `127.0.0.1:15721`，或 token 为 `PROXY_MANAGED`），`take_over` 返回 `CcSwitchConflict`，由前端提示，用户确认后才继续。
  - Tauri 命令在 C3-11 暴露。
- 验收：对 Claude、Codex、Gemini、Grok、Desktop 分别测试：临时 HOME 下准备配置 → 接管 → 配置指向 `127.0.0.1:15821` 且 token 符合格式 → 还原 → **文件逐字节一致**。崩溃中断测试：接管写到一半（用 `live-state.json` 的 pending 机制）后 `startup` 能恢复。
- 证据：

### C3-11 Tauri 命令
- [ ] 完成
- 修改：`commands/aicli.rs`，并在 `lib.rs` 的 `generate_handler!` 注册：`aicli_get_account`、`aicli_bind(access_token, line)`、`aicli_unbind(delete_created_tokens)`、`aicli_refresh(kind)`、`aicli_get_catalog`、`aicli_get_routes`、`aicli_list_lines`（含健康状态）、`aicli_select_line(id)`、`aicli_probe_lines`、`aicli_get_profiles`、`aicli_save_profile`、`aicli_delete_profile`、`aicli_set_active_profile(scope, scope_key, id)`、`aicli_takeover_status`、`aicli_take_over(app)`、`aicli_release(app)`、`aicli_get_usage_summary`、`aicli_get_request_bill(request_id)`、`aicli_get_privacy`、`aicli_set_privacy`。返回值**不得**含访问令牌或 `sk-` 明文，用 `masked`。读取类命令遵守 C1-06，立即返回缓存。`aicli_bind` 的参数不得写入日志。
  `aicli_probe_lines`：`/api/status` 无鉴权探测，写 `aicli_line_health`，结果带 `checked_at`。失败退避：同一线路失败后 30s、60s、120s…封顶 600s 才再次探测。
- 验收：单测覆盖；序列化结果用测试断言不含 `sk-` 和测试令牌的值。
- 证据：

### C4-01 深链
- [ ] 完成
- 修改：`deeplink/parser.rs`：只接受 `aicliapi://v1/bind`，可选参数 `line`（必须是线路 id）。**带任何其他参数都拒绝，包括 `accessKey`、`apiKey`、`ticket`。** `DeepLinkImportRequest` 改成 `BindRequest`。成功时只发事件 `aicli-open-bind`，由前端打开绑定窗口。删除 `deeplink/{provider,codex_link,grok_link,mcp,prompt,skill,icon_fetch}.rs` 及相关测试、`tests/deeplink_import.rs` 中对应用例。日志用 `url_for_log`。
- 验收：测试覆盖合法、带密钥参数被拒、旧 `v1/import` 被拒；捕获日志和事件负载中不含 URL 参数值。
- 证据：

### C4-02 绑定后自动接管
- [ ] 完成
- 修改：`aicli_bind` 成功后：刷新数据 → 启动代理（若未启动）→ 编译默认配置集并取得 Token → **只接管用户已启用的 Tier 1 客户端**（首次绑定默认全部已安装的 Tier 1）→ 发事件。任一客户端接管失败不影响其他客户端，也不回滚绑定，结果逐项返回。
- 验收：集成测试：绑定成功时代理已启动，客户端配置已写入；一个客户端写入失败时其余正常，错误逐项可见。
- 证据：

### C4-03 账单回填
- [ ] 完成
- 修改：`aicli/usage.rs`：请求完成时从响应头取 `X-Oneapi-Request-Id` 写入 `upstream_request_id`。流式在头部即可取到。入队后台任务，按 1s、3s、10s 调 `log_by_request_id`，命中后写 `server_group`（`group` 字段）、`billed_quota`（`quota` 字段）、`billed_at`。三次都查不到时保持 NULL，UI 显示"等待服务端记账"；账户关闭消费日志时同样如此。队列上限 1000，不落盘。**不得用本地估算值覆盖这些字段。**
- 验收：mock 上游和 mock new-api：第 2 次查询才命中；三次都未命中；队列满时丢弃最旧并记一次警告。
- 证据：

### C4-04 用量记录补字段
- [ ] 完成
- 修改：`proxy/usage/logger.rs:100` 一带写入 `aicli_line_id`（来自最终 provider）、`aicli_profile_id`。`provider_id` 保持种子 id，这样 `usage_stats.rs:441` 的 JOIN 能显示线路名称。
- 验收：单测断言写入值；故障切换后记录的是实际成功线路。
- 证据：

### C4-05 流式首选线路（条件项）
- [-] 按负责人于 2026-11-02 的决定不启用；依据是本决定，不是 A5-01 已测出 120 秒结论。
- 原条件：仅当 A5-01 判定 120 秒是总时长上限时实施；当前 A5-01 保持 `[-]`，120 秒语义未定案，负责人已明确决定跳过本项。
- 原修改要求（本项跳过，不执行）：`resolver` 第 6 步：请求 `stream=true` 且用户未显式固定线路时，首选 `apac`。设置页说明原因。
- 原验收要求（本项跳过，未运行）：单测；显式选线优先于该规则。
- 证据：负责人 2026-11-02 明确决定 `main`、`global` 流式总时长为 `600s`（未实测），C4-05 不启用并标 `[-]`；仅记录决定，未实现该首选线路规则、未运行其单测，不声称 CI 验证了 120 秒语义。见 `docs/aicli-evidence/A5-01-owner-decision.txt`。

### C5-GATE 真实客户端验收（方案 PoC A）`[需 H-01、H-02]`
- [ ] 完成
- 环境：先在 A6 环境用 mock 跑通，再用 H-01 的令牌走真实绑定，连真实线路。测试结束后删除本次创建的 `aicli:` Token。
- 步骤：对 `Claude Code → gpt-6.1-sol`、`Claude Code → claude-*`、`Codex → gpt-6.1-sol`、`Gemini CLI → gemini-*`，按下表测试，结果存 `docs/aicli-evidence/C5-GATE.md`：
  - 选择器能选到别名（Claude Code `/model`）
  - 普通文本（流式、非流式）
  - 工具调用（含并行、工具结果回传）
  - thinking 开启时多轮对话
  - 图片输入
  - 超长上下文
  - 错误格式（模型不存在、额度不足时客户端显示正常）
  - 客户端取消
- **比较转换方式**：对 `Claude Code → gpt-6.1-sol` 同一用例集分别走客户端转换（`wire=openai-response`）和服务端转换（`wire=anthropic`，发 `/v1/messages`），记录差异。
- 定案规则：
  - 客户端转换通过项 ≥ 服务端转换 → 保持默认规则。
  - 否则在 `src-tauri/src/aicli/compat.json` 中为该模型覆盖 `wire`。
  - 每个"客户端协议 | 模型族"按结果写 compat：全部通过 `full`；文本和流式通过但工具或 thinking 有限制 `partial`；只有文本 `text_only`；否则 `unavailable`。
- 不通过的处理：`unavailable` 的组合不算门禁失败，但 Tier 1 **原生协议**组合（Claude→claude、Codex→gpt、Gemini→gemini）必须全部 `full`，否则标 `[!]`。
- 证据：

---

## E. 阶段 E：隐私引擎

全部在 `src-tauri/src/aicli/privacy/`。**接入点只有一个中间件，不改 handlers。**

### E-01 获取源码和声明
- [ ] 完成
- 修改：把 Magpie 克隆到 `source-reference/magpie` 并检出 `f11bf27c1f3f0f7d5284dffa2cb8b066debaed84`（该目录已被 gitignore）。新增 `THIRD_PARTY_NOTICES.md`，写入 Magpie 的 MIT 全文和版权行，以及 jieba 的声明（来自 Magpie 自身的 `THIRD_PARTY_NOTICES`）。`src-tauri/Cargo.toml` 增加 `data-encoding`、`unicode-normalization`。
- **禁止**：不得把 Maskit 的代码、正则、规则表、测试夹具复制或翻译进仓库。不得克隆 Maskit 到本仓库目录内（E-07 另行放置）。
- 验收：`git status` 中没有 `source-reference`；声明文件存在。
- 证据：

### E-02 占位符和密钥
- [ ] 完成
- 修改：`privacy/placeholder.rs`。格式 `{{KIND_xxxxxxxx}}`，正则 `\{\{[A-Z][A-Z0-9_]*_[a-z2-7]{8}\}\}`，HMAC-SHA256(key, `kind\0value`) 取 base32 前 8 位并小写，与 Magpie `redact.go:682-685` 完全一致。密钥 32 字节随机，存 Keychain `privacy-hmac-key`。值表：内存 `HashMap`，上限 200,000，满时淘汰最旧一半。
- 验收：
  - 新增 `tools/magpie_golden/`（Go，只在开发时运行，不进产物）：固定测试密钥，用 Magpie 代码对语料生成占位符 JSON。Rust 用相同密钥比对，**完全一致**。
  - 同值同占位符；模拟重启（新值表、同密钥）后不变；淘汰后请求再次带原值时可恢复。
- 证据：

### E-03 检测规则
- [ ] 完成
- 修改：`privacy/rules/`，把 Magpie `internal/redact/` 的下列文件逐个移植：`redact.go`、`more.go`、`shape.go`、`ids.go`、`phone.go`、`fieldvalue.go`、`dateword.go`、`known.go`、`scrub.go`、`rules.go`。文件头注明来源和 commit。
  - **不移植** `model.go`。`zhname.go`、`zhdict.txt` 放到 E-08 决定。
  - 中文身份证、Luhn 用 Magpie 现成函数（`chineseID`、`luhn`）；银行账号 mod-97 按公开算法实现。
  - Go `regexp`（RE2）迁移到 Rust `regex` crate，语义差异写进代码注释。
  - 默认启用类别：密钥类、手机、邮箱、证件、卡号、用户自定义词和自定义规则。
- 验收：
  - 移植 Magpie 对应的 `_test.go` 中不依赖模型的用例，**通过率 100%**，未移植的用例必须列出原因。
  - 用 `tools/magpie_golden` 在 ≥ 500 条语料上比较 Rust 和 Go 的输出，**差异为 0**，或每条差异都有写明原因的豁免。语料自行编写，存 `src-tauri/tests/fixtures/privacy/corpus.jsonl`，覆盖中英文和代码片段。
  - 手写 `expected.json`：每条标注必须被遮盖的值，全部被遮盖。
  - 手写误报用例：订单号、UUID、提交哈希、数字 ID、文件路径不被遮盖，通过。
- 证据：

### E-04 JSON 遍历和协议保护
- [ ] 完成
- 修改：`privacy/walker.rs`，移植 Magpie `json.go`（`keepIn`、`scan`、`signatureKeys`、`sealed`）。必须满足：
  - 只读整块：Anthropic `thinking`、`redacted_thinking`；Responses 带 `encrypted_content` 的 reasoning；Gemini 带 `thoughtSignature` 的 part。
  - 不扫描：`model`、`signature`、`encrypted_content`、`thoughtSignature`、`data`（base64）、`call_id`、`tool_use_id`、`tool_call_id`、`id`、`file_id`、`previous_response_id`、`type`、`role`。
  - 必须扫描：`tool_use.input`、`functionCall.args`、工具结果文本，即使里面出现同名字段 `signature`。
  - 不改变 JSON 键顺序，未命中规则时**字节不变**。
- 验收：四种协议各写一份真实结构的请求夹具（含工具调用、thinking、图片），断言上述每条；Codex 真实请求样本取自 Magpie `testdata/codex-0.162.1-request.json`（MIT）。无敏感内容时输出与输入逐字节相同。
- 证据：

### E-05 流式还原（方案 PoC C）
- [ ] 完成
- 修改：`privacy/stream.rs`，移植 Magpie `writer.go` 的事件级扣留和 `partialTail`（`braceTail`、`wordTail`、`shapeTail`）。按 SSE 事件切帧，增量 UTF-8 解码，兼容 CRLF。规则：
  - 事件文本尾部可能是占位符开头 → 扣留该事件；下一事件上下文（`eventCtx`）和路径相同时把尾部移过去；不同则原样放行。
  - `final`（流结束、错误、取消）时放出全部扣留内容。
  - **Anthropic `thinking_delta`、Responses reasoning 不还原**，保留占位符（只读规则的一部分）。
  - JSON 字符串内的还原值做 JSON 转义。
  - Gemini 事件没有 `type/index` 字段，必须补上按候选和 part 路径区分的上下文，并有专门测试。
  - 流式响应不是 SSE（NDJSON 或 JSON）时走整体还原。
  - 非 SSE 响应：整体 `restore`，遵守实体头处理（删除 `content-length`）。
- 验收（四种协议各一套，用属性测试 `proptest`，加到 `[dev-dependencies]`）：
  - 把一段含占位符的完整响应**在任意字节位置**切成任意多个 chunk，还原结果与一次性还原相同，且不丢、不增事件内容。
  - 占位符跨事件、裸 token、转义形式、工具参数流式增量。
  - 流中断时已扣留内容被放出；客户端取消时状态被释放（无泄漏，用计数断言）。
  - 多字节字符切在 chunk 边界。
- 证据：

### E-06 中间件接入
- [ ] 完成
- 修改：`privacy/layer.rs`，在 `proxy/server.rs` 的 router 上加 `from_fn_with_state`，位于 C3-08 本地鉴权之后：
  - **隐私关闭时直接 `next.run(req)`，不读 body**。
  - 只处理 AI 路径的 POST（`/v1/messages`、`/chat/completions`、`/responses`、Gemini `generateContent` 系列及其前缀变体、`/app/*`）。图片生成和编辑接口在隐私优先下返回 `privacy_media_unsupported`。
  - 请求：读取 body → 按 `Content-Encoding` 解压（复用 `proxy/content_encoding.rs`，并移除该头）→ `mask` → 重建请求（更新 `content-length`）→ 设置 `Accept-Encoding: identity`。
  - 含 base64 图片或文件的请求：`allow_unscanned_media=false` 且隐私优先 → 返回 `privacy_media_blocked`，错误信息说明如何放行；否则放行并计数。
  - 响应：SSE → 包装流式还原；其他 → 整体还原。上游以压缩返回流式内容 → fail-closed 返回 `privacy_compressed_stream`。
  - 引擎异常（包括 `catch_unwind` 捕获的 panic）：隐私优先 → 阻断，返回 `privacy_engine_failed`；可用性优先 → 放行原文，首页和错误日志显示降级，计数。
  - 日志只记录类别和数量。**不记录原文、占位符映射。**
  - 非流式和错误响应同样还原。`proxy_request_logs.error_message` 写入发生在中间件内层，保持脱敏状态。
- 验收：
  - `tests/aicli_privacy_e2e.rs`：起真实代理 + mock 上游，四种协议、流式和非流式。断言：`/__requests` 中不含任一敏感值；客户端收到的内容完全还原；`proxy_request_logs` 和应用日志中不含敏感值（搜索断言）。
  - 关闭隐私时，上游收到的 body 与未加中间件时**逐字节相同**（录制比较）。
  - 协议转换链路（Claude→Responses）同样通过，占位符不被转换破坏。
  - 重启后对话继续：同一历史两次发出的 body 中占位符相同（提示缓存前缀不变）。
  - 故障注入：两种模式行为符合设定。
- 证据：

### E-07 Maskit 黑盒对照（仅开发机）
- [ ] 完成
- 修改：把 Maskit 放在 `/root/maskit-reference`（仓库外），按其文档独立运行。新增 `scripts/aicli-poc/maskit_diff.py`：把 E-03 的语料分别过两个引擎，输出差异到 `docs/aicli-evidence/E-07.md`。只提交**差异统计和自写语料的结论**，不提交 Maskit 的规则或代码。
- 验收：每一类差异都有结论：我方漏遮（修复或说明不修的原因）、对方误报、口径不同。我方漏遮的密钥类、证件类条目必须修复后才能通过。运行不起来时标 `[!]`，不阻塞 E-GATE，但要在第 12 节说明。
- 证据：

### E-08 中文姓名（可选开关）
- [ ] 完成
- 修改：移植 `zhname.go`、`zhdict.txt`（Magpie 注释说明它是在开启模型时启用的，这里单独作为开关），默认关闭。设置项 `privacy_detect_cjk_names`。
- 验收：用 ≥ 200 条自写中文语料评估：精确率 ≥ 95%，召回率写入证据。**精确率不达标时，功能不开放**：保留代码但隐藏开关，把结果写入第 12 节。
- 证据：

### E-GATE 阶段 E 门禁
- [ ] 完成
- 验收：E-01～E-06 全部 `[x]`；`scripts/aicli-check.sh` 通过；用真实 Claude Code（H-02）在隐私开启状态下跑一轮工具调用 + thinking 的多轮对话，能正常完成，mock 或测试账号日志中查不到植入的假密钥。
- 证据：

---

## F. 阶段 F：前端

共同要求：每项都要补 vitest；所有文案写入 5 种语言文件，不留硬编码；组件遵循现有样式。额度显示按线上 `/api/status` 的 `quota_per_unit`、`quota_display_type` 换算，与网页端一致。

### F-01 API 层
- [ ] 完成
- 修改：`src/lib/api/aicli.ts`（对应 C3-11 全部命令）、`src/hooks/useAicliAccount.ts`、`useAicliLines.ts`、`useAicliUsage.ts`、`useAicliProfiles.ts`（react-query）。类型放 `src/types/aicli.ts`。
- 验收：`pnpm typecheck`；hooks 单测（msw 或 mock invoke）。
- 证据：

### F-02 导航和冷启动
- [ ] 完成
- 修改：`src/lib/navigation.ts`：
  - `AppPage` 的 `"providers"` 改为 `"client"`，`appPageBelongsTo` 同步。
  - `GlobalPage` 删除 `"auth"`。
  - `readStoredView()` 恒返回 `"usage"`，删除 `storeView` 和 `App.tsx:210` 的持久化。
  - `LEGACY_VIEWS` 增加 `providers → usage`、`auth → usage`。
- 验收：测试：localStorage 中存有任意旧值时冷启动都是 `usage`；运行期切页正常。
- 证据：

### F-03 首页
- [ ] 完成
- 修改：`src/components/aicli/AicliHome.tsx`，作为 `usage` 页。自上而下：
  1. **未绑定**：`AicliBindCard`：按钮"打开网页获取令牌"（系统浏览器打开 `https://aicliapi.com/profile`），密码型输入框（调用 `aicli_bind`，提交后立刻清空），标题、占位、说明、警告都用第 0.4 节的文案并常驻显示。深链事件 `aicli-open-bind` 打开同一界面。
  2. **账户**：账户名、用户组、剩余额度、已用额度，标注"来源：服务端"和最后同步时间。
  3. **线路**：`AicliLineSelector`：当前线路、延迟、类型、切换。标注切换只影响后续请求。
  4. **配置集**：当前配置集、可用模型数、需要修复时的警告、入口。
  5. **本地统计**：现有 `UsagePage`/`UsageDashboard` 不改动，标题标注"来源：本机记录"。
- 状态规则：**有缓存时立即渲染，不显示整页加载态**，后台刷新完成后原地更新。同步失败时**显示缓存和时间**，徽标"缓存数据"；令牌失效时显示"令牌已失效，可能已在其他地方重新生成，请粘贴新令牌"，缓存仍可看；没有缓存时显示"暂无数据"，**不得显示 0**。隐私开启时显示引擎状态；可用性优先降级时显示红色提示。
- 验收：组件测试分别覆盖：未绑定（断言绑定界面文案在中文下与第 0.4 节逐字一致，且另外 4 种语言都有翻译；粘贴 `sk-` 开头的值时显示对应错误）；已绑定；有缓存时首屏不等待接口；同步失败有缓存；同步失败无缓存（断言页面不含 `0` 额度）；令牌失效；需要修复；隐私降级。
- 证据：

### F-04 客户端页
- [ ] 完成
- 修改：`src/components/aicli/AicliClientPage.tsx`，替换 `renderProviderList`（`App.tsx:1502`）。显示接管状态（已接管/未接管/冲突）、接管和停止接管按钮（确认对话框说明会还原配置）、当前配置集、本客户端可用模型表（模型名、配置集、线路、兼容级别、能力标签）、"刷新模型目录"按钮。OpenClaw、Hermes 的专属页保留。
- 验收：组件测试覆盖各状态；Claude Code 的兼容级别 `partial` 时有提示。
- 证据：

### F-05 配置集编辑
- [ ] 完成
- 修改：`src/components/aicli/profiles/`：
  - 列表：系统预设（只读，"从预设新建""查看差异"）、用户配置集（编辑、删除、设为默认）。
  - 编辑器：勾选和拖动分组（用仓库已有的拖拽依赖）→ 预览合并后的模型表 → 多来源模型列为冲突，展示来源分组、倍率提示、协议、能力 → 用户为每项选来源、排除或起别名，或点"按分组顺序全部确认"。未解决的冲突时保存按钮不可用。
  - 作用域：全局、某客户端（项目、会话级只通过代码接口设置，首版不做 UI）。
  - `needs_repair` 时显示缺失分组和受影响的模型。
  - 普通用户路径：不进入此页也能用（默认用推荐预设）。
- 验收：组件测试：未解决冲突不能保存；一键确认后可保存；校验错误都有对应文案。配置集只存本机，界面要说明不跨设备同步。
- 证据：

### F-06 设置和请求详情
- [ ] 完成
- 修改：
  - 设置 `routing`：线路说明、隐私（开关、失败模式、允许未扫描媒体、自定义词和规则）、secret 后端降级提示。删除与供应商相关的分组。
  - `RequestDetailPanel.tsx`：增加线路、配置集、服务端分组、服务端实扣额度；`billed_at` 为空时显示"等待服务端记账"，**不显示估算值冒充实扣**。估算值保留时必须标"预估"。
- 验收：组件测试断言"预估"和"实扣"标签；未记账状态。
- 证据：

### F-GATE 阶段 F 门禁
- [ ] 完成
- 验收：`pnpm typecheck`、`pnpm test:unit` 通过；`[HUMAN]` 或 `xvfb` 下走通：冷启动进入用量页 → 绑定 → 切线路 → 编辑配置集 → 查看客户端页。截图存 `docs/aicli-evidence/F-GATE/`，图中不含 Token。
- 证据：

---

## G. 阶段 G：移除 Provider 与官方账号

每一项删除后都要跑 `scripts/aicli-check.sh`。证据中列出删除的文件数和测试数。

### G-01 移除 Provider UI
- [ ] 完成
- 修改：
  - `src/App.tsx`：删除 `AddProviderDialog`、`EditProviderDialog` 相关状态和对话框（`:82,87,1821,1829`），删除复制逻辑（`:1067-1192`）、`useProviderActions` 及 provider 事件监听（`:444`）。
  - 删除 `src/components/providers/`（**先把** `mode/` 中仍被 F-04 引用的组件移到 `src/components/aicli/mode/`）、`src/components/universal/`、`src/hooks/useProviderActions.ts`、`src/lib/api/providers.ts`（保留类型的先移动）、`src/components/DeepLinkImportDialog.tsx`、`src/components/deeplink/`、`src/config/*ProviderPresets.ts`、`src/components/UsageScriptModal.tsx`、`src/components/ProviderIcon*`（若无其他引用）。
- 验收：`grep -rln 'AddProviderDialog\|EditProviderDialog\|useProviderActions\|ProviderPresets' src tests` 无结果；检查通过。
- 证据：

### G-02 移除官方账号前端
- [ ] 完成
- 修改：删除 `Claude/Codex/Copilot/Kimi/Xai/Agy/Subscription AccountQuota*`、`CodexOauth*`、`UsageFooter`、`src/components/quota/` 及授权中心页、Sidebar 中 `page:"auth"` 项（`Sidebar.tsx:281`）。
- 验收：`grep -rln 'Copilot\|CodexOauth\|XaiOauth\|KimiAccount\|AgyAccount' src` 无结果；检查通过。
- 证据：

### G-03 移除 Rust 命令和服务
- [ ] 完成
- 修改：按以下顺序**逐个**删除，每删一组就编译一次。遇到编译错误时，只删除依赖该模块的调用分支；如果该模块还被非账号逻辑使用，保留被用到的部分并写入第 13 节。
  1. `commands/`：`copilot.rs`、`codex_oauth.rs`、`xai_oauth.rs`、`kimi_oauth.rs`、`claude_account.rs`、`agy_account.rs`、`auth.rs`、`subscription.rs`、`coding_plan.rs`、`balance.rs`，以及 `lib.rs` 的注册。
  2. `services/`：`claude_account/`、`agy_account/`、`kimi_oauth/`、`dsh_account.rs`、`codex_oauth_models.rs`、`subscription*.rs`、`coding_plan.rs`、`balance.rs`。
  3. `proxy/providers/`：`copilot_auth.rs`、`codex_oauth_auth.rs`（含 `browser.rs`）、`xai_oauth_auth.rs`、`copilot_model_map.rs`；`proxy/copilot_optimizer.rs`；`forwarder.rs:2112-2247` 的 OAuth 分支，以及 `forwarder.rs:223-280` 的 Copilot 上下文。
  4. `database/dao/providers_seed.rs` 的官方种子和 `init_default_official_providers`（C3-02 已替换）。
  5. `commands/provider.rs`：只保留 C3-02 和接管所需的内部函数，删除所有对外增删改命令。`services/provider/` 中直连相关（`claude_official.rs`、`codex_login.rs`、`codex_official_models.rs`、`gemini_auth.rs` 等）在没有引用后删除。
  6. Provider 导入导出、`usage_script.rs`（无引用后）。
- **不得删除**：`proxy/providers/transform*`、`streaming*`、`gemini_*`、`inline_think.rs`、`reasoning_bridge.rs`、`responses_late_arguments.rs`，以及 `session_manager`、`mcp`、`skill`、`prompt`、各 `*_config.rs` 配置写入器。
- 验收：
  - `grep -rln 'CopilotAuth\|CodexOAuth\|XaiOAuth\|kimi_oauth\|claude_account' src-tauri/src` 无结果，例外写入第 13 节。
  - `cargo nextest` 通过；`transform*`、`streaming*` 相关测试数量**不减少**（前后对比，写入证据）。
  - `src-tauri/tests/provider_*.rs` 删除或改写，写明原因。
- 证据：

### G-04 设置和托盘清理
- [ ] 完成
- 修改：托盘（`tray.rs`、`tray_panel.rs`）去掉供应商切换菜单，改为：打开主窗口、当前线路（可切换）、退出。`settings.rs` 去掉不再使用的字段。`Settings` 的 WebDAV、S3 同步：**移除**（`commands/webdav_sync.rs`、`s3_sync.rs`、`services/s3*.rs`），因为这些会把数据库同步到外部。
- 验收：检查通过；`grep -n 'providers' src-tauri/src/tray.rs` 只剩内部调用或无结果。
- 证据：

### G-GATE 阶段 G 门禁
- [ ] 完成
- 验收：
  1. 应用内找不到任何增加、编辑、删除供应商的入口，也没有官方登录入口（`[HUMAN]` 或 UI 自动化截图）。
  2. 主链路 `tests/aicli_routing.rs`、`aicli_privacy_e2e.rs` 依然通过。
  3. `grep -rn 'sponsor\|赞助' src src-tauri/src` 及预设的第三方中转名称无结果（名单来自删除前的 `src/config/*ProviderPresets.ts`，存入证据）。
- 证据：

---

## J. 阶段 J：Tier 2 客户端

### J-01 通用入口
- [ ] 完成
- 修改：`proxy/server.rs` 增加 `/app/:client/v1/messages`、`/app/:client/v1/chat/completions`、`/app/:client/v1/responses`、`/app/:client/v1/models`、`/app/:client/v1beta/*path`。路径决定协议；`client` 必须是 `opencode|openclaw|hermes|pi|mcode|kimi|dsh|omp|agy`，否则 404，只用于统计 `app_type`。对 `get_adapter` 返回 None 的 app（`providers/mod.rs:279-284`），AICLI 路径按**协议**选适配器：anthropic → `ClaudeAdapter`，openai/responses → `CodexAdapter`，gemini → `GeminiAdapter`。
- 验收：对每个 client 和每种协议各一条测试，使用种子 Provider，确认命中和统计；未知 client 返回 404。
- 证据：

### J-02～J-10 逐个客户端
每个客户端都按同样步骤做。先读其配置写入器，用种子 Provider 通过 `services/provider/live.rs::write_live_snapshot`（或其专用入口）写入，`base_url` 指向 `http://127.0.0.1:15821/app/<client>/...`，key 使用本地令牌，**绝不写入真实 Token**。接管和还原走 C3-10 的同一套（逐字节一致）。

每项的验收：
1. 配置写入后的文件中不含 `sk-` 和访问令牌。
2. 还原后逐字节一致。
3. 安装该客户端的真实版本，发一个文本请求和一个工具调用请求，经代理成功，`proxy_request_logs` 中 `app_type` 正确。
4. 真实客户端无法安装或无法运行 → 标 `[!]`，第 12 节说明。不通过的客户端在 UI 中显示"暂不支持"，不提供接管按钮（`aicli/tier.rs` 中标记）。**不得**退回到写入真实凭证。

| 编号 | 客户端 | 写入器 | 状态 | 证据 |
|---|---|---|---|---|
| J-02 | OpenCode | `opencode_config.rs` | [ ] | |
| J-03 | OpenClaw | `openclaw_config.rs::set_provider` | [ ] | |
| J-04 | Hermes | `hermes_config.rs::set_provider` | [ ] | |
| J-05 | Pi | `pi_config/` + `services/provider/pi.rs` | [ ] | |
| J-06 | Mcode | `mcode_config.rs` | [ ] | |
| J-07 | Kimi | `kimi_config.rs` | [ ] | |
| J-08 | DSH | `dsh_config.rs` | [ ] | |
| J-09 | OMP | `omp_config.rs` + `services/provider/omp.rs` | [ ] | |
| J-10 | Agy | `agy_config.rs` + `services/provider/agy_direct.rs` | [ ] | |

### J-GATE 阶段 J 门禁
- [ ] 完成
- 验收：9 个客户端都有明确结论（支持或暂不支持），UI 显示与结论一致；暂不支持的名单写入 README。支持率不设下限，但结论必须来自真实测试。
- 证据：

---

## K. 阶段 K：最终回归（模型可完成部分）

### K-01 全量检查
- [ ] 完成
- 修改：新增 `tests/aicli-testenv/e2e.sh`，走完：绑定 → 目录 → 配置集 → 取得 Token → 发请求 → 查账 → 解绑。
- 验收：`scripts/aicli-check.sh` 通过；`e2e.sh` 通过；对测试环境 MySQL 开启通用查询日志，确认客户端全程没有连接数据库（只有 new-api 自己的连接）。
- 证据：

### K-02 密钥扫描
- [ ] 完成
- 修改：新增 `scripts/aicli-secret-scan.sh`：在运行 e2e 的临时 HOME 下，用植入的金丝雀值（假 `sk-canary...`、假访问令牌、隐私假密钥）搜索 `~/.aicli-client`（SQLite 用 `strings`）、应用日志、崩溃日志、所有客户端配置。同时用 `gitleaks` 或 `trufflehog` 扫描两个仓库的新增提交。
- 验收：`sk-` 和访问令牌金丝雀在配置目录中**只出现在** `secrets.json`（回退模式）或 Keychain；客户端配置和日志中出现次数为 0。扫描结果无新增发现。
- 证据：

### K-03 隔离检查
- [ ] 完成
- 验收：运行全流程前后，对临时 HOME 下预先放置的 `~/.cc-switch` 目录做校验和，**完全相同**。`grep -rn 'farion1231\|ccswitch\|dl.ccswitch' src-tauri/tauri.conf.json src-tauri/Info.plist src-tauri/wix package.json` 无结果。安装包内容（`pnpm tauri build` 的 Linux 产物）不含 Maskit、`source-reference`、`tools/magpie_golden`。
- 证据：

### K-04 整理偏差和阻塞
- [ ] 完成
- 修改：逐条核对第 12、13 节，把未解决项列入"交付摘要"。
- 验收：所有 `[!]` 都能在第 12 节找到；没有无证据的 `[x]`（用 `grep -n '证据：$'` 检查已勾选项）。
- 证据：

---

## L. `[HUMAN]` 阶段 L：上线前（负责人执行，模型只准备材料）

- [ ] **L-01** 换正式图标（H-05）。
- [ ] **L-02** 配置更新器并恢复 `createUpdaterArtifacts`（H-06）。
- [ ] **L-03** macOS、Windows、Linux 安装包：安装、深链注册、卸载、升级（H-03）。
- [ ] **L-04** 三条线路分别验证：TLS、`/api/status`、绑定、`/api/pricing`、`/v1/models`、四种协议、SSE 长连接；Cloudflare 线路的中断行为、CDN 长流式。
- [ ] **L-05** 后台设置（不部署任何东西）：按 H-04 确认 `auto` 和 `MaxTokenAutoGroups`；按 H-08 设置 `RetryTimes`；可选新增 A6-05 同款亲和规则；确认 `LogConsumeEnabled=true`。
- [ ] **L-06** 对测试账号走完整链路，核对服务端实扣和客户端显示一致。
- [ ] **L-07** 法务确认（H-07）。
- [ ] **L-08** new-api 升级时重跑 A6 和 `e2e.sh`，确认用到的接口没有变化。

---

## 12. 阻塞记录

| 项目编号 | 现象 | 已尝试 | 需要谁决定什么 |
|---|---|---|---|
| A5-01 / C4-05 | 负责人明确决定不再测 180 秒持续输出语义；120 秒语义未实测定案。 | 保留历史探测和失败结果；负责人于 2026-11-02 另行决定 `main`、`global` 流式总时长为 `600s`（未实测），并决定不启用 C4-05。第 0.4 节已更新，两项均为 `[-]`。 | 当前采用值和是否启用 C4-05 均已由负责人决定，无待决事项；不得把采用值或跳过当成 120 秒实测结论。 |
| A5-02 | 初次核对时账号可用分组不含 `auto`。 | 负责人告知已开启 `auto`；随后通过只读接口复测，五个接口均 HTTP 200，`auto_available=true`、`auto_max_count=15`、最大模型分组数 3；未写入任何 Token。 | 已解决；A5-02 通过，H-04 条件已满足。 |
| A6-03 | 初次路由验收时，隔离环境创建的 alice 用户无账户额度，模型请求返回 HTTP 403 `insufficient_user_quota`。 | 隔离环境通过管理 HTTP API 的 `/api/user/manage` `add_quota` 操作给 alice 注入测试额度；在 CI 重建环境并通过现有路由用例，未触及线上账号或数据库。 | 额度问题已解除；实现提交 `c66defc2` 的 CI run 41317794458 成功，但不代表下行列出的旧测试验收缺口已解决。 |
| A6-03（验收缺口） | 旧测试使用登录短期凭证且未比较 T1/T2 quota，旧 CI 不足以证明完整验收。 | A6-01 修正通过两次 CI 后，独立实现 `53567f1c` 的 run 41383467961 完整成功；真实系统访问令牌、request_id 精确对应、相同 192/3 token 用量及 quota 98/195（比例 1:1.989796）全部通过，T3 命中 g-pro。 | 已关闭；本项已以自己的新提交和 CI 重新验收并勾选，未沿用 c66defc2。 |
| A6-01（前置环境） / A6-03 | run 41375253825 attempt 1、2 曾在 A6-01 失败：socket 健康检查提前通过，new-api 连接 TCP 3306 被拒绝。 | 经负责人授权，健康检查改为指定的显式 TCP 命令；最终修正版本 `75f6e4d7` 的 run 41379889841 attempt 1、2 均完整成功。两次均核对 MySQL port 3306 就绪早于 new-api StartedAt，new-api 无 connection refused；日志过滤后仅输出到 job 日志并完成保密抽查，未上传或保存容器日志。证据仅记时间点与链接，旧失败记录保留。 | 已关闭环境阻塞；未增加 sleep、自动重启或重试，未放宽断言。A6-03 已另用独立提交 53567f1c 和 CI run 41383467961 通过重新验收。 |
| A6-04 | 原验收与系统访问令牌接口禁令的适用范围存在冲突。 | 负责人于 2026-11-02 明确本地测试授权范围；实现 `cf9147a5` 的 CI run 41369181694 成功。POST `/api/user/token` 和 DELETE `/api/user/self` 均 HTTP 403、`success:false`、message=`安全验证状态无效`、code=`SECURITY_PROOF_INVALID`；调用后原令牌仍可访问账户，管理 API 确认用户仍存在。完整无密钥记录见 `docs/aicli-evidence/A6-04.txt`。 | 已解除并验收通过；未出现接口意外成功，无需负责人再决定。 |

## 13. 偏差记录

| 项目编号 | 清单写的 | 实际做的 | 原因 |
|---|---|---|---|
| A5-01 | 第 2 节只允许方案 3.3 所列账户接口，未包括方案 3.2 中读取现有 Token 所需的 GET /api/token/ 和 POST /api/token/batch/keys。 | 经负责人明确授权，将这两个接口补充到第 2 节，仅允许读取测试账号已有 Token；完整流式探测已使用它们读取现有 Token，未创建、修改或删除 Token，未生成或撤销系统访问令牌。 | 系统访问令牌不能调用 /v1 模型接口；A5-01 流式探测需要安全读取已有 sk- Token，不能把系统访问令牌误用为模型密钥。 |
| A5-01 | 本机线上探测，不在 CI 运行；原 TLS 路径在握手阶段被重置。 | 仓库外安装自写按进程启用的 `aicli-direct`，仅对 `api.aicli.qzz.io`、`aicli.qzz.io` 的首个 ClientHello 拆 TLS record，保留原握手字节、SNI、证书及主机名校验，清除该子进程代理环境变量。隔离 HOME 复验 60/60 及拒绝对照均通过；工具和源码未加入仓库、工作流或产品。 |
| A5-01 / C4-05 | 清单要求通过超过 180 秒持续输出来判定 120 秒语义，再决定流式总时长和 C4-05。 | 负责人于 2026-11-01 决定跳过该测量；于 2026-11-02 另行决定 `main`、`global` 流式总时长为 `600s`（未实测），C4-05 不启用。第 0.4 节已更新；A5-01 与 C4-05 均为 `[-]`，历史测量证据保留。 | 按负责人明确决定采用参数和跳过条件项，不将决定伪装成实测，不声称得出 120 秒语义结论。 |
| A5-02 | 清单验收要求按 `/api/pricing` 的 `enable_groups` 与账号可用分组取交集，并在 `auto` 缺失或上限不足时交 H-04。 | 实际响应以 `data` 字段承载分组，报告统计 69 个模型；复测 `auto_available=true`、`max_count=15`、最大交集分组数为 3。 | 只记录实际响应形状，不改变接口或数据结构；复测结果通过验收。 |
| A6-01 | 负责人要求健康检查和成功日志取证放在同一修正提交。 | 健康检查 `21aae9ea` 已推送后才补充授权取证，因此保留已公开历史并追加同编号修正 `75f6e4d7`；最终版本同时包含二者，连续两次验收均只使用最终 SHA。旧 21aae9ea 的未完运行取消，未计作通过。日志取证为独立 always / continue-on-error 步骤，清理移到其后，原启动、等待和验收代码保持一致。 | 遵守禁止强推和已公开提交不得改写的规则；本次仅修复获授权的测试环境健康检查并补充取证。 |

## 14. 交付摘要（全部完成后填写，交给验收人）

```text
AICLI-Client 分支和最新提交：feat/aicli-client-v1；阶段性进度记录，未完成全量改造。最新提交由 git log -1 --format=%H 读取；最近完成项 A6-03 独立实现为 53567f1c4468a71efe93fe35d67914da8015c4ab（Actions run 41383467961，完整 CI success）；A6-01 修正 75f6e4d7 的同一 SHA 两次完整通过记录保留。
完成/阻塞/跳过：8 / 0 / 2（完成含 A1-01、A2-01、A3-01、A5-02、A6-01、A6-02、A6-03、A6-04；A6-01 启动竞争与 A6-03 旧验收缺口均已关闭。A5-01、C4-05 仍按负责人决定跳过；A6-05 及之后尚未完成，不冒充已验证或已跳过）。
scripts/aicli-check.sh 结果（Rust 通过数 / 前端通过数，对比 A3-01）：A6-01 最终修正 75f6e4d7 两次完整 CI success，均为 Rust 4443 passed / 原有 10 skipped，前端 266 文件 / 3415 passed；A6-03 独立实现 53567f1c 完整 CI 也为相同数量；未减少测试或增加 ignore。
tests/aicli-testenv/e2e.sh 结果：未创建、未运行；尚未进入 K-01。
A5-02 的 auto、max_count 结论：五个只读接口 HTTP 200；`auto` 在可用分组中，`max_count=15`，单模型最大可用分组数为 3；H-04 条件满足。
线上测试账号实际创建的 aicli: Token 数量：0；线上仅通过获授权的列表和 batch/keys 接口读取已有 Token，未创建、修改或删除 Token，未生成或撤销系统访问令牌。A6-04 在隔离容器中新建 3 个一次性用户和各自的本地系统访问令牌；其中一个用户创建并删除 1 个 aicli:test 模型 Token，未使用 alice、bob 的凭证进行敏感操作。
未完成项（[!] 和 [-] 全部列出）：[-] A5-01，负责人决定跳过 180 秒语义测量，120 秒语义未实测定案；[-] C4-05，负责人于 2026-11-02 决定不启用，依据是本决定。A6-03 已完成独立重新验收；A6-05 及之后的任务除 C4-05 已明确跳过外尚未开始。A6-04 已完成；两个敏感调用均 403、success:false、message=安全验证状态无效，原令牌与用户状态正常；batch/keys 不带 sk- 前缀，删除模型 Token 后首个检查即 401（0.001169s）。
Tier 2 支持结论：未测试。
compat 表结论：未测试、未修改。
A5-01 的 120 秒结论及采用的超时：120 秒语义未实测定案；第 0.4 节 main、global 的流式总时长均采用负责人 2026-11-02 决定的 600s（未实测）。C4-05 按同一决定不启用，标 [-]；A5-01 仍为 [-]，两项决定都不是测量结论。
A6-05 的流中重试结论：未测试；A6-03 已完整通过，下一项为 A6-05。
需要负责人提供的事项：A6-04 无待决事项，已通过 CI 验收；A6-01 前置环境修正已通过同一 SHA 连续两次完整 CI，无待决事项；A6-03 已按负责人要求独立重新验收通过，无待决事项，未操作线上凭证或额度。A5-02/H-04 无待决事项，180 秒语义测量按负责人决定跳过。线上测试令牌仅用于获授权的本机子进程；A6-04 仅在本地容器中新建一次性用户，不读取线上测试令牌。无密钥进入代码、日志、证据、提交或 Actions secrets。H-02～H-08 仍按各自依赖阶段处理，未代办任何 HUMAN 项。
```

验收时，我会按此摘要和各项证据抽查重跑，并核对代码是否符合本清单和方案。
