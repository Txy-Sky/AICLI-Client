# AICLI Client 完整改造方案

本文档是基于根目录的 cc-switch 源码、`source-reference/maskit`、`source-reference/new-api-v1.0.0-rc.34` 整理出的第一版实施方案。根目录继续作为二开代码，参考目录只用于查阅上游实现。

## 一、目标与已经确定的约束

应用定位为 AICLI 中转站专用客户端，保留 cc-switch 对原有客户端的配置能力，去掉通用供应商管理，把所有中转请求收敛到 AICLI 的固定线路。应用标识和深链如下：

| 项目 | 定义 |
| --- | --- |
| 产品显示名 | AICLI Client（最终品牌文案可再调整） |
| 包名 / Bundle Identifier | `com.aicliapi.com` |
| 深链 Scheme | `aicliapi://` |
| 主线路 | `https://aicliapi.com` |
| 全球备用线路 | `https://aicli.qzz.io` |
| 亚太加速线路 | `https://api.aicli.qzz.io` |
| 服务端版本基线 | new-api `v1.0.0-rc.34` |
| 第一版客户端范围 | cc-switch 当前支持的原有客户端和协议适配器 |
| 隐私化 | 用户可选，第一版只处理 API 请求 / 响应文本和 JSON / SSE，不处理浏览器扩展和文件 |

三个地址使用同一套 new-api 路由。前两个地址经过 Cloudflare，存在约 120 秒连接上限；`api.aicli.qzz.io` 走 CDN，不采用这个 120 秒限制，适合作为长时间流式请求线路。线路地址写死在客户端，用户只能选择线路，不能编辑、添加或删除地址。

参考依据包括 [AICLI 文档](https://aicliapi.com/doc/)、[new-api v1.0.0-rc.34 源码](https://github.com/QuantumNous/new-api/tree/v1.0.0-rc.34) 和当前工作区中的 [上游源码索引](E:/项目/开发/AICLI-Client/source-reference/README.md)。

## 二、用户打开应用后的主流程

应用启动页改成「用量统计」，不再默认落到某个客户端的供应商列表。启动流程如下：

1. 启动本地数据库、密钥存储和线路健康检查。
2. 如果没有绑定账户，首页显示绑定引导和空状态；用户可以通过深链导入 `apiKey` 与 `accessKey`。
3. 如果已经绑定，首页优先加载本地缓存，随后异步刷新账户额度、Token 分组、可用模型和线路状态。
4. 用户从侧栏进入具体客户端时，仍然可以管理该客户端的启用状态和模型映射；这些操作不会改变 AICLI 账户和线路配置。
5. 关闭应用后再次打开仍然进入用量统计页。运行期间用户可以切换到其他页面；不把「供应商页」保存为下一次启动首页。

首页建议分成四块：

- **账户总览**：显示账户名称、用户组、总额度、已使用额度、剩余额度、最后同步时间和绑定状态。
- **线路状态**：显示当前线路、延迟、最近一次成功请求、线路类型（Cloudflare / CDN）和切换按钮。
- **分组 Key**：显示每个分组的掩码 Key、可用模型、剩余额度、过期时间、最近错误；不显示完整 Key。
- **本地请求统计**：复用 cc-switch 已有的 `UsageDashboard`，显示当天、7 天、30 天请求量、Token、模型分布和本地会话记录。远端统计和本地统计分别标注来源，避免把两种口径混在一起。

当远端暂时不可用时，首页保留最近一次缓存，并明确显示「缓存数据」和时间；不能把网络错误渲染成额度为零。

## 三、线路设计与 120 秒限制

线路使用静态注册表，不进入用户可编辑的 Provider 表单：

```ts
type AicliLine = {
  id: "main" | "global" | "apac";
  baseUrl: string;
  label: string;
  priority: number;
  transport: "cloudflare" | "cdn";
  maxIdleMs: number;
};
```

建议初始配置：

| 线路 | 默认优先级 | 传输特征 | 客户端策略 |
| --- | ---: | --- | --- |
| 主线路 `aicliapi.com` | 100 | Cloudflare，约 120 秒上限 | 默认线路；普通请求优先使用 |
| 全球备用 `aicli.qzz.io` | 90 | Cloudflare，约 120 秒上限 | 主线路失败后备用 |
| 亚太加速 `api.aicli.qzz.io` | 80 | CDN，无该 120 秒上限 | 长流式请求和亚太用户优先选择 |

线路探测分为两层：

- **无密钥探测**：请求 `/api/status`，只判断域名、TLS、基础 HTTP 是否可达。
- **带密钥探测**：用当前账户访问 `/api/user/self` 或用已绑定 API Key 请求 `/v1/models`，确认线路不仅能打开，而且能服务当前账户。

Cloudflare 线路的客户端读超时设置为 105～110 秒，用于在服务端 120 秒断联前尽早给出线路错误；CDN 线路使用较长的安全上限，例如 10 分钟，同时仍保留连接、响应头和整体请求上限。线路健康状态使用指数退避，避免每次失败都同时打满三个地址。

流式请求必须遵守下面的重试边界：

- 在响应头或第一个有效数据块到达前失败，可以换线路重试一次。
- 已经向客户端返回过正文后断开，不能自动重放原请求，避免重复扣费、重复执行工具调用或产生两份回答；页面显示「流式连接中断」，让用户选择重试。
- 预计持续时间较长的请求，用户可以在首页把当前线路切到亚太加速；后续客户端配置统一使用所选线路。
- 线路切换只影响后续请求，不强行中断正在进行的请求。

线路选择持久化为 `selected_line_id`。当用户选择某条线路时，不修改域名文本，只更新注册表中的线路 ID。自动故障切换只在当前线路不可建立连接、鉴权失败前或响应头之前触发；如果三条线路都不可用，保留最后一次线路并显示离线状态。

## 四、账户绑定与 Deep Link

第一版深链格式定义为：

```text
aicliapi://v1/bind?apiKey=<relay-api-key>&accessKey=<dashboard-access-token>&line=main
```

`line` 可选，缺省为主线路；后续可以扩展 `ticket` 或一次性绑定码，不改变 `v1/bind` 的入口。深链处理规则：

1. 校验 Scheme、版本、路径、参数长度和字符集。
2. 解析后立即从 URL、事件日志、错误日志、Toast 和崩溃上下文中清除 `apiKey`、`accessKey`。
3. 优先按 `line` 访问；线路失败时按主线路、全球备用、亚太加速的顺序尝试，成功后记录实际线路。
4. 用 `accessKey` 访问 `/api/user/self`，取得账户资料和额度；失败时不落库。
5. 用 `accessKey` 获取用户可用分组和模型目录。
6. 用 `accessKey` 获取 Token 列表；列表接口只返回掩码 Key 和元数据，需要使用某个 Token 时再调用 `/api/token/:id/key` 获取完整 Key。
7. 将完整 Key 和 `accessKey` 写入操作系统 Keychain；SQLite 只保存引用、掩码、摘要和状态。
8. 完成绑定后自动刷新首页，并把所有受管客户端切到 AICLI 内置 Provider。

new-api `v1.0.0-rc.34` 的相关接口可以按下表实现：

| 用途 | 接口 | 鉴权 |
| --- | --- | --- |
| 账户资料、总额度 | `GET /api/user/self` | `accessKey` |
| AICLI 绑定会话和配置集凭证 | `POST /api/client/session`（建议新增） | `accessKey` 或一次性 ticket |
| AICLI 路由配置集 / 模型目录 | `GET /api/client/catalog`（建议新增） | 绑定会话 |
| Token 分组自动路由信息 | 仅服务端内部配置 `AutoGroups` | 服务端管理 |
| 用户聚合统计 | `GET /api/log/self/stat` | `accessKey` |
| 单 Key 额度、过期时间和模型限制 | `GET /api/usage/token/` | 对应 `apiKey` |
| OpenAI 兼容模型列表 | `GET /v1/models` | 对应 `apiKey` |

绑定只接收服务端发放的配置集 runtime credential，不提供「新增供应商」「新增 API Key」「编辑 URL」功能。若服务端凭证被撤销或禁用，客户端下一次同步将对应配置集标记为不可用，并从模型路由中剔除。`/api/token/` 和取完整 Key 的接口只供服务端运维或兼容旧绑定流程，不应作为普通客户端的批量同步接口。

## 五、Channel 凭证与模型路由

不能把账户当成单个 API Key。服务端为每个已启用的 Route Profile 维护受管 runtime credential 或服务端策略，客户端使用「模型 → Route Profile → runtime credential」三级路由，真实分组留给 new-api：

1. 绑定会话返回已授权 Route Profile 的短期凭证引用、过期时间、额度摘要和目录版本。
2. 每个 Route Profile 使用服务端目录生成统一模型列表；不把 `AutoGroups` 或完整 Key 暴露到前端。
3. 同一模型在多个配置集存在时，按用户选择和会话粘滞固定配置集；只有用户允许自动切换或当前凭证失效时才切换。
4. 收到 401 / 403 / 凭证失效错误时，只冷却当前配置集，避免把全部配置集熔断；首包前可以切换备用配置集。
5. 收到模型不存在或协议能力不支持错误时，刷新配置集目录；若仍不可用，给出具体的「模型—配置集—协议」提示。
6. 在客户端配置里只写入 AICLI 本地代理地址和内部 Provider ID，不把 runtime credential 写入 Claude、Codex、Gemini 等客户端配置文件。

建议的数据结构：

```text
model_id
  -> profile candidates [{ profile_id, runtime_credential_id, priority, cooldown_until }]
  -> selected profile
```

实际请求的 `Authorization` 头只在 Rust 代理进程内拼接。前端只接收配置集名称、凭证掩码和状态；服务端分组在请求完成后通过日志或可选响应头回填。

## 六、Provider 改造范围

cc-switch 目前以 Provider 为中心，包含新增、编辑、复制、删除、通用 Provider、预设供应商和第三方宣传资源。二开后改成「受管 AICLI 配置」：

- 每个原有客户端仍保留一个内部 AICLI Provider，Provider ID 由客户端固定生成。
- Provider 名称、图标、URL、类别和路由配置由程序控制，用户不能编辑。
- 用户可见操作只剩启用客户端、选择模型、选择线路、查看状态和打开用量统计。
- 删除 `AddProviderDialog`、`EditProviderDialog`、复制、删除和通用供应商入口。
- 删除第三方中转站的预设、推广卡片、图标、赞助信息和相关文案；保留客户端适配器、配置生成器、会话、MCP、技能等与 Provider 市场无关的能力。
- 迁移时直接删除旧 Provider 数据，不做备份；启动迁移后创建或修复受管 AICLI Provider。原始实现仍可在 `source-reference/cc-switch` 查询。

迁移必须是幂等的：应用重启、升级或用户重复绑定都不能生成第二份 AICLI Provider。旧 Provider 的 `settings_config`、`provider_endpoints` 和旧深链导入记录不再参与运行时路由。

## 七、隐私化模块

Maskit 的核心逻辑与 mitmproxy、Flask 面板和全局会话状态耦合，第一版不把整套工程直接复制进前端，也不把它暴露成公网服务。采用私有本地模块：

```text
客户端请求
  -> cc-switch Rust 本地代理
  -> 可选 Maskit 私有本地引擎：mask
  -> AICLI 线路
  -> 可选 Maskit 私有本地引擎：restore
  -> 客户端
```

模块接口保持小而稳定：

```text
mask(session_id, content_type, body) -> masked_body, sid, diagnostics
restore(session_id, stream_id, content_type, body, final) -> restored_body
ping() -> engine_version, rule_version, ready
```

实现要点：

- 首期支持 OpenAI Chat Completions / Responses、Anthropic Messages、Gemini 文本与 SSE 等现有代理路径中的文本 JSON；文件、图片附件、浏览器扩展和独立上传接口留到后续版本。
- 只处理请求正文和响应正文，不把 `Authorization`、API Key、Access Key、Cookie、线路 URL 等传给 Maskit。
- 流式响应按会话维护 `sid` 和 `stream_id`，每个 chunk 只还原一次；在 `final` chunk 到达后释放映射。
- 保留 Maskit 对 Anthropic thinking signature、OpenAI encrypted content、Gemini thought signature 等协议字段的保护逻辑，避免隐私化破坏协议完整性。
- 隐私开关为用户可选，建议默认关闭以保持现有行为；打开后默认采用「隐私优先」模式，Maskit 不可用时阻断请求。另提供「可用性优先」模式，由用户明确选择后才允许降级直通。
- 日志只记录规则版本、耗时、数量、摘要和错误类型，不记录敏感原文、完整占位符映射或明文 Key。
- 本地引擎使用随机回环端口并配随机认证令牌，或使用操作系统管道；只允许本客户端进程访问。

Maskit 为 AGPL 项目，当前按私有内部组件设计，不把其源码作为公开客户端的一部分发布。即使当前仓库只供私有使用，也应保留内部来源和许可证记录；如果未来把带 Maskit 的二进制或源码提供给外部用户，需要重新评估 AGPL、对应源码提供和 NOTICE 要求。

## 八、数据存储

建议新增受管账户表，而不是把密钥塞进原 Provider JSON：

```text
aicli_profiles
  id, display_name, username, user_group, selected_line_id,
  privacy_enabled, privacy_failure_mode, last_sync_at, created_at

aicli_secrets
  id, profile_id, kind(access_key|runtime_credential), keychain_ref, key_digest,
  masked_value, last_verified_at

aicli_route_profiles
  id, account_id, display_name, scope, source, preset_id, revision,
  selected_groups_json, model_bindings_json, credential_ref,
  catalog_version, enabled, expires_at,
  remaining_quota, used_quota, last_ok_at, last_error

aicli_model_routes
  model_id, route_profile_id, runtime_credential_id, server_group_digest,
  priority, cooldown_until, last_error

aicli_line_health
  line_id, checked_at, status, latency_ms, http_status, error_code
```

原有本地用量表继续保存客户端会话统计；新请求日志补充 `aicli_line_id`、`group_name`、`key_id` 的不可逆摘要，不能保存完整 Key。远端统计刷新失败时不删除本地缓存。

## 九、文件级改造清单

前端：

- 新增 `src/config/aicli.ts`：三条线路、默认线路、协议路径和客户端显示信息。
- 新增 `src/lib/api/aicli.ts`：绑定、同步、线路健康、额度、分组和 Key 状态 API。
- 新增 `src/hooks/useAicliAccount.ts`、`useAicliLines.ts`、`useAicliUsage.ts`。
- 新增 `src/components/aicli/AicliUsageHome.tsx`、`AicliAccountBindDialog.tsx`、`AicliLineSelector.tsx`、`AicliKeyGroupTable.tsx`。
- 修改 `src/App.tsx`：首页入口固定到 usage；移除 Add/Edit/复制/删除 Provider 的 UI；接入 AICLI 首页和绑定状态。
- 修改 `src/lib/navigation.ts`：首页默认值改为 `usage`，旧版保存的 Provider 首页值迁移为 usage。
- 保留 `src/components/usage/UsageDashboard.tsx` 的本地统计能力，增加 AICLI 远端统计适配层。
- 修改所有语言包中的产品名、深链、Provider 文案和首页文案；删除第三方供应商文案。

Rust / Tauri：

- 新增 `src-tauri/src/services/aicli/`：`client.rs`、`lines.rs`、`account.rs`、`token_pool.rs`、`usage.rs`、`secrets.rs`、`privacy.rs`。
- 在 `src-tauri/src/database/schema.rs` 增加 AICLI 表和幂等迁移。
- 在 `src-tauri/src/proxy/` 接入模型选 Key、线路选择、请求前 mask、响应后 restore 和流式会话映射。
- 在 `src-tauri/src/deeplink/` 把 `ccswitch://v1/import` 改成 `aicliapi://v1/bind`，旧 Scheme 不再注册。
- 修改 `src-tauri/tauri.conf.json`、`Info.plist`、Windows 安装器、Linux Desktop 文件、更新地址和图标元数据：identifier 改为 `com.aicliapi.com`，Scheme 改为 `aicliapi`。
- 清理 Provider 预设、第三方资源和通用 Provider 服务的运行时入口；保留原有客户端配置适配器。

## 十、实施阶段与验收标准

### 阶段 0：基线冻结

记录当前 cc-switch 构建、启动、深链、四类代理协议和所有原有客户端配置的行为。参考代码只读，根目录作为唯一修改位置。

### 阶段 1：品牌、标识和首页

完成包名、产品名、深链注册、更新配置、首页默认 usage、导航和语言包；不改变请求路由。验收：安装包能正常启动，双击 `aicliapi://` 能进入绑定页，冷启动总是看到用量统计。

### 阶段 2：固定 AICLI Provider 与线路

完成三条线路注册表、线路选择、健康检查、受管 Provider 和旧 Provider 清理。验收：用户无法看到新增 / 编辑 / 删除供应商入口；切换线路后所有原有客户端使用所选地址；主线路故障可在首包前切备用线路。

### 阶段 3：账户绑定与 Key 池

完成 Deep Link、Keychain、账户同步、分组模型目录、模型到 Key 的路由和失效冷却。验收：一个账户有多个分组时，每个模型选择正确 Key；Key 不出现在前端配置、普通日志和错误提示中。

### 阶段 4：用量统计首页

完成账户额度、分组 Key 状态、线路状态和本地 UsageDashboard 合并。验收：远端失败显示缓存和错误状态，不显示假零值；刷新间隔可控；首页能从绑定、切线路和查看客户端用量继续导航。

### 阶段 5：隐私化模块

先实现私有本地引擎的 ping、mask、restore，再接入非流式 JSON，最后接入 SSE 和协议保护。验收：关闭隐私化时请求字节级行为与现有版本一致；开启后敏感文本被替换，响应还原正确；引擎故障符合用户选择的阻断 / 直通模式；工具调用、thinking signature 和加密字段不被破坏。

### 阶段 6：回归和发布

覆盖 Windows、macOS、Linux 安装包、深链注册、三线路、Cloudflare 120 秒断联、CDN 长流式、四类代理协议、所有原有客户端配置和升级迁移。发布包不包含公开 Maskit 源码、调试 Key 或第三方供应商资源。

## 十一、第一版默认行为

为减少后续实现歧义，第一版按下面的默认值开发：

- 默认线路：`https://aicliapi.com`。
- 长流式或主线路连续失败：提示切换 `https://api.aicli.qzz.io`；首包前允许自动重试一次。
- 首页：每次冷启动进入用量统计。
- 隐私化：默认关闭；用户打开后默认隐私优先，Maskit 不可用则阻断，设置中可改为可用性优先。
- Key 同步：自动同步服务端已启用 Token 的元数据，按需取明文 Key 并放入 Keychain；用户不手工新增 Key。
- 旧 Provider：迁移时直接删除，不备份；受管 AICLI Provider 幂等重建。
- 远端统计：能取到就展示，失败使用缓存并显示同步时间。

需要在进入编码前最终确认的只有一个产品取舍：隐私化打开后，是否接受「默认阻断请求」的隐私优先行为。如果没有额外指定，就按本文档实施。

## 十二、隐私化到底放在哪里

隐私化不做成用户需要单独启动、单独配置、单独选择地址的第二个代理服务。它应该融合到现有 cc-switch 的本地 Rust 代理链路中；但由于 Maskit 当前核心是 Python/mitmproxy 实现，第一版可以由 Tauri 在后台启动一个仅监听回环地址的私有 Maskit 引擎。

因此采用「逻辑融合、进程隔离」的形态：

- 对用户来说只有一个 AICLI Client 和一个本地代理端口。
- 对实现来说，Rust 代理负责协议判断、模型路由、线路选择、重试和客户端响应；私有 Maskit 引擎只负责 `mask / restore / ping`。
- Maskit 引擎不监听公网，不接受浏览器访问，不保存长期请求正文；Rust 进程给它分配随机回环端口和随机认证令牌。
- 未来如果把 Maskit 的规则和占位符引擎完整重写为 Rust，可以把它替换为进程内模块，外部接口不变，不影响主链路设计。

### 完整请求链路

```text
Claude Code / Codex / Gemini / 其他客户端
        │  原生 Anthropic / OpenAI / Gemini 请求
        ▼
本地 AICLI Rust Proxy（127.0.0.1）
        │
        ├─ 1. 读取请求、解压、解析 JSON、识别客户端协议
        ├─ 2. 提取 session_id、原始 model、stream、工具和能力要求
        ├─ 3. Model Resolver 选择公开模型和候选分组 / Key / 线路 / 上游协议
        ├─ 4. Privacy Adapter（开启时）对原生请求正文做 mask
        │       └─ 只传正文文本，绝不传 Authorization / API Key / Access Key
        ├─ 5. Provider Adapter 做模型别名替换、协议转换和兼容性整流
        ├─ 6. 根据候选线路拼接 URL，根据候选 Key 注入 Authorization
        ├─ 7. 发送到 AICLI 三条固定线路之一
        │
        ├─ 8. 接收上游 JSON / SSE，记录 usage、实际模型、分组和倍率快照
        ├─ 9. 将上游协议转换回客户端原协议
        ├─ 10. Privacy Adapter（开启时）在发给客户端前 restore
        └─ 11. 返回 Claude Code / Codex / Gemini 等客户端
```

隐私化的插入点有两个硬规则：

1. 请求正文必须在协议转换、提示词缓存日志和上游发送之前 mask。这样本地转换器、调试日志和远端模型都看不到原始敏感文本。
2. 响应必须先完成上游协议到客户端协议的转换，再在最终输出流上 restore。这样 Maskit 看到的是客户端实际收到的结构，SSE chunk、Anthropic content block 和 OpenAI delta 不会被错误拆分。

非流式请求在一次 JSON 级 mask / restore 中完成。流式请求为每个本地请求建立：

```text
local_request_id -> mask_sid -> stream_id -> restore state
```

每个 SSE chunk 只 restore 一次，收到终止事件、连接错误或客户端取消时释放状态。已经向客户端发送过正文后发生上游断联，不重放请求；重试只允许发生在响应头或首个有效数据块之前。

### 隐私引擎不可用时的行为

Rust 代理启动时调用 `ping()`，首页显示引擎状态和规则版本。隐私开关关闭时完全跳过 Maskit。隐私开关打开时提供两个模式：

- **隐私优先**：mask 失败、引擎未启动或 restore 状态丢失时阻断请求。
- **可用性优先**：mask 失败时允许原文直通，但必须在首页、请求错误和日志中显示明确的降级状态。

第一版默认采用隐私优先。无论哪种模式，都不能因为 Maskit 故障把原始敏感内容写入日志。

## 十三、统一模型目录：让一个客户端使用全部分组模型

可以实现，而且应该把“客户端看到的模型”和“服务端实际调用的模型”分开。客户端只面对一个统一目录，Rust 代理在每次请求前决定实际的分组、Key、线路和协议。

本节中的 Channel 候选在当前方案中统一解释为用户可配置的 Route Profile 候选；固定三档不是产品限制。

### 统一模型对象

每个公开模型建立一条稳定的内部记录：

```text
AicliModel
  public_id                 客户端看到的 ID，例如 claude-sonnet-4-6、gpt-6.1-sol
  display_name              UI 显示名
  upstream_model            服务端实际模型名
  supported_client_protocols [anthropic, openai, gemini]
  required_capabilities     tools / vision / thinking / stream / cache
  context_limit             最大上下文
  output_limit              最大输出
  base_pricing              基础输入 / 输出价格
  candidates[]              可用 Channel、协议和线路候选
```

每一个候选项包含：

```text
ModelCandidate
  model_id
  channel_id
  runtime_credential_id
  server_group             服务端确认后才填充，客户端启动时不硬编码
  wire_protocol             anthropic / openai_chat / openai_responses / gemini
  request_transform         客户端协议到 wire_protocol 的转换器
  response_transform        wire_protocol 到客户端协议的转换器
  pricing_snapshot          模型价格目录版本和 Channel 价格提示
  supported_lines           main / global / apac
  capability_flags
  quota_state
  health_state
```

同一个模型出现在多个 Channel 时，不生成多个重复模型，也不要求用户在 Claude Code 里记忆分组名。默认情况下 UI 显示一个模型和三个 Channel 标签；实际分组、倍率和 Key 只在服务端返回或日志同步后作为详情展示。

### 同模型不同分组、不同倍率的选择规则

Channel 不是简单的故障转移顺序，而是用户能理解的价格和服务策略。建议提供三种用户策略：

| 策略 | 选择方式 | 适用场景 |
| --- | --- | --- |
| 智能省钱 | 在能力满足、额度有效的 Channel 中选择价格较低者；价格相同再比较延迟和健康度 | 用户主动开启 |
| 稳定优先 | 先比较健康度、成功率和延迟，再在可用 Channel 中选择价格合适者 | 默认策略、长会话 |
| 固定渠道 | 用户为模型绑定一个 Channel，失败时可选择是否允许备用 | 需要固定价格或固定能力 |

智能省钱不能只比较倍率。候选必须先通过以下过滤：

1. 该 Channel 的服务端模型目录确实允许调用这个模型。
2. Token 未过期、未禁用、剩余额度足够。
3. 候选协议有对应的请求和响应转换器。
4. 客户端请求的工具、图片、thinking、流式等能力满足候选的能力声明。
5. 当前线路可用；Cloudflare 线路的长流式请求应优先排除或降权。

实际预估费用使用：

```text
estimated_cost = base_model_price × group_ratio × token_usage
```

Channel 价格提示和 Token 状态以服务端同步结果为准；最终扣费以 new-api 返回的实际 usage 和额度变化为准。首页和请求日志要记录“本次实际使用的模型、Channel、服务端分组、Key 掩码、倍率快照、线路”，这样用户能解释同一个模型为什么产生不同费用。

### 模型基础价格和能力目录

已实测 `GET /api/pricing` 能提供模型基础价格、倍率和协议能力，但它不能表达三档 AICLI Channel 的归属。因此它应作为 `PricingSnapshot`，同时由私有 new-api 增加 AICLI Channel 目录（或在响应顶层增加 `aicli_channels`）。当前返回包含 69 个模型，以及：

```text
data[].model_name
data[].model_ratio / completion_ratio / cache_ratio / create_cache_ratio
data[].model_price / quota_type
data[].enable_groups
data[].supported_endpoint_types
data[].billing_mode / billing_expr
group_ratio
usable_group
auto_groups
supported_endpoint
pricing_version
```

`supported_endpoint` 当前明确映射了：

```text
anthropic               -> POST /v1/messages
openai                  -> POST /v1/chat/completions
openai-response         -> POST /v1/responses
openai-response-compact -> POST /v1/responses/compact
gemini                  -> POST /v1beta/models/{model}:generateContent
openai-alpha-search     -> POST /v1/alpha/search
image-generation        -> POST /v1/images/generations
```

同一模型只返回一条记录，通过 `enable_groups` 表示可用分组。例如 `gpt-6.1-sol` 当前允许 `gpt`、`gpt-pro`、`gpt-max` 和 `svip`，客户端需要把模型记录与 `group_ratio` 合并成候选路由。`svip` 当前没有出现在公开 `group_ratio` 和 `usable_group` 中，不能擅自按倍率 1 处理；缺少最终倍率的分组必须标记为待鉴权或不可选。

`billing_expr` 是真实计费表达式，`gpt-6.1-sol` 等模型会根据输入长度在标准上下文和长上下文之间切换。客户端不应只用 `model_ratio × group_ratio` 假算完整费用；路由选择可以比较同一模型不同分组的最终倍率，首页费用则显示服务端实际 usage 和服务端账单结果。若要显示请求前估算，应明确标注“预估”，并保留 `pricing_version`、表达式摘要和倍率快照。

客户端请求 `/api/pricing` 时应带 `Authorization: Bearer <accessKey>`。new-api 的 `GetPricing` 会在有用户身份时应用用户组对目标分组的特殊倍率；匿名结果只能作为公共目录缓存，不能作为该用户的最终价格。

`vendors` 字段只用于服务端站点展示，AICLI Client 不应把其中的第三方供应商列表重新暴露给用户。`openai-video`、`image-generation` 等媒体模型可以保留在统一目录，但要标记“第一版隐私化不支持文件 / 二进制正文”；隐私优先模式下应阻断或要求用户关闭隐私化后再使用。

客户端同步 `/api/pricing` 后保存一个不可变的 `PricingSnapshot`：

```text
PricingSnapshot
  source_line
  fetched_at
  pricing_version
  group_ratio
  usable_group
  auto_groups
  supported_endpoint
  data[].pricing_version
```

候选生成规则是：

```text
for channel in aicli_channels:
  for model in channel.models:
    if model endpoint supports the client protocol:
      create ModelCandidate(model, channel, pricing_snapshot)
```

客户端不再根据 `enable_groups` 自行决定 Channel 归属，也不把未知倍率当成 1。`billing_mode=ratio` 的模型可以使用服务端提供的 Channel 价格提示做预估，`billing_mode=tiered_expr` 的模型必须保留表达式，客户端最多展示服务端表达式计算出的预估结果或“按实际用量结算”。`quota_type=1` 且 `model_price` 非零的图片、视频和按次模型，单独显示按次 / 按张 / 按任务计费，不套用 Token 公式。`enable_groups`、`group_ratio` 和 `usable_group` 仍保留在内部价格快照，用于服务端校验和诊断，不作为用户侧路由配置。

分组倍率和分组质量是两个维度。当前公开目录里的描述已经显示：`gpt` 是综合池，`gpt-pro` 是稳定首选，`gpt-max` 强调官方直连和不降智；`claude-code` 还明确限制为 Claude Code。这些策略应由服务端映射到 Channel，客户端只消费结构化 Channel 元数据：

```text
group_policy
  allowed_apps
  quality_level
  stability_level
  latency_level
  supports_long_context
  supports_tools
  supports_thinking
  user_visible
```

仅凭 `usable_group` 的中文描述做机器决策不够稳定，正式版本必须让服务端在 AICLI Channel 目录提供结构化的价格、能力和模型字段。真实分组实际可用性仍以服务端的 `AutoGroups`、Token 状态和模型权限为准。

### 会话级固定路由

同一模型如果每条消息都在不同分组之间跳转，会破坏提示词缓存、增加延迟，也可能因为不同分组的上下文能力不同导致结果不一致。因此采用会话级路由：

```text
session_id + public_model_id -> selected Channel candidate
```

首次请求按用户策略选出 Channel 后，后续同一会话沿用该 Channel、runtime credential 和协议。只有以下情况才切换：凭证失效、额度不足、线路不可用，或者用户主动切换策略。首包前允许换到另一个 Channel 候选；首包后不切换并重放。

### Claude Code 使用 Claude 和 GPT-6.1-sol

Claude Code 继续使用本地 Anthropic 兼容入口，模型目录同时发布 Claude 模型和 GPT 模型，例如：

```text
claude-sonnet-4-6
claude-opus-4-6
gpt-6.1-sol
```

用户在 Claude Code 的模型选择器或 AICLI Client 的模型策略页面中选择 `gpt-6.1-sol` 时，链路如下：

```text
Claude Code Anthropic Messages
  -> 本地 AICLI Proxy
  -> Resolver 选择 gpt-6.1-sol 的 Channel 和 runtime credential
  -> Anthropic Messages 转 OpenAI Responses 或 Chat Completions
  -> AICLI /v1/responses 或 /v1/chat/completions
  -> OpenAI 响应转回 Anthropic Messages / SSE
  -> Claude Code
```

现有 cc-switch 已经有 Claude、OpenAI Chat、OpenAI Responses、Gemini Native 适配器和流式转换器，AICLI 只需要把这些适配器从“Provider 配置决定”改成“ModelCandidate 决定”。同一客户端可以使用多个协议，客户端本身不需要知道协议差异。

模型目录必须带能力标记。比如某个 GPT 分组不支持 Anthropic thinking 或工具调用，就不能在 Claude Code 中伪装成完整 Claude 模型；应在目录中标记限制，或者从需要该能力的候选中排除。对 Claude Code 来说，首版至少验证：普通文本、工具调用、流式输出、thinking、图片输入、上下文长度和错误格式。

## 十四、模型路由的完整执行顺序

每个请求按下面顺序执行，不能把 Channel 选择和协议选择留给上游的随机行为：

1. 从客户端请求提取 `app_type`、客户端协议、公开模型名、session_id、stream 和能力要求。
2. 用模型目录解析公开模型。如果是 `gpt-6.1-sol`，得到该模型的所有可用 Channel 候选；如果客户端传入的是别名，先解析为公开模型。
3. 按用户策略过滤候选：固定渠道 / 智能省钱 / 稳定优先。
4. 过滤 runtime credential 状态、模型权限、能力、上下文限制和线路策略。
5. 为当前 session 固定候选，生成 `RoutePlan`：

   ```text
   RoutePlan
     public_model = gpt-6.1-sol
     route_profile = claude-code-stable
     runtime_credential_id = runtime-42
     line = apac
     client_protocol = anthropic
     wire_protocol = openai_responses
     server_group = resolved-after-request
     pricing_mode = tiered_expr
     pricing_snapshot = ...
   ```

6. 先对客户端原生请求正文执行隐私 mask。
7. 按 `RoutePlan` 执行模型名改写、协议转换、thinking / tool / image 兼容处理。
8. 注入所选 Key，发送到所选线路。
9. 如果连接、鉴权或首包前发生可恢复错误，标记候选冷却并选择下一个候选；如果已经发出正文，不重放。
10. 记录本次实际的 `RoutePlan` 和服务端 usage。
11. 将响应转换回客户端协议，再执行隐私 restore，最后返回客户端。

这样可以让一个 Claude Code 会话使用 Claude，也可以使用 GPT-6.1-sol；模型相同但分组倍率不同时，费用和实际分组清晰可追踪。

## 十五、需要新增的路由配置

在 AICLI Client 设置中增加“模型路由策略”，不再把这些信息塞进 Provider 表单：

```text
global_policy: cost_first | stability_first

model_policy:
  public_model_id
  selection_mode: auto | pinned_profile
  pinned_profile_id
  preferred_line: inherit | main | global | apac
  allow_protocol_conversion: true | false
  allow_capability_degradation: true | false
```

默认值：自动选择、会话稳定优先、允许线路自动切换、不允许能力降级、允许协议转换。用户固定 Route Profile 后，客户端不自动换到其他配置集；只有 runtime credential 失效时才提示用户或按用户开启的备用策略切换。用户主动选择“智能省钱”后，才在配置集之间重新选择候选。

首页的模型表中，每一行展示：模型名、当前 Route Profile、价格提示、线路、能力标签和“切换策略”。服务端实际分组、倍率、Key 掩码和最终费用在请求详情中展示；客户端配置文件仍然只写入统一的本地代理入口。

## 十五补充、三档渠道与 Magpie 调研结论（历史 PoC 基线）

本次加入的 Magpie 参考项目是提交 `f11bf27c1f3f0f7d5284dffa2cb8b066debaed84`。它对本项目最有价值的不是 Provider 数量，而是把“用户看到的模型”“路由组”“候选 Key / 账号”“会话粘滞”“首包前故障转移”和“实际用量记录”分成了不同层次：

本节的“三档 Channel”只用于说明 new-api Token 方案的可行性，已被下方“用户可配置的路由配置集”取代。正式产品不限制配置集数量，也不把价格档位写死为三档。

```text
Agent 请求
  -> model / group 解析
  -> 候选池排序
  -> session affinity
  -> 首包前 failover
  -> 协议转换与上游请求
  -> Requested / Served / Provider / Key / usage 记录
```

Magpie 的 `Group` 可以包含多个模型、其他嵌套组、路由策略、Affinity、Fallback、能力和展示信息；`Provider` 负责多个 Key / 账号的轮询、冷却、并发和额度；`usage.Record` 同时保留请求模型和实际服务模型。AICLI 应复用这些分层思想，但不把 Magpie 的 Provider UI 搬进来，因为 AICLI 的上游只有固定的 new-api 服务。

### 三档渠道的正确抽象

用户看到的“经济 / 标准 / 旗舰”是 **AICLI Channel**，不是 new-api 的真实分组名，也不是客户端可以编辑的 Provider。它只代表一套服务端分配的运行凭证和候选分组集合：

```text
Channel
  id: economy | standard | premium
  display_name
  runtime_credential
  model_catalog
  price_hint
  allowed_protocols
```

当前 new-api v1.0.0-rc.34 已经有适合第一版的能力：Token 有 `Group`、`AutoGroups` 和 `CrossGroupRetry`。当 Token 的 `Group=auto` 时，服务端会按 Token 的有序 `AutoGroups`，对当前模型逐组寻找可用渠道；每个真实分组仍按自己的倍率结算。也就是说，服务端可以创建三组受管 Token：

```text
economy  Token: Group=auto, AutoGroups=[服务端分配的低价组...]
standard  Token: Group=auto, AutoGroups=[服务端分配的平衡组...]
premium   Token: Group=auto, AutoGroups=[服务端分配的高稳定组...]
```

客户端只保存三组短期 runtime credential，不保存和编辑 `AutoGroups`。用户切换的是 Channel；实际使用的 new-api 分组由服务端根据模型是否存在、分组顺序、Key 状态和重试策略决定。当前接口还支持 Token 按分组返回模型列表，因此客户端可以分别读取三个 Channel 的 `/v1/models`，按模型 ID 合并成一个统一目录。

注意：rc.34 的 `MaxTokenAutoGroups` 默认值是 5，服务端分配每个 Channel 的组列表前必须检查这个上限；如果某个 Channel 需要超过 5 个真实分组，应在私有服务端提高配置或先建立服务端聚合组，不能让客户端自行截断。

还要区分“按模型找可用组”和“按模型使用不同组优先级”：rc.34 的 `AutoGroups` 是 Token 级有序列表，当前逻辑会按同一顺序对不同模型找第一个可用组，并不是每个 `model_id` 一套独立顺序。如果产品规则要求 `gpt-6.1-sol` 和 `claude-sonnet-4-6` 在同一个 Channel 使用不同的分组优先级，需要在私有 new-api 增加 `channel_id + model_id -> ordered_groups` 策略，或拆分为更多受管 Token；不能让客户端自己猜测倍率。

### 这能否只用一个 API Key

不能直接用当前 Token 数据结构表达“同一个 Key 在三个价格策略之间切换”。一个 Token 只有一个 `Group` 和一套 `AutoGroups`。第一版建议使用三个服务端受管 Token，由绑定会话一次性返回三组短期凭证；客户端在 Keychain 中保存，界面不显示原文。

如果产品规则要求不同模型在同一个 Channel 使用不同的分组优先级，需要在私有 new-api 增加 `channel_id + model_id -> ordered_groups` 策略，或拆分为更多受管 Token；不能让客户端自己猜测倍率。建议的服务端策略表为：

```text
AicliChannelPolicy
  channel_id
  model_id                 支持通配符或模型族
  ordered_groups[]         有序真实分组
  cross_group_retry
  policy_version
```

第一版 PoC 可以先使用 Token 级 `AutoGroups`，验证协议和计费；正式版本再在 `channel_select` 取得 `modelName` 后套用这张表。这样“同一个应用支持全部模型”和“不同模型走不同倍率组”都由服务端控制，客户端不需要新增分组配置。

如果坚持只给一个 API Key，就需要改 new-api 增加 `tier_id` 会话或请求头，并在服务端把 `tier_id + model_id` 映射到分组集合。这条路线还要处理伪造请求头、权限、日志和缓存，适合作为后续版本，不适合作为第一版基础。

### 必须补充 AICLI 目录接口

`/api/pricing` 能提供模型价格和真实分组倍率，但它不能表达“经济 / 标准 / 旗舰”这三个用户概念，也不应该让客户端通过硬编码推断分组归属。建议在私有 new-api 中增加 AICLI 专用目录接口，或者在 `/api/pricing` 顶层增加 `aicli_channels`：

```json
{
  "catalog_version": "...",
  "pricing_version": "...",
  "channels": [
    {
      "id": "economy",
      "display_name": "经济",
      "price_hint": "低",
      "credential_id": "runtime-...",
      "models": ["gpt-6.1-sol", "claude-sonnet-4-6"],
      "protocols": ["anthropic", "openai", "openai-response"]
    }
  ]
}
```

这个接口不返回真实 API Key 和内部 `AutoGroups`，只返回当前用户能用的 Channel、模型、协议能力和展示价格。真实分组、倍率和最终 `quota` 仍只在服务端日志中确认。目录版本变化后客户端丢弃旧的模型候选和会话路由。

### 同一模型在三个渠道都存在时

客户端统一目录只保留一个公开模型 ID，例如 `gpt-6.1-sol`，附带三个 Channel 能力标签。用户选择 Channel 后，客户端使用对应 runtime credential；同一 Channel 内由 new-api 的 `AutoGroups` 决定实际分组。客户端 RoutePlan 应记录：

```text
public_model = gpt-6.1-sol
aicli_channel = standard
runtime_credential_id = runtime-...
requested_protocol = anthropic
wire_protocol = openai-response
line = main
```

实际 `group`、实际倍率和最终消费额以服务端响应头 / 日志为准。客户端不应把 `standard` 误报成某个真实 new-api 分组。

### Magpie 可直接借鉴的细节

- **Session affinity**：同一会话尽量保持同一 Channel 和服务端候选，保留缓存；切换价格档位应从新会话或用户明确操作开始。
- **首包前 failover**：只有没有向客户端输出内容前才能换候选；首包后断开不能无感重放。
- **实际路由可观测**：Magpie 会返回实际 Provider / Model，并在 usage 中区分 Requested / Served。AICLI 应增加 `X-AICLI-Channel`、`X-AICLI-Group`、`X-AICLI-Model`、`X-AICLI-Pricing-Version`，同时继续传递 `X-Oneapi-Request-Id`。
- **候选失败冷却**：借鉴 Magpie 的 rate limit、quota、slow first token 冷却，不要每次请求都盲目重试同一分组。
- **能力过滤**：Magpie 会按模型窗口、工具、图片和 reasoning 能力过滤候选。AICLI 也必须按客户端协议和模型能力过滤，不因为模型名相同就假设三档渠道完全等价。
- **隐私链路位置**：Magpie 的网关 middleware 证明脱敏应该位于请求路由之后、上游发送之前，并覆盖流式事件；本项目仍使用 Rust 主链路加本地 Maskit 引擎，采用隐私失败关闭策略。

结论是：第一版应把三档做成三个服务端管理的 `auto` Token / Channel，沿用 new-api 已有的 `AutoGroups` 和跨分组重试；客户端只负责统一模型目录、Channel 选择、协议转换和会话粘滞。只有当三个 Token 方案不能满足账号权限或计费要求时，再增加服务端 `tier_id` 路由层。

## 十五补充二、当前推荐：用户可配置的路由配置集

固定三档只是便于讨论的临时例子，不应成为产品数据结构。正式概念改为 **路由配置集（Route Profile）**：用户可以从服务端允许的真实分组中自由组合、排序、命名，并随时恢复系统预设。

### 三个层次必须分开

```text
服务端分组（Server Group）
  new-api 的真实 group，权限、倍率、模型和 Key 由服务端控制

路由配置集（Route Profile）
  用户选择的一组 Server Group、顺序、模型取舍和策略

公开模型（Public Model）
  客户端和 Claude Code 看到的唯一模型 ID
```

用户可以编辑 Route Profile，但不能创建、修改或伪造 Server Group。配置集名称是用户自己的，例如“Claude Code 稳定”“低价实验”“长上下文”；它不应该被称为供应商或渠道。

### 配置集数据结构

```text
RouteProfile
  id
  display_name
  scope                    global | app | project | session
  source                   system_preset | user_custom
  preset_id                可选，指向系统预设
  revision
  selected_groups[]        group_id、显示名、顺序、启用状态
  selection_mode           fixed_priority | cost_first | stability_first
  model_bindings[]         每个 public_model_id 只能有一个生效绑定
  line_policy              main | global | apac | auto
  allow_profile_fallback
  catalog_version
```

`model_bindings` 至少保存：

```text
public_model_id
source_group_id
upstream_model_id
supported_protocols
capability_flags
pricing_snapshot
alias                    可选，供冲突模型保留第二份
```

同一个配置集内，`public_model_id` 必须唯一。真实分组、Key、倍率和计费表达式属于服务端事实，不由客户端写入最终账单。

### 配置集编辑和冲突处理

用户选择多个 Server Group 后，客户端先生成预览，不立即启用：

1. 拉取当前用户可用的分组目录和每组模型目录。
2. 按 `model_id` 合并模型。
3. 只有一个来源的模型自动加入候选。
4. 多个来源的同名模型生成冲突项。
5. 冲突项展示来源分组、倍率提示、支持协议、上下文、工具、thinking、图片能力和最近健康状态。
6. 用户必须选择一个来源、排除该模型，或给其中一个来源创建明确的用户别名。
7. 没有解决的冲突不能激活配置集。

默认不允许静默覆盖。即使两个分组的模型信息当前完全相同，也建议把“合并”作为用户可见的确认选项，避免服务端价格或能力变化后悄悄换组。

高级用户可以使用别名，例如：

```text
gpt-6.1-sol          -> gpt-pro
gpt-6.1-sol-stable   -> gpt-max
```

别名只改变客户端公开模型名，服务端真实模型仍由绑定记录决定；别名中不强制暴露真实分组名，用户可以自己命名为“稳定版”。

### 系统预设和恢复

系统预设由服务端提供并版本化，例如：

```text
系统预设：Claude Code 推荐
系统预设：低成本
系统预设：高稳定
系统预设：全部模型
```

预设必须是只读模板。用户操作“恢复系统预设”时，应提供三种明确动作：

- **从预设新建**：保留现有配置集，生成一个新的用户配置集。
- **重置当前配置集**：覆盖当前选择，生成新 revision，并支持撤销。
- **查看差异**：显示分组、模型绑定和策略的变化后再确认。

服务端更新预设时不能覆盖用户自定义配置；如果原分组被撤销，配置集进入“需要修复”状态，并明确指出缺失的分组和模型。

### 后端实现边界

当前 new-api 的 Token 级 `AutoGroups` 可以作为 PoC：保存配置集时由服务端生成短期 `Group=auto` runtime credential，并把选中的分组写入该 Token 的 `AutoGroups`。但把每个用户配置集都生成一个长期 Token 会造成 Token 数量膨胀，而且当前 `AutoGroups` 不能表达每个模型不同的分组顺序。

正式实现应在私有 AICLI 服务端维护：

```text
RouteProfilePolicy
  profile_id
  public_model_id
  ordered_groups[]
  conflict_resolution
  cross_group_retry
  policy_version
```

客户端请求只携带服务端签发的 profile runtime credential 或不可伪造的 `profile_id`，服务端验证用户权限后，再把模型请求映射到允许的分组。客户端不能把任意 `group_id` 直接塞进请求头，否则用户可以绕过配置集权限和价格策略。

### 配置集的用户体验

默认提供一个系统预设配置集，用户可以：

1. 直接使用预设。
2. 复制预设并重新命名。
3. 添加或移除允许的分组。
4. 拖动分组优先级。
5. 查看统一模型列表。
6. 处理冲突模型。
7. 将配置集绑定到全部客户端、某个客户端，或某个项目 / 会话。

作用域优先级建议为：

```text
session > project > app > global
```

这样 Claude Code 可以使用“Claude Code 稳定”，OpenAI Responses 可以使用“低成本实验”，而不需要把 Provider 配置暴露给用户。

这一设计取代固定三档方案：价格只是配置集的一个属性，用户可以自由组合；模型 ID 冲突必须显式解决；系统预设可以持续更新；服务端仍然掌握真实分组、权限和计费。

## 十六、原 cc-switch 路由怎么保留和改造

cc-switch 现在实际上有三层路由，不能把它们混成一个概念：

1. **HTTP 入口路由**：`proxy/server.rs` 按本地请求路径进入 `handle_messages`、`handle_chat_completions`、`handle_responses`、`handle_gemini` 等 handler。它负责识别客户端协议和端点。
2. **ProviderRouter**：`proxy/provider_router.rs` 按当前 Provider、故障转移队列和熔断器选择一条 Provider。它解决的是“哪一家 Provider 可用”，本身不知道同一模型的分组倍率。
3. **ProviderAdapter / RequestForwarder**：根据 Provider 配置决定 base URL、鉴权方式、模型映射、Anthropic ↔ OpenAI ↔ Gemini 转换和流式响应转换。它解决的是“怎么把这次请求发给这一家”。

另外，`mode/stack` 是一个更高层的聚合功能：它把多家 Provider 的模型发布给客户端，再根据发布的模型 ID 固定到某一家。它适合原来的“用户自己添加多家 Provider”场景，但直接沿用会把 Provider、分组和模型暴露给用户，也不能自然表达“同一个模型在多个 AICLI 分组中按倍率选择”。

二开不需要重写协议转换器，建议这样改：

```text
保留：HTTP handlers
保留：ProviderAdapter 的协议转换能力
保留：RequestForwarder 的超时、SSE、熔断、usage、错误处理
复用：ProviderRouter 的熔断器和首包前故障转移思想
替换：ProviderRouter 的 Provider 选择来源
新增：AicliRouteResolver，返回模型级 RoutePlan
```

第一版可以让 `AicliRouteResolver` 在内存中生成受管的候选 Provider 视图，或者给 `RequestForwarder` 增加 `forward_with_route_plan`。这样底层已有的转换器可以继续使用，用户界面的 Provider CRUD 和第三方资源则完全移除。后续再把熔断器的 key 从：

```text
app_type:provider_id
```

升级为：

```text
profile_id:model_id:channel_id:runtime_credential_id:line_id:wire_protocol
```

避免一个分组故障时把同模型的其他分组一起熔断。

## 十七、怎样避免用户体验变复杂

路由复杂度必须留在后台。普通用户只需要看到三件事：绑定账户、选择模型、选择线路。分组、Key、协议和倍率在默认界面只作为状态信息显示，不要求用户手工配置。

建议的用户界面层级：

1. **首页用量统计**：账户额度、当前线路、模型可用数、同步状态。
2. **模型选择**：只显示统一模型名，例如 `claude-sonnet-4-6`、`gpt-6.1-sol`。
3. **模型策略**：默认“会话稳定优先”；高级用户可以改为“智能省钱”或固定 Route Profile。
4. **线路选择**：主线路、全球备用、亚太加速，显示简单的在线状态和延迟。
5. **高级详情**：展开后查看实际分组、倍率、Key 掩码、协议和最近错误。

为了避免自动切换带来意外费用，默认行为需要明确：

- 同一会话固定当前候选分组和 Key，不每条消息重新比价。
- 候选切换时在本地请求日志和首页显示“本次已切换到分组 X，倍率 Y”。
- 自动切换不会悄悄降低能力；不支持工具、thinking 或图片的候选直接排除。
- 模型不存在、能力不支持和额度不足分别显示不同错误，不能统一显示“请求失败”。
- 模型目录更新后保留旧目录缓存；正在运行的会话继续使用原 RoutePlan，新会话使用新目录。
- 客户端模型列表变更时提供“刷新模型目录”按钮，不强行修改用户当前模型。

这样用户的实际体验是“在 Claude Code 里选择一个模型即可”，而不是“先选供应商，再选协议，再选分组，再选 Key”。复杂路由仍然存在，但只在 AICLI Client 的高级详情和调试日志中可见。

## 十八、这套设计的体验风险和控制点

最大的风险不是协议转换本身，而是自动路由造成的不可解释行为。需要用四条规则控制：

- **价格可解释**：每次请求记录模型、分组、倍率和费用快照。
- **能力可解释**：候选目录声明 tools、thinking、vision、stream、context 等能力，过滤后再选择。
- **会话可复现**：同一 session 默认固定 RoutePlan，避免上下文和缓存漂移。
- **失败可恢复**：只在首包前切换候选；流式已经开始后只报告中断，不重复发送完整请求。

因此，路由层不会让用户体验变差；真正会导致体验变差的是把内部路由对象直接做成 Provider 列表。AICLI 版本应将原来的 Provider 列表降级为内部执行对象，把统一模型目录作为用户看到的主要入口。

## 十九、目前已经发现的关键问题

下面这些问题如果不先解决，直接进入大规模编码，后面很可能需要重做数据结构、路由层或发布方式。

### 1. Access Key 不应该直接变成“取回所有明文 Key”的总钥匙

当前设计用 `accessKey` 调用 `/api/token/:id/key`，再把所有分组 Token 拉到本地。这虽然能实现多分组路由，但权限很大：桌面客户端一旦被调试、恶意插件或同机进程读取，就可能一次性暴露用户所有 API Key。

更安全的服务端方案是增加 AICLI 专用的会话接口：

```text
accessKey
  -> POST /api/client/session
  -> 返回短期、可撤销、只允许指定模型 / 分组 / 线路的 runtime credential
```

客户端只保存短期 runtime credential，不保存账户全部 Token 明文。服务端仍然负责真正的分组选择和撤销。若第一版暂时必须拉取明文 Key，至少要加：Keychain 存储、内存短驻留、按需获取、禁止批量获取、撤销绑定、应用退出清理和完整的本地日志脱敏。

### 2. 深链中直接携带 Access Key 有泄漏面

`aicliapi://v1/bind?apiKey=...&accessKey=...` 会出现在操作系统深链日志、启动参数、崩溃上下文、剪贴板历史或用户分享的链接中。建议正式方案改成短期一次性绑定票据：

```text
aicliapi://v1/bind?ticket=<short-lived-one-time-code>
```

如果必须兼容当前格式，也应该优先支持 `ticket`，对 `apiKey/accessKey` 只作为过渡格式，并在绑定成功后立即作废。不能把原始密钥写进普通数据库、前端状态快照或错误提示。

### 3. “同一模型跨协议”不代表能力完全等价

Claude Code 使用 GPT 模型需要 Anthropic ↔ OpenAI 转换，但下列能力可能无法完全等价：thinking、tool choice、并行工具调用、prompt cache、图片块、computer use、上下文窗口、停止原因和流式事件顺序。

因此模型目录必须有兼容级别，而不是只有一个 `supported=true`：

```text
full       文本、工具、流式、thinking 均通过验证
partial    文本和流式可用，部分工具或 thinking 有限制
text_only  只保证普通文本
unavailable 当前客户端协议不可用
```

Claude Code 中的 GPT 模型不能默认伪装成完整 Claude 能力。第一版应先用真实 Claude Code 版本完成一组端到端验收，再决定哪些模型显示在模型选择器中。未知模型能否被 Claude Code 正确选择，也不能只靠推测，必须做实际客户端测试。

### 4. 价格优先不一定适合作为默认策略

同一个模型的不同分组可能在延迟、额度、上下文、工具能力和稳定性上不同。默认每次选择最低倍率，可能出现价格下降但响应变慢、缓存失效或能力下降的情况。

我建议把默认策略改为“会话稳定优先”，在同一会话内固定候选；“智能省钱”作为用户主动选择的策略。无论使用哪种策略，都必须在用量页展示实际分组和倍率。否则用户看到的只是模型名称，无法解释账单变化。

### 5. Cloudflare 的 120 秒限制无法被客户端重试真正解决

如果请求已经向客户端输出过内容，120 秒断联后不能安全地从另一条线路重新发送完整请求。工具调用、扣费、上下文和模型状态都可能已经发生。客户端只能：

- 首包前失败时换线路。
- 首包后断开时报告中断，让用户重新发起。
- 长流式请求优先使用 CDN 线路，必要时在发送前提示用户切换。

不能把“自动故障转移”宣传成所有长请求都能无感恢复。主线路可以继续是默认线路，但流式请求应根据类型自动降低 Cloudflare 线路优先级。

### 6. 远端额度、倍率和本地统计不是同一个口径

new-api 的 `quota` 通常是站点额度单位，不天然等于人民币或美元；本地 UsageDashboard 记录的是客户端会话和 Token 用量；模型价格目录又是估算值。三者混在一张“余额”卡片中会误导用户。

首页必须分开标注：

- 服务端账户额度：以服务端返回为准。
- Token 剩余额度：以对应 Key 的接口为准。
- 本地请求统计：只代表本机记录。
- 估算费用：基于模型价格和分组倍率，不作为最终账单。

### 7. 直接删除旧 Provider 需要限定删除范围

用户不需要备份旧的第三方中转 Provider，但不能在迁移中无条件删除所有 Provider 行。cc-switch 的 Provider 数据还可能承载官方 OAuth、客户端登录状态、Claude Desktop 配置或其他原有客户端的必要状态。

迁移前必须定义可删除集合：

```text
删除：第三方中转 Provider、旧预设、旧线路、推广资源
保留或单独迁移：官方登录账户、OAuth 状态、客户端本地配置、会话和历史记录
创建：每个受支持客户端的受管 AICLI 路由
```

如果产品决定连官方账户也全部移除，需要明确写入迁移说明，并对客户端配置做可恢复的重建，而不是只删数据库行。

### 8. “私有仓库”不等于“私有分发”

当前参考快照中，cc-switch 和 Magpie 是 MIT；Maskit 和 new-api v1.0.0-rc.34 是 AGPLv3。把 Maskit 源码直接复制进 MIT 客户端、把 new-api 代码编进客户端，都会改变发布时的许可证和源码提供边界，不能只因为仓库是私有的就忽略。更稳妥的实现是保留 Maskit 为独立本地进程或独立组件，通过明确的本地接口调用，并在发布前由项目负责人确认是否对外分发、对应源码和 NOTICE 的处理方式。

### 9. 三条线路的后端契约要做真实一致性测试

本次已实际请求两条线路：`https://aicliapi.com/api/pricing` 和 `https://aicli.qzz.io/api/pricing` 均返回 HTTP 200、69 个模型、同一个 `pricing_version=a42d372ccf0b5dd13ecf71203521f9d2`，响应 SHA-256 也一致。`https://api.aicli.qzz.io/api/pricing` 在当前环境发生 TLS 握手失败，不能把它记为已验证成功；这可能是证书、CDN TLS 配置或当前网络路径问题，需要在发布前从实际用户网络和服务器侧分别验证。

即使三条线路最终返回相同价格目录，客户端仍应分别测试：TLS、`/api/status`、绑定接口、`/v1/models`、Anthropic Messages、OpenAI Responses、OpenAI Chat、Gemini、SSE 长连接和错误格式。不能因为 DNS 或首页能打开，就假设所有协议路径和超时行为一致。

### 10. 需要先做三个小型 PoC，而不是立即改完整 Provider 系统

建议先完成以下三个垂直切片：

1. **Claude Code → gpt-6.1-sol**：真实模型发现、Anthropic 请求转 OpenAI、工具调用、thinking、SSE 响应和错误处理。
2. **同模型双分组**：两个不同倍率分组，验证会话固定、费用记录、Key 失效切换和服务端实际扣费。
3. **Maskit 流式链路**：包含普通文本、工具调用、thinking、SSE 分块、客户端取消和引擎故障恢复。

这三个 PoC 通过后，再批量替换 Provider UI、数据迁移和所有客户端配置。否则一旦发现某个协议无法等价转换，前面的统一模型目录和 UI 都要返工。

### 11. 计费以网站日志为准，本地只做即时状态和预估

本地重复计算费用一定可能和服务端不一致。原因包括服务端最终收到的 Token 数、thinking / cache Token、协议转换后的 usage、重试与预扣、退款、分组倍率实时变化、计费表达式版本、舍入规则，以及服务端是否记录消费日志。客户端不能把本地估算值当成扣费结果。

new-api v1.0.0-rc.34 已有可复用的用户侧接口：

```text
/api/log/self
    用户明细日志，需要 UserAuth（Dashboard session 或 PAT）

/api/log/self/stat
    用户聚合统计，需要 UserAuth，返回 quota / rpm / tpm

/api/log/token
    当前 API Token 的最近日志，需要 relay API Key，只读，返回数量受 MaxRecentItems 限制

/api/usage/token
    当前 API Token 的已用 / 剩余额度，需要 relay API Key

/dashboard/billing/usage
    兼容 Dashboard Billing 的累计已用额度，需要 relay API Key
```

客户端应直接调用这些 JSON API，不抓取网站页面。首页显示服务端余额、服务端已用额度和最近消费日志；本地只保存请求状态、请求 ID、线路、模型、分组、延迟、错误和可选的预估费用。

请求完成后，客户端读取响应头中的 `X-Oneapi-Request-Id`，用用户级日志接口按 `request_id` 查询最终 `quota`。日志写入有短暂延迟，应采用 1 秒、3 秒、10 秒的有限重试；查询不到时显示“等待服务端记账”，不能用本地估算值覆盖服务端账单。服务端日志可能因 `LogConsumeEnabled` 关闭、日志清理或存储故障而缺失，所以余额接口仍然是总额校验来源。

为了聚合多个分组和多个 Token，绑定流程应优先使用带只读用量权限的 AICLI 客户端会话。单个 relay API Key 只能查询该 Token 的日志，不能代表用户全部分组。不要在桌面端使用拥有账户管理能力的长期 Dashboard PAT；正式服务端应提供短期、可撤销、只读的 usage / pricing session。
