# Alpha.7 候选收口状态

核对日期：2026-10-22（Asia/Shanghai）。第 1 项文档合并与第 2 项候选安装包已完成；第 3 项独立实机验收按维护者要求暂缓，不计为通过。Alpha.7 保持 Draft，未公开。

## 已合并的源码基线

- **PR #8 已合并**：[#8](https://github.com/qingtian-4/TaskPulse/pull/8)，使用 merge commit 保留上游和本项目完整历史。
- 原 PR head：`2cc13b1f11049047fdf6f868ca92b3d5df8aefce`；实际合并 SHA：`ea21c85fc6b81992de72cf45729cfe6abcc1c15f`，已进入默认 `main`。
- 原 PR 的 [Windows quality（重跑 attempt 2）](https://github.com/qingtian-4/TaskPulse/actions/runs/39617437276/attempts/2) 和 [Security checks](https://github.com/qingtian-4/TaskPulse/actions/runs/39617437161) 成功，针对原 PR head，不冒充 main 检查。
- 合并后的确切 `ea21c85`：[Windows quality](https://github.com/qingtian-4/TaskPulse/actions/runs/39625854499) 与 [Security checks](https://github.com/qingtian-4/TaskPulse/actions/runs/39625854268) 均成功。此前延迟未生成运行的状态已过时。
- **PR #9 已合并**：[#9](https://github.com/qingtian-4/TaskPulse/pull/9)，文档修复 head 为 `71ed60bcf89ca9fb5fece29794d35452c5245db8`；实际 main 合并提交和 Alpha.7 唯一源 SHA 为 `b5dc64a760bae916c9d40b147616f9720405887b`。
- 最终 `b5dc64a` 自身的 [Windows quality](https://github.com/qingtian-4/TaskPulse/actions/runs/39633895368) 与 [Security checks](https://github.com/qingtian-4/TaskPulse/actions/runs/39633895341) 均成功，不是复用旧 PR 或旧 main 的检查结果。
- 四个版本文件仍一致为 `0.2.0-alpha.7`，MSI 数字版本为 `0.2.0.1007`。没有为了文档修复变更版本或生命周期逻辑。

维护者已明确确认在 `b5dc64a` 创建并推送不可变标签 `v0.2.0-alpha.7`。后续文档核对提交不改变这个安装包源 SHA，不能移动标签把它们算进既有安装器。

## 当前安装包与证据边界

**Alpha.7 安装器 CI 已通过，独立实机验收未完成。** [发布运行 39636404358](https://github.com/qingtian-4/TaskPulse/actions/runs/39636404358) 成功生成 [Draft pre-release](https://github.com/qingtian-4/TaskPulse/releases/tag/untagged-c38492295dd893cb1dbc)，ID `419811221`；核对时 `draft=true`、`prerelease=true`，没有公开下载。构建、NSIS 和 MSI 各自独立 runner 上的安装验收均已完成；不是复用 Alpha.6 或 build-tree EXE 的证据。

| 环节 | 本轮记录与限制 |
|---|---|
| 源码运行 | 从默认 `main` 克隆；运行 `git rev-parse HEAD` 核对实际版本提交 |
| 质量 / 安全 CI | 候选 `b5dc64a` 的实际 main 检查和标签发布门禁通过 |
| Alpha.7 NSIS / MSI | MSVC 安装包已生成；独立 CI 安装、版本、内嵌 UI/IPC、重启、同版本重装和卸载通过 |
| 资产核对 | 两种安装器、manifest 和校验文件共四项与 Draft asset digest 一致；签名均为 `NotSigned` |
| 干净 Windows 验收 | 暂缓，不计为通过；CI Windows Server runner 不是无 Rust/Bun 的 Windows 10/11 VM |
| 真实 Agent 端到端 | 暂缓，不计为通过；没有新增真实任务、焦点、重启通知去重或 P50/P95 样本 |
| 跨版本 / 安装格式迁移 | 未验证；同版本重装和状态标记保留不等于完整配置迁移 |
| 公开 Alpha | 未公开；仍需维护者单独确认，不能因第 3 项暂缓而自动跳过人工门禁 |

完整资产 SHA256、实际 runner 版本、安装验收字段和本机下载路径见[候选安装包验收记录](ALPHA7_CANDIDATE_VALIDATION_2026-10-22.md)。安装包未加入 Git，也未在开发机执行安装或卸载。

既有本机验证包含 Rust 390 通过 / 12 opt-in 忽略、UI Model 28 通过、Check/Build、fmt/clippy 与隔离 EXE bridge smoke；前一轮原生 WebView2 渲染证据见[视觉验收](VISUAL_POLISH_VALIDATION_2026-10-21.md)。这些不等于宿主实测，不能提升 Kimi/TRAE/ZCode/WorkBuddy 的支持等级。已知前端 low 与 Rust 维护/unsound 告警保留在[安全记录](internal/reports/SECURITY_AUDIT_2026-10.md)，审计阈值通过不等于零风险。

## 维护者发布决策边界

1. 候选身份、资产和 CI 已核对，不能因此宣称所有 Agent、干净 Windows、升级或长期可靠性已验收。
2. 独立实机与真实 Agent 验收暂缓；若恢复，须针对同一安装包记录宿主/系统版本、预期与实际状态、事件到提醒延迟及误报/漏报。无意义的付费压测不属于验收。
3. [发布流程](RELEASING.md)的未完成门禁保持未完成。当前建议保留 Draft，公开发布仍需维护者单独确认；创建候选草稿的授权不包含 Publish。

本次仅同步交付文档和回归测试；不移动标签、不修改远端草稿或资产、不公开发布，不安装真实 Hook，不修改 Agent、代理/模型、审批策略或用户持久状态。
