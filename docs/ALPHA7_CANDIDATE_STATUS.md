# Alpha.7 候选收口状态

核对日期：2026-10-21（Asia/Shanghai）。对应三个独立任务中的 **任务 1**，不将其作为真实 Agent 或无开发工具 VM 验收。

## 已核对的远端基线

- 仓库：`qingtian-4/TaskPulse`，默认分支 `main`。
- PR：[#8](https://github.com/qingtian-4/TaskPulse/pull/8)，`codex/desktop-discovery-visibility → main`，仍 open，未合并。
- 已验 CI 的候选源 SHA：`d991f2e51720afe7a81c72d8542f8fdb068128ed`。
- main：`7a1929aaf1dfb98f425d2868c251865e184fefd5`，尚不包含本次 Alpha.7 功能与 UI。
- [Windows quality](https://github.com/qingtian-4/TaskPulse/actions/runs/39608987901) 与 [Security checks](https://github.com/qingtian-4/TaskPulse/actions/runs/39608987889) 的 PR 运行均成功，head_sha 与上述候选一致。另一次同 SHA push quality 运行被取消，不能单独把 cancelled 说成成功；验收依据是已成功的 PR 运行。
- PR API 快照为 `mergeable=true`、`mergeable_state=unstable`，不是无条件可合并的 clean 状态；不能仅凭没有文本冲突或某个绿色运行绕过最新检查/审阅要求。新候选准备合并时需重新核对。
- `package.json`、Cargo.toml、Cargo.lock、tauri.conf.json 版本均为 `0.2.0-alpha.7`，MSI 数字版本为 `0.2.0.1007`。
- 未发现 `v0.2.0-alpha.7` 远端标签或 Alpha.7 Release。连接器读取到的旧 Alpha.6 仍为 draft / prerelease，不替代新候选安装器验收。

以上是核对时的快照，不是未来分支状态。收口前 PR 的旧正文仍引用更早的 `7493638` 和历史测试数量，不能拿它替代当前 SHA 的 Actions 结果；本轮授权后将更新正文。

## 本次收口修改

- 中英文 README 显式克隆候选分支，避免读者落入旧 main；安装指南同步。
- 新增回归先复现了默认分支命令与 CI 未运行文档检查的问题，再修复；quality/release 各自以独立步骤运行 `test:docs`。
- 更新 Alpha.7 CHANGELOG 和 Release notes，包含现有接入、可靠性、UI 的实际交付，同时保留 fixture-tested 和未验收限制。
- 本次修改会产生新的提交 SHA，**上面的 d991f2e 绿色 CI 不覆盖它**。用 `git rev-parse HEAD` 取得本次提交，在获推送授权后触发 CI，并在 PR 记录准确的新 head 与成功运行链接；此前不能称“最终 CI 已完成”。

本机已运行 `scripts/verify.ps1 -Frontend -TargetDir src-tauri/target`，退出码为 0：前端 Check/Build、fmt/clippy、Rust 390 通过 / 12 忽略、UI Model 28 通过、文档 5 通过、Release 8 通过、安全脚本 6 通过、接入契约 3 通过；版本一致性与前端 moderate+ 审计门禁通过，已知 low 告警仍保留。真实 EXE bridge smoke 通过，证据位于 `src-tauri/target/integration-bridge-F7LQcv`，未改宿主配置；这不是实际 Agent 宿主端到端验收。全历史脱敏 secrets 扫描为 0 发现，提交后仍需复跑。正式候选仍要求 [RELEASING.md](RELEASING.md) 的完整质量、安全与原生门禁，不将本机结果代替远端 CI 或安装器验收。

## 后续动作与确认边界

1. 获授权后，仅推送当前候选分支、更新 PR #8 说明并等待本次确切 SHA 的质量/安全 CI；不直接修改 main。
2. 维护者明确确认 PR 和 head SHA 后，使用保留历史的 merge commit 合入 main。重新读取实际 merge SHA，确认 main CI，不能提前猜 merge SHA。
3. 对维护者确认的 main SHA 创建 `v0.2.0-alpha.7`，不覆盖既有标签；按现有工作流构建 NSIS/MSI，各自独立 runner 验收成功后才生成 **Draft pre-release**。
4. 不公开发布；真实 Agent、无 Rust/Bun Windows VM、升级/格式迁移与小范围试用仍为独立任务。没有对应证据就保持未验收。

README 候选分支说明和防回归断言只适用于未合并阶段；合并 main 后须作为单独审阅变更更新默认分支说明，不能提前让读者克隆旧 main。

维护者已明确授权本轮候选分支 push 与 PR #8 正文更新，仅用于运行并确认 CI；这不包含 merge、Tag 或 Release。实际远端结果与确切新 SHA 以 PR 记录为准。未修改真实 Hook、Agent、代理/模型、审批策略、用户配置或持久状态。
