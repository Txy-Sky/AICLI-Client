# AICLI Client 改造方案 v2.2（调研后重写）

> 编写日期：2026-10-31。本文是当前的实施依据。第一版方案已删除，需要时可在提交 `8ff0ac25` 中查看。
>
> **v2.2 修订**：线上是未修改的公开版 new-api rc.34。客户端直接用网页端的系统访问令牌完成绑定，并直接读取线上接口，本地只做缓存。**不改 new-api，不新增服务端，不写数据库。** 执行步骤见 `docs/aicli-client-implementation-checklist-zh.md`。

## 背景

第一版方案是逐步演进出来的，前后章节互相冲突，而且写作时没有逐条对照源码。这次调研了 cc-switch（本地代码）、new-api `v1.0.0-rc.34`（按 tag 读）、Maskit（`xiaYuTian11/maskit`），以及后加入的参考项目 Magpie（`yetone/magpie@f11bf27`），目标是重新回答"怎么结合最好"。

产品目标不变：做 AICLI 中转站专用客户端，用户只需绑定账户、选模型、选线路，复杂度留在后台。

**已确认的决策**
- 隐私引擎用 Rust 原生实现，以 Magpie（MIT）为基础，Maskit 只作开发期黑盒对照。
- 移除官方账号管理。接管客户端前先备份配置，用户可对单个客户端"停止接管"并还原。
- 不修改 new-api，不新增服务端。客户端用系统访问令牌直接调用 new-api 现有接口，线上数据为准，本地缓存只用来加快显示。

**证据等级**：以下结论来自读源码，没有运行过任何一个项目。标【推断】的是推演，标【实测】的必须在阶段 0 验证后才能定案。

---

## 一、各项目优点与取舍

| 项目 | 真正的优点 | 采用 | 不采用 |
|---|---|---|---|
| **cc-switch**（MIT） | 协议转换约 3.86 万行（5 条转换链、4 条透传），proxy 有 1,689 个测试；14 个客户端的配置写入器；技能、MCP、会话、用量、托盘、更新器 | 整条代理链路、转换器、配置写入器、模型发布链路、UsageDashboard | Provider CRUD 和预设（前端 91 个文件、3.6 万行）、官方 OAuth、Stack 的"单家不转移"语义 |
| **new-api**（AGPL） | 按真实分组结算和细粒度日志、`tiered_expr` 计费、渠道优先级和自动禁用、会话亲和、token 级权限约束 | 分组选择（Token 级 `AutoGroups`）、渠道级重试和亲和、计费事实（`/api/log/self`）、账户和 Token 的现有接口 | `pricing_version`（硬编码常量）、修改其源码或数据库 |
| **Maskit**（AGPL） | 协议边界处理细（thinking、加密字段、关联 ID）、失败可见、80 多个回归测试文件、按通道的流式还原、中文证件校验思路 | 行为规格和黑盒对照基准 | 代码本身：13,625 行单文件、全局状态、强绑 mitmproxy、发布包 129～244MB、随机 token 重启后会变，且读码看 Gemini 流式有缺口【推断】 |
| **Magpie**（MIT，参考） | 模型、组、候选的分层；两级冷却和退避常量；会话亲和判定；`Requested/Served` 用量字段；可移植的脱敏模块（HMAC 确定性占位符、事件级流式扣留） | 移植冷却算法和脱敏模块，保留 MIT 声明 | Provider UI、多账号池、全部 8 种路由策略 |

**一句话结合**：cc-switch 是客户端和代理的主体，new-api 掌握分组、计费和权限事实，并直接提供账户和目录接口，Magpie 提供路由和脱敏算法，Maskit 提供验收基准。

---

## 二、总架构与三条职责边界

```text
Claude Code / Codex / Gemini / 其他客户端（配置只含本地地址和本地令牌）
   │
   ▼ 本地代理 127.0.0.1（校验本地令牌）
   ├─ 1 解析请求（必要时解压）
   ├─ 2 隐私 mask（开启时）          ← 所有转换和日志之前
   ├─ 3 AicliRouteResolver：模型 → RouteDecision（配置集、线路候选、wire 协议）
   ├─ 4 cc-switch 协议转换（需要时）
   ├─ 5 按模型注入对应的 sk- Token + X-AICLI-Session
   ▼ 三条固定线路之一（同一后端）
   未修改的 new-api：按 Token 的 AutoGroups → 亲和 → 选渠道 → 结算
   （账户、目录、用量：客户端用系统访问令牌直接调用 /api/*）
   ▲
   ├─ 6 cc-switch 响应转换
   ├─ 7 记录用量（只看到脱敏文本）
   ├─ 8 隐私 restore（最终输出流）
   └─ 9 异步按 X-Oneapi-Request-Id 回填服务端账单
```

### 边界 1：重试只分两层，互不重叠

三条线路连接的是**同一个后端**，这是整个故障设计的出发点。

| 故障 | 谁处理 | 动作 |
|---|---|---|
| 连接、TLS、超时、CDN 5xx（首字节前） | 客户端 | 换线路，记线路级冷却 |
| 分组或渠道不可用 | 服务端 | 配置集内跨组、跨渠道重试 |
| 401/403、额度不足、模型不存在、能力不支持 | 客户端 | **不换线路**（换了也一样），透传并给出区分明确的错误 |
| 429 | 客户端 | 记 `线路+模型` 级冷却，按 Retry-After 处理，不换线路 |
| 已向客户端输出正文后断开 | 客户端 | 报告中断，不重放 |

现状 `forwarder.rs:3354-3358` 把 401、404、429 都算 Retryable，AICLI 路径必须换一套分类。原方案的六段式熔断键 `profile:model:channel:credential:line:wire` 不再需要，因为渠道和分组故障由服务端管。客户端只保留 Magpie 的两级结构：`line`、`line+model`。

冷却常量沿用 Magpie 的起点：退避 `min(1m << min(n-1,10), cap)`，普通失败上限 10m，限流上限 30m，无额度 15m，Retry-After 只在长于退避时覆盖。

### 边界 2：协议只转换一次

new-api 和 cc-switch 都能做格式转换，两边都转会叠加损失。规则：

1. 模型支持客户端原生协议，就透传。
2. 否则由 cc-switch 转成模型的原生端点，new-api 再透传给渠道。
3. 目录按 `client_protocol × model` 下发 `wire_protocol`，客户端不猜。

默认选客户端转换，理由是它有更多针对真实客户端的修复和测试，且可以离线回归。这是假设，两边转换质量没有实际比较过，由 PoC A 定案【实测】。

### 边界 3：事实归服务端，表现归客户端

- 服务端决定：真实分组、倍率、权限、最终账单。客户端不比价，也不猜倍率。
- 客户端决定：线路、协议转换、模型别名、隐私。
- 粘性分两层：客户端按会话固定配置集和线路，服务端亲和固定渠道。

---

## 三、服务端（不改代码，只用现有接口）

### 3.1 两种凭证，用途不同

| 凭证 | 从哪来 | 用来做什么 | 不能做什么 |
|---|---|---|---|
| 系统访问令牌（access token） | 用户在网页个人设置中生成，粘贴到客户端 | 账户、分组、目录、日志、管理 Token（`/api/*`） | 发模型请求：`/v1/*` 只查 `tokens` 表，访问令牌会被拒绝（`middleware/auth.go` 的 `TokenAuth`） |
| `sk-` Token | 用现有接口取得 | 发模型请求（`/v1/*`） | 调账户接口 |

网页登录用的 Bearer 只有 15 分钟，刷新 Cookie 也限定了路径，桌面端无法长期使用，所以绑定用系统访问令牌。

### 3.2 为什么还需要 `sk-` Token

new-api 把分组路由放在 Token 上：一个 `sk-` Token 只有一个 `Group` 或一份有序 `AutoGroups`。线上模型按分组明显成簇（如 `gpt/gpt-max/gpt-pro`、`claude-aws/claude-code/claude-kiro`、`gemini-cli`、`xAI`），所以只需要少量 Token：

```text
配置集 + 模型 → 该模型可用的分组顺序 → 同样顺序共用一个 Token
```

**不写数据库。** 取得 Token 的顺序：

1. 先用 `GET /api/token/` 找现成的：分组顺序相同、已启用、未过期、无模型限制、无限额度的 Token（包括用户自己建的）。
2. 找不到才调用 `POST /api/token/` 创建，名称 `aicli:<hash 前 8 位>`。这和用户在 `/keys` 页点"创建"是同一个接口，在 `/keys` 页可见可删。
3. 用 `POST /api/token/batch/keys` 取明文，存 Keychain。这个接口有 Critical 限流（默认每 IP 20 次/20 分钟），批量取一次只算一次，取到后不再重复请求。

### 3.3 客户端直接读线上

| 数据 | 接口 |
|---|---|
| 账户、额度 | `GET /api/user/self` |
| 可用分组、用户专属倍率 | `GET /api/user/self/groups` |
| 模型、协议、价格、分组归属 | `GET /api/pricing`（带令牌）：`enable_groups` 与可用分组取交集 |
| Auto 上限 | `GET /api/token/auto-groups` |
| 用量统计 | `GET /api/log/self/stat`、`GET /api/log/self` |
| 单次账单 | `GET /api/log/self?request_id=` |

**加载策略**：先显示本地缓存，后台刷新后替换。刷新失败时继续显示缓存并标注时间，绝不显示 0。过去日期的用量不会再变，存在本地后只拉今天和缺失的日期。配置集只存在本机。

系统预设也不需要服务端：`推荐` 用线上 `auto_groups` 的顺序，`低成本` 按用户倍率升序。

### 3.4 后台设置（不改代码）

- `auto` 必须在用户可用分组内，否则 `Group=auto` 的 Token 会被拒绝。匿名 `/api/pricing` 的 `usable_group` 不含 `auto`，要用真实账号确认。
- `MaxTokenAutoGroups` 不小于单个模型的分组数（默认 5，运行时**静默截断**，客户端按 `/api/token/auto-groups` 校验）。
- 可选：新增亲和规则，键取请求头 `X-AICLI-Session`，`SkipRetryOnFailure=false`。不修改现有默认规则。
- 保持 `LogConsumeEnabled=true`，否则账单来源消失。
- `RetryTimes` 由运营决定。

### 3.5 多设备

所有设备粘贴**同一个系统访问令牌**（网页生成，不是 `sk-` 密钥）。绑定界面说明：`在网页「个人设置」中生成。令牌只显示一次，请妥善保存；绑定其他设备时需要粘贴同一个令牌。` 这是因为令牌每个账号只有一个，接口读不回明文，重新生成会让其他设备失效。

- 用户不接触 `sk-` 密钥。客户端用系统访问令牌自动取得，按分组顺序复用，第二台设备不会重复创建。
- 账户、余额、服务端用量都是账号级数据，各设备看到的一致；本地统计和配置集只属于本机。
- 令牌失效时（如在别处重新生成），设备进入"请粘贴新令牌"状态，缓存仍可查看，粘贴后配置都保留。
- 无法单独撤销某台设备：撤销等于所有设备一起失效。解绑只清除本机内容。

### 3.6 这样做的代价

- **访问令牌权限大**：没有 scope、不过期、每个用户只有一个，桌面端被攻破就能操作账户里的 Token 和资料。缓解措施：只存 Keychain，不进日志、客户端配置和前端状态；删号、重新生成令牌等操作需要 Security Proof，访问令牌做不了。
- **令牌靠用户自己保存**：丢了只能重新生成，所有设备都要重新粘贴；客户端也无法替用户撤销，要到网页上操作。
- **每次调用都写审计**：new-api 对访问令牌请求记审计，所以只在打开页面时和每 10 分钟刷新，不做高频轮询。
- **流中重试无法修复**：`controller/relay.go` 的重试循环不检查是否已写出，`RetryTimes > 0` 时可能污染已发出的 SSE【推断】，阶段 A 实测后由运营决定取值。
- **响应头不带所选分组**：分组和费用只能事后查日志。
- **配置集不跨设备同步**：首版只存本机。

---

## 四、客户端路由改造

### 4.1 Provider 降级为内部 DTO

`Provider` 在 proxy 中出现 235 次，还有外键（`provider_health → providers`）、托盘和用量 JOIN，**整删代价大且没有收益**。

做法：每个 app 按线路保留三行隐藏种子 Provider（`aicli-main`、`aicli-global`、`aicli-apac`，参考 `database/dao/providers_seed.rs`），这样外键、熔断键和用量 JOIN 都不用改。解析器克隆这些行，覆盖模型和协议。凭证不写入 `settings_config`，只在内存中注入。

### 4.2 关键接入点

| 需求 | 位置 | 要处理的陷阱 |
|---|---|---|
| 路由解析 | `handler_context.rs:128` 的 `stack` 参数泛化为 `RouteDecision`；6 个 handler 调用点 | **不能直接复用 Stack 分支**：它会关掉故障转移和全部超时（`:133`、`:250-265`）。Gemini 模型名在 URL 里（`:231`） |
| 线路 | 适配器读取 base_url 处（`providers/claude.rs:731`、`gemini.rs:168`、`codex.rs:1025`） | 三个适配器的 `settings_config` 形状不同，合成时分别处理 |
| 凭证注入 | `forwarder.rs:2110`，删除 `:2112-2247` 的 OAuth 分支 | 策略见 `providers/auth.rs:90-107` |
| 关闭路由记录 | `forwarder.rs:653` 的 `try_switch` | 命中备用线路会被当成切换供应商写入路由状态，必须关闭 |

新代码集中放在 `src-tauri/src/aicli/`（`lines`、`session`、`catalog`、`resolver`、`secrets`、`usage`、`privacy`），对 forwarder 和 handlers 只做最小改动，以便继续同步上游。

### 4.3 线路与超时

超时目前按 app 配置，且**流式总时长不存在**（reqwest 设 24h）。要做三处改动：

1. 给 `StreamingTimeoutConfig`（`handler_context.rs:20`）加每线路的首字节、静默、总时长。
2. 转发器改为**每次尝试**取超时值（`forwarder.rs:2772`、`:3117`）。
3. 原生 Anthropic 走 hyper，等响应头用的是非流式超时（`:2729`、`:2803`），要一并覆盖。

初始值：Cloudflare 线路首字节 100s、静默 110s；CDN 线路首字节和静默 300s（对齐 new-api `STREAMING_TIMEOUT`），总时长 10 分钟。

**未解决的关键问题**：120 秒到底是总时长还是空闲超时？原方案同时写了"连接上限"和 `maxIdleMs`。new-api 已有 SSE ping（`relay/helper/stream_scanner.go:108-115`）。如果是空闲超时，长流式请求就不必回避 Cloudflare，"自动降低 Cloudflare 优先级"这条规则可以取消。由阶段 0 实测决定【实测】。

### 4.4 全部客户端经代理，且有本地鉴权

- 现状：14 个 AppType 里只有 5 个经过本地路由（Claude、Codex、Gemini、GrokBuild、Desktop）。其余 9 个直连，路由和隐私都不生效。不能把真实凭证写进它们的配置，所以全部指向代理。
- 新增入口 `/app/<client>/...`：路径决定协议，client 只用于统计。Pi、Mcode、Kimi、DSH、Omp、Agy 目前 `get_adapter` 返回 None，AICLI 路径按协议选适配器。
- **Tier 1**（阶段 3，先过验收）：Claude Code、Codex、Gemini CLI、Claude Desktop、Grok。
- **Tier 2**（阶段 7）：其余 9 个。逐个做真实端到端测试，未通过的标记为不支持，**绝不降级成写入真实凭证**。
- **本地令牌**：现在入站只有 Desktop 校验网关令牌，本机任何进程都能消耗注入的凭证。为每个客户端生成随机本地令牌，代替 `PROXY_MANAGED` 写入配置，在 `server.rs:423` 的 layer 校验。

### 4.5 模型目录发布与别名

复用 Stack 的发布链路（Claude 的 `modelPicker`、`mode/controller.rs:220`；Codex 目录文件；Desktop 模型路由）和陈旧客户端检测，只替换数据源。

**必须有别名层**。按 `mode/stack.rs:33-44` 的注释：Claude Code 的 id 要含 `claude` 且不能以 `claude-` 开头（否则 MAX 窗口不生效）；Desktop 有厂商名黑名单，所以 id 里只放数字。原方案示例中直接发布 `gpt-6.1-sol` 到 Claude Code 选择器的做法，按这些约束走不通。

- Claude Code：`aicli-claude-<key>--<model>`
- Desktop：`claude-fable-aicli-<6 位数字>`
- `/v1/models` 由解析器统一返回，取代现在的"Stack 列表或目录文件"

### 4.6 Route Profile（保留原设计，简化落地）

- 策略在客户端**编译**成 `profile + model → ordered_groups → token_id`。价格和分组事实始终取自线上，客户端不改写。`fixed_priority`、`cost_first` 都这样处理；稳定优先首版由运维维护的预设顺序表达，不做动态指标。
- 编辑流程保留：先预览，同名模型列出冲突，不静默覆盖。为避免冲突太多难以操作，提供"按分组顺序全部确认"一键操作。高级用户可用别名保留第二份。
- 作用域 `session > project > app > global`。同一会话固定配置集，目录更新只影响新会话。
- 系统预设只读，提供"从预设新建 / 重置 / 查看差异"。分组被撤销时配置集进入"需要修复"状态。
- 普通用户默认用预设，看不到这些。

### 4.7 线路优先级（原方案未定义）

`模型 preferred_line > 配置集 line_policy > 全局 selected_line_id`。优先级最高的作为首选，其余线路作为首字节前的备选。

---

## 五、隐私引擎（Rust 原生，进程内）

### 5.1 来源

| 能力 | 来源 |
|---|---|
| 占位符 `{{KIND_8位base32}}`，HMAC-SHA256(`kind\0value`)，密钥存 Keychain | Magpie `internal/redact/redact.go`。同值恒得同占位符，重启后不变，天然保住前缀缓存 |
| 值表：内存有界（20 万条，满时淘汰最旧一半）；客户端重发历史时自动重新填充 | Magpie |
| JSON 遍历和协议保护 | Magpie `json.go`（`signatureKeys`、`keepIn`、`scan`） |
| 流式：事件级扣留，三类尾部判断（花括号、裸词、形状） | Magpie `writer.go`，读码显示裸 token 也能处理，比 Maskit 只扣留 `{` 开头更完整，由 PoC C 验证 |
| 按通道的工具参数、thinking 只读不还原、NFKC 和零宽归一化、出口复检、未还原计数 | **Maskit 的行为规格**，只借思路，不复制正则、注释、测试夹具 |
| 中文证件和银行卡校验（GB 11643、Luhn、mod-97） | 按公开算法自行实现 |

Magpie 的手机号规则还会结合上下文排除订单号这类误判，这是 Maskit 规则之外值得直接带上的长处。

### 5.2 接入

- **Mask**：6 个解析点之后、选路之前（如 `handlers.rs:326` 与 `:328` 之间），抽成公共函数。
- **Restore**：流式包在 `create_logged_passthrough_stream` 的 7 个调用点之外；非流式和错误出口有 7 处左右（`response_processor.rs:326`，`handlers.rs:910/1738/1970/2134/2299/2347`）。还要清除实体头。
- **压缩**：Claude 和 Gemini 目前不解压请求体（`handlers.rs:928` 只有 Codex 走），要补上。隐私开启时上游发 `Accept-Encoding: identity`，流式收到压缩内容就 fail-closed（现在只告警，`response_processor.rs:176`）。
- **落库**：`proxy_request_logs.error_message` 存上游错误全文，放在 restore 之前写，保证落库的是脱敏文本。

### 5.3 产品行为（沿用原方案）

默认关闭；开启后默认隐私优先（失败即阻断），可选可用性优先（明确显示降级）。关闭时必须跳过全部隐私代码，行为与基线一致。

无法扫描的图片和文件在隐私优先下阻断并给出明确提示，提供"允许未扫描媒体"设置，默认关闭。

### 5.4 已知局限

- **首版不含 NER**。Magpie 的中文姓名启发式（`zhname.go`）注释写明是"model on"时才启用。Rust 版要单独评估它脱离模型后的误报，首版姓名检测作为可选项，默认范围是密钥、手机、邮箱、证件、卡号、自定义词。本地模型放到后续版本。
- **工作量上调**。Magpie `internal/redact` 非测试代码约 231KB。扣除模型相关和姓名部分后，估算核心 3～5 人周，高于最初 2～4 人周的粗估，误报长尾还要持续投入。
- **Gemini 流式未确认**。`eventCtx` 只看 `type/item_id/index`，Gemini 事件没有这些字段，按 path 拼接理论上可行【推断】，但没找到专门测试。PoC C 要覆盖。
- 上游模型仍可关联同一实体，也可能转述占位符含义，这不在隐私化能力范围内。

---

## 六、身份、绑定与数据

### 6.1 全新身份，不做迁移

配置目录用 `~/.aicli-client`，本地端口也换掉（默认 15721 会和 cc-switch 冲突），不读取也不修改 `~/.cc-switch`。

**这直接取消了原方案的"删除旧 Provider"迁移**：新应用从空数据库开始，官方 OAuth 文件原地不动，风险随之消失。

要改的地方：
- `config.rs:318`、`live/engine.rs:36`，以及**不走覆盖路径**的 `DeviceStore::for_device`（两处都要改）。
- 改名面：249 个文件，`src/` 和 `src-tauri/src` 共 2014 处；Info.plist、WiX、Flatpak、CI 也要改；`lib.rs:1143` 硬编码的 `cc-switch-handler.desktop`。
- 新的更新器公钥和端点。旧版 cc-switch 不会升级到本应用【推断】。

**接管和停止接管**：写入客户端配置前先备份原文，复用 `proxy_live_backup`，但备份可能含用户原有密钥，应存入 Keychain 或 0600 文件。"停止接管"按客户端还原。检测到 cc-switch 也在接管同一客户端时给出警告，否则两者会互相覆盖配置。

### 6.2 绑定

```text
客户端点"绑定" → 浏览器打开网页个人设置 → 复制系统访问令牌 → 粘贴到客户端
→ 用 GET /api/user/self 校验 → 令牌入 Keychain → 接管客户端 → 刷新首页
（aicliapi://v1/bind 只负责打开绑定窗口，不携带任何密钥）
```

深链**不携带** `apiKey/accessKey`，避免进入系统日志和启动参数，这是对原方案第四节的主动变更。还要修复现有缺陷：解析失败时原始 URL 会存入内存并交给前端（`lib.rs:304`）。

### 6.3 密钥存储

现状只有 macOS 通过调用 `/usr/bin/security` 用钥匙串，Windows 和 Linux 没有。新增 `keyring` crate。Linux 无 Secret Service 时退回 0600 文件，并在设置页显示降级状态。

### 6.4 数据库（user_version 24 → 25）

```text
aicli_account       id, user_id, username, user_group, selected_line_id, active_profile_id,
                    privacy_enabled, privacy_failure_mode, last_sync_at
aicli_cache         kind, fingerprint, fetched_at, body_json   -- 账户、分组、目录的最近一次结果
aicli_profile       id, name, source, preset_id, revision, body_json
aicli_managed_token token_id, groups_hash, masked, name
aicli_usage_daily   day, quota, requests, is_final
aicli_line_health   line_id, checked_at, status, latency_ms, http_status, error_code
proxy_request_logs  增加 line_id, profile_id, upstream_request_id, server_group, billed_quota, billed_at
```

令牌和 `sk-` 明文只存 Keychain，不进数据库。

### 6.5 用量口径

首页分开标注：服务端余额、服务端已用（`/api/log/self/stat`）、本地请求统计（现有 UsageDashboard）、预估费用。远端失败时显示缓存和时间，不显示零。

单次请求账单：取 `X-Oneapi-Request-Id`，按 1s、3s、10s 查询，查不到时显示"等待服务端记账"。日志是同步写入，但发生在响应处理之后，客户端可能先看到流结束。

### 6.6 前端

- `src/lib/navigation.ts` 默认页改 usage；`src/App.tsx`（1925 行，226 行涉及 provider）先摘除入口，再删 `components/providers`，**保留** `mode/` 下仍需要的面板。
- 新增 `src/components/aicli/`（首页四卡片：账户、线路、配置集、本地统计）和配置集编辑页。
- `Sidebar.tsx` 比较干净，改动小。5 种语言都要改。

### 6.7 上游同步

全量合并不现实（Provider 目录近 3 个月 278 次提交，`mode/` 约 2026-09 底才新增）。记录分叉基线 commit，配置 upstream 远程，把 `providers/transform*`、`streaming*` 和客户端写入器当作上游来源，定期 cherry-pick。不要在这些文件里做改名，否则每次都冲突。

---

## 七、实施阶段

先验证再替换，每个阶段的出口条件不满足就不进下一阶段。

| 阶段 | 内容 | 出口条件 |
|---|---|---|
| **0 实测和 PoC** | 测 120 秒语义；确认 `api.aicli.qzz.io` 的 TLS（2026-10-31 从开发机访问已返回 200）；PoC A/B/C（见下） | 三个 PoC 通过，文中【实测】项全部定案 |
| **1 账户与目录** | 第三节的绑定、目录、Token 获取、缓存 | 用未修改的 rc.34 集成测试通过 |
| **2 身份和绑定** | 改名、新目录、新端口、深链、Keychain、首页骨架 | 安装包能启动，票据绑定跑通，冷启动进入用量页 |
| **3 路由主链路** | Resolver、合成 Provider、线路、超时、错误分类、本地令牌、Tier 1 | 切线路生效；模拟主线路故障时首包前切换；401/429 不换线路 |
| **4 目录和 UI** | 模型发布、别名、配置集编辑、移除 Provider UI | 无增删改供应商入口；Claude Code 选择器能选到 GPT 模型 |
| **5 用量** | 账单回填、四卡片、口径标注 | 远端失败显示缓存；账单与服务端日志一致 |
| **6 隐私** | Rust 引擎，先非流式后 SSE，接入全部出口 | 见第八节隐私验收 |
| **7 收尾** | Tier 2 客户端、三平台安装包、许可证、发布 | 第八节全部通过，法务确认 |

**三个 PoC**
- **A：Claude Code → gpt-6.1-sol**。比较客户端转换和服务端转换，覆盖文本、工具调用、流式、thinking、图片、错误格式、别名是否进入选择器。定出 `full/partial/text_only` 兼容级别。
- **B：多配置集**（未修改的 rc.34 测试环境）。同一模型按不同配置集走不同分组，核对日志中的分组和实际扣费；验证亲和不串；实测流中错误与重试的行为。
- **C：Rust 脱敏流式**。四种协议，覆盖占位符跨 chunk、工具参数、thinking、客户端取消，并与 Maskit 黑盒对照。

**许可证**（工程判断，发布前需法务确认）
- 不修改 new-api，因此不产生修改版的 AGPL §13 义务；若日后打补丁，须重新评估。
- 客户端只通过 HTTP 调用，不因此成为衍生作品【推断】。
- Magpie 移植要保留版权和 MIT 声明。`zhdict.txt` 来自 jieba（MIT），一并保留声明。
- Maskit 只在开发机独立运行做对照，不进入产物和仓库，不复制其代码、规则和夹具。

---

## 八、验证

**方案文档本身**：接口和行号来自对 tag 或本地代码的读码调研，其中高影响项经过二次抽查，其余行号没有逐条复核，实施时要以当时代码为准。抽查曾纠正一条错误结论：`/api/log/token` 实际存在（`router/api-router.go:340`），只是有 Critical 限流，不适合轮询。

**实现的端到端验收**
- **基线**：隐私关闭时，四种协议的请求和响应与阶段 0 录制的基线一致；现有 `cargo test`、`pnpm test` 不退化。
- **路由**：同一模型两个配置集，服务端日志分组不同、费用可解释；同会话不跳组；目录更新后旧会话继续使用原路由。
- **线路**：断开主线路，首包前切换；首包后断开不重放，并报告中断；三线路全部不可用时显示离线；三条线路分别测 TLS、`/api/status`、绑定、`/v1/models`、四种协议、SSE 长连接。
- **错误**：模型不存在、能力不支持、额度不足、凭证失效各显示不同错误。
- **密钥**：在配置文件、SQLite、日志、前端状态、错误提示、崩溃上下文中搜索凭证，结果为零；未带本地令牌的请求被拒绝。
- **隐私**：敏感文本不出现在上游请求和本地日志；跨 chunk 占位符还原正确；工具调用、thinking 签名、加密字段不被破坏；重启后同值同占位符，提示缓存命中率不下降；引擎故障时两种模式行为符合设置；与 Maskit 对照样本的差异都有解释。
- **接管**：停止接管后客户端配置和备份完全一致，官方登录可用；`~/.cc-switch` 全程未被修改。
- **发布**：三平台安装、深链注册、升级、撤销设备后请求被拒绝；产物不含 Maskit、调试 Key、第三方供应商资源。

---

## 九、相对原方案的修正清单

| 原方案 | 核查结果 | 新方案 |
|---|---|---|
| `GET /api/token/:id/key` | 实际是 POST，Critical 限流（默认每 IP 20 次/20 分钟） | 用 `POST /api/token/batch/keys`，取一次存 Keychain |
| `accessKey` 调用用户接口 | 必须是系统访问令牌；`sk-` 返回 401，令牌也不能调 `/v1` | 令牌管账户，`sk-` 发请求，各司其职 |
| 多 Token / Key 池 | 按顺序去重后数量很少 | 复用现有 Token，缺的才调接口创建 |
| `pricing_version` 判断变更 | 硬编码常量 | 目录内容 hash |
| new-api 首字节前才重试 | 代码没有这个保证 | 实测；不改 new-api，由运营决定 `RetryTimes` |
| 依赖服务端重试 | 默认亲和规则使其失效，`RetryTimes` 默认 0 | 新增专用亲和规则，不改默认规则 |
| `X-AICLI-*` 四个响应头 | 服务端不暴露所选分组 | 不加响应头，事后查 `logs` |
| 直接发布 `gpt-6.1-sol` | 与选择器约束冲突 | 别名层 |
| 所有客户端都走路由 | 只有 5 个经过代理 | 全部接入 + 分 Tier 验收 |
| Key 入 Keychain | 只有 macOS 有 | 新增 `keyring` |
| 复用 Stack | 会关掉故障转移和超时 | 只复用发布链路 |
| 新增服务端接口 | 现有接口已够用 | 不新增服务端，不写数据库 |
| 线路自动切换 | 401/404/429 会误触发 | 只有传输错误才换线路 |
| Maskit sidecar | 无法脱离 mitmproxy，包体大，AGPL 分发义务 | Rust 原生 |
| 迁移删除旧 Provider | 有误删风险 | 全新目录，不迁移 |
| 深链带密钥 | 泄漏面大 | 深链只打开绑定窗口，令牌由用户粘贴 |
