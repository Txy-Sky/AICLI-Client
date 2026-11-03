# AICLI Client

AICLI Client 是面向 AI 编程客户端的桌面应用，使用 Tauri 2、Rust、React 和 TypeScript。
当前分支正在按实施清单改造，尚未完成全部功能和真实客户端验收；不应把本分支当作正式发布版。

应用使用独立目录 `~/.aicli-client`，本地代理默认端口为 `127.0.0.1:15821`。
路线、账户绑定、隐私处理及客户端接管的目标和进度，分别见
[改造方案](docs/aicli-client-redesign-plan-v2-zh.md)与[实施清单](docs/aicli-client-implementation-checklist-zh.md)。

## 开发

版本以 `.node-version`、`package.json` 的 `packageManager` 和 `rust-toolchain.toml` 为准。
本分支完整构建及测试在 GitHub Actions 的 `AICLI Acceptance` 中执行，规则见清单第 0.6 节。

```bash
# 安装依赖（CI 或独立开发环境）
pnpm install --frozen-lockfile
# 开发与不打包的调试构建（须使用临时 HOME）
pnpm tauri dev
pnpm tauri build --debug --no-bundle
# 完整验收，脚本自行隔离 HOME
bash scripts/aicli-check.sh
# 每次推送前本机静态检查，不编译
bash scripts/aicli-pre-push-check.sh
```

本机无完整依赖时，可在仓库外安装锁文件版本的 Prettier，通过 `AICLI_PRETTIER_BIN` 指定；
另需 rustfmt 和 Python/PyYAML。不要用真实客户端配置目录运行测试。

问题反馈见 [SUPPORT.md](SUPPORT.md)，贡献流程见 [CONTRIBUTING.md](CONTRIBUTING.md)，
敏感漏洞报告见 [SECURITY.md](SECURITY.md)。不要在 Issue、日志或提交中粘贴真实令牌。

## 上游与许可证

基于 farion1231/cc-switch，MIT。保留上游作者版权声明；本项目修改的版权声明见 [LICENSE](LICENSE)。
