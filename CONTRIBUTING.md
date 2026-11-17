# 贡献 AICLI Client

开发依据为 [实施清单](docs/aicli-client-implementation-checklist-zh.md)，方案用于说明原因。
两者冲突时以清单为准，并记录偏差。版本与开发命令见 [README](README.md)。

1. 在 `feat/aicli-client-v1` 上按编号执行；不要合并到 main、发布或部署。
2. 每项分别提交实现，使用 `<type>(<scope>): <项目编号> <说明>`。
3. 每次推送前运行 `bash scripts/aicli-pre-push-check.sh`，全部通过后再推送。
4. 完整验收由 GitHub Actions 执行。成功后另提交清单及真实证据；并行和合并验收按清单第 0.6 节执行。
5. 仅暂存明确路径，禁止 `git add .` 和 `git add -A`。不要改未跟踪的仓库副本。

测试必须使用临时 HOME，不能改动开发机真实的客户端配置。
真实密钥只从环境变量读取，不写进代码、日志、证据或提交。
不修改 new-api、不直接读写其数据库、不新增服务端、不碰生产环境。
人工项与待负责人决定的范围按清单处理，不为通过检查放宽断言。

反馈及讨论请在本仓库提交 Issue；安全问题按 [SECURITY.md](SECURITY.md) 私下报告。
