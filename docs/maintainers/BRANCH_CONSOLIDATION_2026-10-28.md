# 分支整合审计（2026-10-28）

核对基线：远端 `main = 0bcecd8f8f77df35f7d198ec85a5c57412c14ba2`，
原工作分支 `codex/p0-remote-acceptance = 2e8aebb`。两者文件树完全一致；
[PR #26](https://github.com/qingtian-4/TaskPulse/pull/26) 已合入该分支全部 11 个提交。
其 Windows quality / Security checks 均成功，核对时没有打开的 PR。

## 依赖与合入状态

下表是清理前的引用快照；“已包含”均通过提交祖先关系核对，不能仅凭分支名判断。
同一分支曾承载多轮工作时，以当前 tip 是否可从 main 到达为准。

| 分支或分组 | 顶端/证据 | 依赖与处理 |
|---|---|---|
| `feat/taskpulse-mvp` | `2c37a96` | 早期基础，已进入 reliability 分支并被 main 包含 |
| `codex/reliability-ui` | `108712a`，PR #1 | 可靠性与界面基础，已包含 |
| `codex/alpha7-postmerge-docs` | 远端 `71ed60b`；本地 `b5dc64a`，PR #9 | Alpha.7 文档，两个 tip 均已包含 |
| `docs/open-source-showcase-20261023` | `eed1655` | 开源展示资料，已包含 |
| `codex/r02-state-preservation` | `17af90d`，PR #16 | 状态保护基础，已包含 |
| `codex/r03-worker-generation` / `codex/r03-merge-accepted` | `e2c93cd` / `342d60a`，PR #17 | 在 R02 上处理 worker 世代，两个 tip 均已包含 |
| `codex/r04-private-diagnostics` / `codex/r04-merge-accepted` | `b58f22b` / `1709de1`，PR #18 | 在 R02/R03 上处理诊断隐私，两个 tip 均已包含 |
| `codex/r02-r04-acceptance` | `14e1742` | R02–R04 的联合验收，随后进入消息中心，已包含 |
| `codex/dashboard-message-center` | 远端 `781113b`；本地 `7c58efb`，PR #20/#21 | 消息中心与原生提醒验证，两个 tip 均已包含 |
| `codex/desktop-discovery-visibility` | 本地 `f27dbfe`；远端 `87a72a1`，PR #21 | 集成发现、消息中心与运行时修复，两个 tip 均已包含 |
| `codex/issue22-installer-ipc` | `74da425`，PR #23 | 安装器 IPC 验收修复，已包含 |
| `codex/fix-hermes-native-dedup` | `8e00866`，PR #24 | Hermes 去重与真实来源验收，已包含 |
| `codex/p0-remote-acceptance` | `2e8aebb`，PR #25/#26 | 扫描器、Alpha.8、MCP、macOS、Logo/README 与豆包实验改动，已包含 |

这些分支没有待合入的独有提交，应核对最新 tip 后直接清理，不再生成重复功能 PR。
依赖顺序已经由上述合并历史保留，删除分支引用不会删除 main 可到达的提交。

## 本地 main 分叉的处理

本地 `main = 27b88b4` 比远端少 31 个提交、另有 5 个独有提交：

| 本地提交 | 远端对应提交 | 结论 |
|---|---|---|
| `3520e47` | `93aff2f` | patch 等价的 CIM 扫描器修复 |
| `d6a804c` | `050ae59` | patch 等价的多来源去重/快照修复 |
| `2040fc9` | `234486e` | patch 等价的失败保留修复 |
| `27b88b4` | `e9a4e42` | patch 等价的输出上限修复 |
| `6528468` | 无完整等价提交 | 3 份历史审查资料和本地开发构建体积配置 |

从最新 `origin/main` 建立整合分支，将本地 `main` 作为合并父提交保留。重复的
扫描器代码以及较旧的测试/Cargo 冲突均按远端已验证实现处理，不把旧逻辑再加一次。
旧提交的全局 `[profile.dev] debug = 0` 不带回：保留 main 默认调试信息与构建行为，
本机磁盘优化可单独使用 Cargo 环境覆盖。发布构建参数保持 main 原值。

独有资料转入历史归档，并加上日期、适用提交和非当前授权说明：

- [通用接入原任务书](../internal/reports/AGENT_INTEGRATION_EXECUTION_PLAN_2026-10-20.md)
- [2026-10-21 实施审查](../internal/reports/AGENT_INTEGRATION_REVIEW_2026-10-21.md)
- [2026-10 早期审查交接](../internal/reports/REVIEW_HANDOFF_2026-10.md)

整合后的生产代码、依赖清单、Logo、截图及中英文 README 应与 `0bcecd8` 完全一致。
本次新增内容仅为历史归档、此审计记录与 [Git 工作流程](../GIT_WORKFLOW.md) / AGENTS 规则。
后续功能从最新 main 建立短期分支，完成验证后经 PR 合并并删除分支。

## 清理与验证边界

清理前重新 fetch，确认待删除 tip 已包含于远端 main。旧 worktree 要检查路径、
工作区状态及占用；有未提交文件或有用的未跟踪资料时保留，不强制清除。
本地 main 在上述合并被接受后即可快进到远端，避免 reset 丢失本地历史。

此次检查用于整合源码和正常启动，不创建版本标签、安装包或 Release。
豆包后续适配暂停；WorkBuddy / CodeBuddy 等新宿主留待独立功能分支核验，
不因分支清理而扩大兼容性声明。

## 整合版本本地验证

- `scripts/verify.ps1 -Frontend` 成功：497 项 Rust 测试通过、24 项外部环境用例
  忽略；fmt、Clippy、Svelte 检查、UI 模型、文档/仓库审计、前端构建及原生
  bridge / MCP 隔离测试通过。
- `native-product-smoke.ts` 的 12 组原生 WebView2 检查通过，覆盖任务列表、搜索、
  查看/固定、管理窗口、MCP 配置导出、明确请求/解除、结束去重、27 条任务滚动、
  窗口边界和 DPR 1/1.25/1.5/2。5 种托盘图标的原生写入成功。
- 测试使用隔离配置和公开合成事件，不更改真实宿主配置。没有验收系统托盘实际像素、
  操作系统混合 DPI 切屏、外部编辑器焦点、真实新 Agent 生命周期或新安装包。
- `audit:rust` 通过，无 vulnerability，registry 检查完整；公告库
  `99670cfc0bc574433034b75876a4b791ccaa2efb`。仍报告 10 项 unmaintained、
  2 项 unsound 警告（glib 0.18.5、rand 0.7.3），保留后续依赖审查。
- `audit:secrets` 扫描本地全部可达历史，通过且无发现。完整 `bun audit` 报告
  4 项 low（SvelteKit 2 项、cookie 1 项、deepmerge 1 项），中风险以上门槛通过；
  阈值通过不代表没有风险。本次没有修改锁文件或通过忽略项压掉告警。

本地证据保存在 ignored 的 `src-tauri/target/branch-consolidation-20261028/`，
原生结果为 `src-tauri/target/ui-smoke/evidence/product-native-2026-10-28T07-15-23-386Z-6lfgla/result.json`。
PR 合并还须等待当前 head 的远端质量与安全检查完成，不能用前一轮 PR 的结果替代。
