# Alpha.7 候选安装包验收记录

日期：2026-10-22（Asia/Shanghai）。本记录交付第 2 项“候选安装包”；第 3 项独立实机验收已按维护者要求暂不执行，不计为通过。尚未公开发布。

## 候选身份

- Tag：`v0.2.0-alpha.7`，经维护者明确确认后创建并推送，未移动或覆盖既有标签。
- Annotated tag 对象：`e438c8aeb39a3e4eef349123790f22ebb92123a7`。
- 唯一源提交：`b5dc64a760bae916c9d40b147616f9720405887b`，即 [PR #9](https://github.com/qingtian-4/TaskPulse/pull/9) 的实际 main 合并提交。
- 该 main 的 [Windows quality](https://github.com/qingtian-4/TaskPulse/actions/runs/39633895368) 和 [Security checks](https://github.com/qingtian-4/TaskPulse/actions/runs/39633895341) 均通过。
- 应用版本：`0.2.0-alpha.7`；MSI ProductVersion：`0.2.0.1007`，已通过本机只读 MSI 元数据查询复核，没有在开发机执行安装器。
- [Draft pre-release](https://github.com/qingtian-4/TaskPulse/releases/tag/untagged-c38492295dd893cb1dbc) ID `419811221`：核验时 `draft=true`、`prerelease=true`、target 为上述确切源提交。草稿仅维护者可见，不是公开可下载版本。

## 工作流与实际安装验收

[Windows Alpha release 39636404358](https://github.com/qingtian-4/TaskPulse/actions/runs/39636404358)，attempt 1，2026-10-22 00:37:17—00:49:04（北京时间），最终 `completed / success`。

全部门禁均成功：版本预检、完整历史密钥扫描、Rust 审计、MSVC 质量检查与 NSIS/MSI 构建、实际 GUI-subsystem bridge smoke、两种安装器独立验收、最终资产校验与 Draft 生成。没有手动跳过失败步骤或直接创建草稿。

| 安装器 | 独立安装验收 Job | 实际宿主（非无开发工具 VM） | 结果 |
|---|---|---|---|
| NSIS | [119353335776](https://github.com/qingtian-4/TaskPulse/actions/runs/39636404358/job/119353335776) | Windows Server 2025 / 10.0.26100；`windows-2025-vs2026` image `20261007.264.1` | `phase=passed`、error=null |
| MSI | [119353335802](https://github.com/qingtian-4/TaskPulse/actions/runs/39636404358/job/119353335802) | Windows Server 2025 / 10.0.26100；`windows-2025-vs2026` image `20261018.275.1` | `phase=passed`、error=null |

两份下载后的 `result.json` 均确认：

- 真实安装后的应用版本为 `0.2.0-alpha.7`，不是 build-tree EXE 冒充安装版。
- 内嵌前端与原生 IPC 可用；首次启动 `onboarding_complete=false`、tasks=0、prompts=0，没有自动确认来源或导入历史任务。
- 普通用户权限下首次启动和重启成功；同版本重装、卸载成功，用户状态标记保留。
- 两种安装包签名均为 `NotSigned`，本机只读签名检查结果一致。

重启证据仅覆盖初始空状态下的安装版启动/IPC，不是实际 Agent 通知重启去重证明。同版本重装不是跨版本升级；状态标记保留也不是完整监控配置迁移证明。

## 已下载并复核的资产

安装包、manifest 和校验文件位于本机 Git 忽略目录：

`src-tauri/target/alpha7-candidate-2026-10-22/windows-installers/`

```text
168bf000ec122a35d66d511daaf344c7335aa6c38fd5f6589b807bc1dba989c9  TaskPulse_0.2.0-alpha.7_windows_x64_setup.exe
df54765e42a4e9aba9748896998d62dab415b094ba8a7c31c90aa1bb80f9db4f  TaskPulse_0.2.0-alpha.7_windows_x64.msi
```

本机 `scripts/release.ts verify` 通过；两种安装器及 `release-manifest.json`、`SHA256SUMS.txt` 的大小与完整 SHA256，均与 Draft 的四个 GitHub asset digest 一致。manifest 的源提交、tag、平台、架构及 MSI 版本与本记录一致。校验值不等于发布者签名，不应为安装关闭系统安全保护。

三个下载的 Actions ZIP 亦先按 GitHub artifact digest 校验，再解压：

| Artifact | ID | SHA256 |
|---|---|---|
| windows-installers | 12363156343 | `2675ed699b1f64a468e6bbdca4e36153ce7c355b47bc2b8363a63899834f4db5` |
| windows-installer-evidence-nsis | 12363381573 | `4be22e2ee265e21a63cc5d4c89b6df006b41d2d376528b05a770395d7fbf1372` |
| windows-installer-evidence-msi | 12363501473 | `a337f544afa3ffac1480f587b34e73e18ad37be10668886717e3615f00354e10` |

证据目录分别为上述本机根目录中的 `windows-installer-evidence-nsis/` 和 `windows-installer-evidence-msi/`。原始日志不提交仓库；安装器也不加入 Git。

## 未完成门禁与发布边界

维护者目前没有独立干净 Windows 环境，并已明确要求第 3 项先不做。以下保持未验收，不执行付费 Agent 压测、真实 Hook 安装或本机安装/卸载来代替：

- 无 Rust/Bun 的 Windows 10/11 首次安装、运行、升级、重启和卸载；WebView2 缺失及网络受限路径。
- 跨版本升级、EXE/MSI 格式迁移与完整用户配置/未读/固定/去重状态保留。CI 原始结果明确为 `not_verified`。
- 真实 Codex 与另一款已安装 Agent 的新任务、工具、明确结束、通知、重启去重、误报/漏报和事件到提醒延迟。本轮没有新增此类样本，也没有可报告的 P50/P95。
- 外部编辑器窗口焦点与长时间运行。CI 的 focus 字段明确为 `not_verified`。

前端仍有 1 项 low cookie 告警；Rust 仍有 10 项维护和 2 项 unsound 告警，风险与平台/feature 分析见[安全记录](SECURITY_AUDIT_2026-10.md)。四款扩展 Agent 的 fixture/candidate 支持等级不因安装器 CI 成功而升级，见[支持矩阵](AGENT_COMPATIBILITY.md)。

第 2 项已完成不代表第 3、4 项通过。[发布流程](RELEASING.md)中的人工门禁仍有效。当前保持 Draft；只有维护者核对真实支持范围、残余风险、资产和未完成门禁后才能单独决定公开预发布。本轮没有 Publish、自动发稳定版或改写标签。

## 后续文档校对（不改变候选安装包）

2026-10-22 再次通过认证只读查询确认：远端 main 仍为 `b5dc64a`，发布运行
`39636404358` 为 `completed/success`；Draft `419811221` 保持 `draft=true`、
`prerelease=true`、`published_at=null`。本机重新校验四项资产的大小与 SHA256，
`scripts/release.ts verify` 通过，未修改草稿、资产或标签。

将候选状态、发布流程和开源清单中“标签/安装器尚不存在”的过时描述同步为实际结果，
仍保留干净系统、真实 Agent、迁移、焦点与时延的未验收状态。README 的“无公开安装包”
依然准确，不改为公开下载承诺；支持矩阵不提升等级。

回归发现 Bun 的无 `./` 路径会作为过滤器，扫描 `target` 下保留的旧 Git worktree，
把旧测试和当前文档混跑。五个仓库测试入口已统一使用 `./scripts/*.test.ts` 精确路径；
补了先失败、后通过的回归，没有删除工作树、减少测试或放宽发布断言。

针对当前根工作树的独立测试结果（不重复计算旧 worktree）：

- 文档：8 通过，201 条断言；链接/截图、main 克隆、候选身份、Draft 和未完成门禁均检查。
- UI Model：28 通过，259 条断言。
- Release：8 通过，75 条断言；其中 Windows process 回归不执行安装器。
- Security Model：6 通过，45 条断言；Integration Contract：3 通过，168 条断言。
- Svelte Check：0 错误 / 0 警告；前端生产 Build 通过。
- Rust fmt/clippy（`-D warnings`）通过；全量单元测试 390 通过 / 0 失败 / 12 opt-in 忽略。
- `scripts/verify.ps1 -Frontend` 最终成功；使用 `src-tauri/target/codex-check` 构建，未替换正在运行的托盘。
- 隔离 EXE bridge 为 `passed=true`、`lockTimeoutVerified=true`、`realHostConfigurationsChanged=false`。
  本机 GNU debug 的 `guiSubsystem=false`，不冒充发行工作流的 MSVC GUI-subsystem 验收；
  本次证据在 `src-tauri/target/ui-smoke/evidence/integration-bridge-2026-10-21T19-22-28-274Z/`。
  测试入口最后调整后，上述五组根工作树命令另行重跑并全部通过。
- 完整 `bun audit` 仍报告 1 low（预期非零退出）；moderate+ 门禁通过，不描述为零风险。

以上是本地文档和隔离回归，不是第 3 项实机验收。后续文档提交不计入既有
`v0.2.0-alpha.7` 安装器，不自动推送、创建 PR 或发布。
