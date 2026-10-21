# Alpha.7 候选收口状态

核对日期：2026-10-22（Asia/Shanghai）。这是源码与发布门禁记录，不是所有 Agent 或干净 Windows VM 的验收证明。

## 已合并的源码基线

- **PR #8 已合并**：[#8](https://github.com/qingtian-4/TaskPulse/pull/8)，使用 merge commit 保留上游和本项目完整历史。
- 原 PR head：`2cc13b1f11049047fdf6f868ca92b3d5df8aefce`；实际合并 SHA：`ea21c85fc6b81992de72cf45729cfe6abcc1c15f`，已进入默认 `main`。
- 原 PR 的 [Windows quality（重跑 attempt 2）](https://github.com/qingtian-4/TaskPulse/actions/runs/39617437276/attempts/2) 和 [Security checks](https://github.com/qingtian-4/TaskPulse/actions/runs/39617437161) 成功，针对原 PR head，不冒充 main 检查。
- 合并后的确切 `ea21c85`：[Windows quality](https://github.com/qingtian-4/TaskPulse/actions/runs/39625854499) 与 [Security checks](https://github.com/qingtian-4/TaskPulse/actions/runs/39625854268) 均成功。此前延迟未生成运行的状态已过时。
- 四个版本文件仍一致为 `0.2.0-alpha.7`，MSI 数字版本为 `0.2.0.1007`。没有为了文档修复变更版本或生命周期逻辑。

以上 SHA 和 CI 是已验证的历史基线，不是未来最终候选的占位符。本轮从该 main 创建短期 `codex/alpha7-postmerge-docs` 分支，经新的 PR 修复合并后的 README、安装指南与文档回归。**本轮修复的 PR head 和新 main SHA 必须分别核对自身 CI**，不能复用上述绿色结果。确切 SHA、PR 和最终运行链接记录在对应 PR 中，避免在同一提交中自引用尚未产生的 SHA。

## 当前安装包与证据边界

**安装器与实机验收仍未完成**。核对时未发现 `v0.2.0-alpha.7` 远端标签或 Alpha.7 Release；仅有旧 Alpha.6 Draft pre-release。旧包、源码构建或合成事件均不能替代新候选安装器与真实宿主验收。

| 环节 | 本轮记录与限制 |
|---|---|
| 源码运行 | 从默认 `main` 克隆；运行 `git rev-parse HEAD` 核对实际版本提交 |
| 质量 / 安全 CI | 原 PR 与已合并基线通过；每个新的 PR / main SHA 仍需独立确认 |
| Alpha.7 NSIS / MSI | 尚未生成；需维护者单独确认最终 main SHA 后才能创建标签 |
| 干净 Windows 验收 | 未完成；需无 Rust/Bun 环境的首次安装、运行、升级、重启与卸载 |
| 真实 Agent 端到端 | 仍需 Codex 与另一款已安装 Agent 的新任务、工具、明确结束、通知、重启去重与窗口焦点证据 |
| 公开 Alpha | 未发布；须维护者核对 SHA256、版本、残余风险和真实支持范围后手动决定 |

既有本机验证包含 Rust 390 通过 / 12 opt-in 忽略、UI Model 28 通过、Check/Build、fmt/clippy 与隔离 EXE bridge smoke；前一轮原生 WebView2 渲染证据见[视觉验收](VISUAL_POLISH_VALIDATION_2026-10-21.md)。这些不等于宿主实测，不能提升 Kimi/TRAE/ZCode/WorkBuddy 的支持等级。已知前端 low 与 Rust 维护/unsound 告警保留在[安全记录](SECURITY_AUDIT_2026-10.md)，审计阈值通过不等于零风险。

## 下一步及确认边界

1. 本轮只更新合并后文档、测试，经新 PR 合并，并确认实际新 main SHA 的质量与安全 CI；保留无关修改及运行中的 Agent。
2. **维护者单独确认**最终 main SHA 后，才创建不可变的 `v0.2.0-alpha.7`；[现有发布流程](RELEASING.md) 执行版本预检、安全审计、MSVC 构建及 NSIS/MSI 各自独立 Windows runner 验收。全部通过才生成 **Draft pre-release**，不会公开发布。
3. 对同一候选安装器补独立 Windows 与真实 Agent 验收，记录宿主/系统版本、实际事件与状态、事件到通知延迟、误报/漏报及未验证项；无意义的付费压测不属于验收。
4. 核对资产 SHA256、版本、风险、支持矩阵和发布说明后，才由维护者手动决定 Publish pre-release。没有证据的门禁保持未完成。

本轮不打 Tag、不创建 Release、不公开发布，不安装真实 Hook，不修改 Agent、代理/模型、审批策略或用户持久状态。
