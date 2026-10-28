# TaskPulse 模型信息、双平台安装与开源入口补齐方案

日期：2026-10-28。实施位置：当前 `codex/task-model-cli-install` 分支及同名 worktree。
代码核对基线：`ddf2515c25a204274034f8d89b11bba8e5397aa9`，Draft PR #29。
本文是剩余工作的执行方案，不表示下面标为待做的功能已经实现。

## 1. 产品边界与借鉴原则

TaskPulse 聚焦 Coding Agent 的任务状态、本轮经过时间、明确阻塞、未查看结果和跨来源关注。
不实现 TPS、token 速度曲线、速度排名、速度称号、赛车/悬浮球仪表盘；本方案也不扩展费用或网络流量计量。
模型名称只是任务上下文。一个项目的多个任务可能使用不同模型，不把整个项目强行归为一个模型。
不复制 Agent Speed 的代码、文案、图标或截图；保留 TaskPulse 现有品牌及 AgentTray 来源声明。

可借鉴的是只读采集、适配器隔离、增量处理、统一字段、可复现测试、版本一致性以及便捷分发。
保留同一套 Rust/Tauri/Svelte 产品；不仿照对方分别维护 Swift 和 C# 两份核心。

```text
Agent 本地日志 / SQLite / 显式事件
        ↓
Provider：读取、解析、来源证据与能力限制
        ↓
范围过滤 / 标准化 / 去重 → TaskStore
        ↓
AttentionTracker → TransitionEngine
        ↓
托盘 / 任务面板 / 明确事件提醒
```

这个分层在本次开发前已经存在。上轮主要沿现有链路补齐 model，而非重新实现整个架构。
进一步完善的是监控口径和覆盖证据，不另建框架或服务。

## 2. 已执行与尚未执行

| 项目 | 当前事实 | 后续动作 |
| --- | --- | --- |
| 参考架构与模型来源研究 | 已读规格、Codex/Claude/DSH 实现、Windows 安装及发布流程；本轮补读 macOS 安装/打包、README 与发布模板 | 固定来源链接，区分可采用机制和不采用产品功能 |
| 模型采集和显示 | 已实现 Codex、Claude、DSH、Hermes；任务列表、详情、搜索、复制均已接入 | 保持身份、计时、未读和通知独立，补齐 Mac 验收 |
| Windows 命令行入口 | install.ps1 已实现；离线失败路径、并发、相对路径、语言和原生 PowerShell 兼容测试通过 | 以新构建安装包验证实际下载/安装/升级链路 |
| Windows 安装包 | 原有 NSIS/MSI 打包与独立安装器 CI 已有 | 为当前 PR 精确提交构建测试包；上轮只交付了开发 EXE |
| macOS 安装包 | 原有 arm64/x64 实验流程；旧 Alpha.8 arm64 DMG 有 CI 证据 | 对当前功能提交分别构建 arm64/x64 并验收，不复用旧证据 |
| macOS 命令行入口 | 尚无 install.sh | 新增，消费已有 DMG 与架构专用校验文件 |
| README | 已有简中/英文、品牌、截图、能力表；上轮只补 Windows CLI 链接和说明 | 重整用户安装入口，将源码开发从普通安装路径分开 |
| 发布规范 | 原有版本/标签/安全/独立安装器/草稿门禁；本次补了 CLI 兼容要求 | 统一双语说明模板、双平台资产核对和当前提交的证据索引 |
| 公开一行安装 | 尚未完成真实公开下载验收；候选为 Draft | 发布经单独授权后才能验收匿名公开入口 |
| Mac 实机验收、签名公证 | 本次未做；现有 Mac 包为 ad-hoc、未公证 | CI 与实机分别记录；正式 Developer ID/公证不是已具备条件 |

上轮 Windows 代码验证：502 项 Rust 测试通过，前端/发布/文档检查通过，真实来源模型读取与
隔离 WebView2 验收通过；上述基线的 Windows quality / Security 通过。这些结果不等于 Mac
实机验证、当前版本安装包验证或公众下载已完成。应用全量英文 UI 也不能仅凭双语 README/安装脚本宣称完成。

## 3. 参考项目实际怎么做

参考固定为 Agent Speed v1.4.1 / `a94a67fcc78e5d8eea5f82d8d5e737bccf994e2c`。

- README 先给产品一句话、语言切换、平台/发布徽章和截图，很早给出两个平台的安装命令；随后解释功能、来源路径、测量口径、隐私、源码构建和许可。可学习信息顺序与限制说明，不复用表述或视觉素材。
- v1.4.1 的应用资产实际是 `AgentSpeed-Windows-x64.zip`、`AgentSpeed-Windows-arm64.zip` 和
  `AgentSpeed-macOS.zip`，另有 SHA256SUMS；不能把它们描述成 NSIS/MSI/DMG 安装器。
- Windows 自包含可执行文件按架构打 ZIP；PowerShell 下载校验后完成用户目录安装、快捷方式、卸载项及自启相关处理。
- macOS 将 arm64/x86_64 合成 universal app，ad-hoc 签名后打 ZIP；shell 下载校验，再复制 app 到 Applications。
- 两种入口本质上消费同一批 Release 产物，命令行入口不是另一份产品构建。
- 对方 macOS 安装脚本还会移除 quarantine、强制结束同名进程和先删除旧 app。TaskPulse 不采用这些做法；沿用系统安全提示，校验目标身份，保留可回滚旧版本。
- 对方 tag 流程组合平台构建、校验和及版本说明。本项目保留已有更严格的 Windows 验收与 Draft-only 策略，不切换到自动公开发布。

来源：
[README](https://github.com/zoefix/agent-speed/blob/a94a67fcc78e5d8eea5f82d8d5e737bccf994e2c/README.md)、
[macOS 安装](https://github.com/zoefix/agent-speed/blob/a94a67fcc78e5d8eea5f82d8d5e737bccf994e2c/install.sh)、
[macOS 打包](https://github.com/zoefix/agent-speed/blob/a94a67fcc78e5d8eea5f82d8d5e737bccf994e2c/macos/scripts/build-app.sh)、
[Windows 打包](https://github.com/zoefix/agent-speed/blob/a94a67fcc78e5d8eea5f82d8d5e737bccf994e2c/windows/scripts/publish.ps1)、
[Release 资产](https://github.com/zoefix/agent-speed/releases/tag/v1.4.1)。

## 4. 执行顺序与完成标准

### A. README 与监控口径

改动 `README.md`、`README.en.md`、`docs/INSTALL.md`、`docs/CLI_INSTALL.md`、
`docs/AGENT_COMPATIBILITY.md` 和 `docs/ARCHITECTURE.md`，必要时同步 `docs/MACOS_DEVELOPMENT.md` 的旧分支示例。

1. 保留当前 Logo、品牌、已验证截图和上游署名，写清“任务状态与本轮经过时间”。
2. README 的用户阅读路径：产品定位 → 实际界面 → 平台/安装选择 → 主要功能 → Agent 支持程度 →
   工作原理与隐私 → 源码开发 → 贡献/许可/路线图。将源码构建命令移至开发说明，普通用户无需 Bun/Rust。
3. 安装表分 Windows x64、macOS Apple Silicon、macOS Intel；每行列安装包、命令行入口、可用阶段。
   Draft、实验、未实机验证按事实标注，不能用失效的 latest 链接假装可下载。
4. 加入模型信息来源和未知值规则；“本轮经过时间”不称为纯模型计算时长，开始时间缺失不补造；
   来源更新时间与执行计时分开。保持思考/工具/模型等待为运行中，批准仅认明确请求。
5. 可公开的新截图由当前构建和明确标注的合成任务生成，展示模型信息，避免私人任务内容。
6. 文档、安装输出和发布说明仅维护简体中文与英文。界面全量国际化不混入这次分发补齐。

完成标准：两份 README 信息对应，安装入口/支持矩阵/版本状态一致；`bun run test:docs` 和
`bun run test:repo` 通过。更新测试以验证真实新结构，保留对品牌、未发布状态和来源证据的要求。

### B. macOS 命令行安装

新增根目录 `install.sh`，以系统自带工具实现，不要求用户安装 Bun、Rust、Python 或 jq。
默认选择公开稳定版；`--prerelease` 显式选择预发布，`--version` 指定公开版本。
其他参数固定为 `--download-only`、`--output-dir`、`--dir`、`--no-launch`、`--language en|zh-CN` 和 `--help`。
不新增自动更新后台服务、卸载/清数据命令或任意下载镜像入口。

1. 检测 Darwin 与真实硬件架构，处理 Apple Silicon 上从 Rosetta 终端启动的情况；arm64/x64 各取对应包。
   架构识别失败停止，不猜测。支持版本以包内声明和实际测试矩阵为准；当前配置的 10.15 最低目标不是完整兼容证明。
2. 只选择本仓库公开 Release；要求匹配的 DMG、`macos-<arch>-SHA256SUMS.txt` 和
   `macos-<arch>-manifest.json` 齐全。复用 `TaskPulse_<version>_macos_<arch>.dmg`，不另造一套 ZIP 产物。
3. 解析 GitHub JSON 使用固定逻辑和系统 `plutil` 等工具，数据不进入 eval。验证文件名、来源、版本、平台、
   架构、源 SHA 格式、manifest 与 SHA-256 一致性；缺资产、坏校验、超时均在安装前失败。
4. 安装模式只读挂载已验证 DMG；验证仅含预期 app、bundle ID `app.taskpulse.desktop`、实际二进制架构、
   app 版本与签名完整性。ad-hoc 不标为 Developer ID 或已公证。
5. 默认安装到 `~/Applications/TaskPulse.app`，`--dir` 可选择已有写权限的目录；不自动 sudo。
   目标是不同产品或符号链接时拒绝覆盖。已有 TaskPulse 正在运行时提示正常退出后重试，不按进程名强杀。
6. 在目标同一文件系统完成暂存和验证，再替换。用目标锁阻止并发更新；旧 app 在新 app 就位前保留。
   失败还原旧 app；异常中断留下的恢复目录显式报告并阻止下一次盲目覆盖。仅清理本次拥有的临时文件/挂载点。
7. 不删除用户 TaskPulse 状态、不安装 Agent Hook，不清除 quarantine 或关闭 Gatekeeper。
   启动失败与系统拦截应保留证据和 Apple 官方操作指引，不把绕过检查当作成功。
8. 仅下载模式在指定输出目录同盘暂存、原子保存；尊重当前 shell 目录，拒绝覆盖已有版本目录。

完成标准：新增 `scripts/test-cli-install.py` 离线测试；覆盖未知系统/架构、Rosetta、稳定/预发布选择、
草稿、缺失/重复资产、错版本、坏校验、下载中断、含空格/中文路径、并发、已安装同版本、升级失败回滚、
运行中 app、符号链接、非本产品目录、残留恢复目录和两种语言。测试 Python 只用于开发，不随用户安装要求新增。

### C. 当前分支的双平台安装包和安装入口验收

涉及文件超过 8 个，需按 A/B/C/D 形成小提交；保持一个 worktree 和当前 PR，不创建新的产品分支。

1. 新增 `.github/workflows/installation-preview.yml`，以同仓库 PR 更新触发，锁定 PR head 的完整 SHA。
   构建流程 `contents: read`，仅上传带 SHA 的测试附件；不创建 Tag、不写 Release，不改主分支。
   新工作流不能仅依赖 workflow_dispatch，因为新文件尚未进入默认分支时不能假定手动触发可用。
2. 原有 `release.yml` 的“标签或准确 main 提交”校验继续保留。新增 PR 测试路径不能通过放宽该校验来实现。
   复用 `scripts/release.ts` 的 staging/verify 能力，给 PR 产物增加明确 `build_kind=pr-preview`、`tag=null` 标记，
   不让公开 CLI 把测试附件当成公开版本。相应更新 `scripts/release.test.ts`。
3. Windows 用 MSVC 生成 NSIS 与 MSI，复用独立 runner 的 `installer-smoke.ps1`，每种格式独立验收，
   不共享安装偏好；保持原生 UI/IPC/版本/重启/卸载与状态保留检查。新增安装 bootstrap 的受控下载探针，
   用本地测试响应喂给同一个验证/执行函数，不给生产脚本加跳过校验开关；安装器仍按正常权限运行。
4. macOS arm64 使用 `macos-15`，x64 使用 `macos-15-intel`，在对应原生架构完成测试和 app/DMG 构建。
   复用 `macos-package.py`、`test-macos-package.py` 及现有签名/启动/重启检查；扩展 manifest 校验而不破坏旧资产读取。
5. 新增 `scripts/macos-cli-smoke.sh` 在隔离 HOME、Applications 和测试下载源中执行真正的安装脚本，
   不能只测试一个模拟安装函数。生产默认来源保持固定；测试传输替身仅在测试进程提供，不能在产品里留下任意执行入口。
6. 新增 `scripts/installation-preview.test.ts`，检验精确 SHA、只读权限、测试标记、资产命名、无发布步骤、
   未放宽旧发布门禁。对安装器和正式 bootstrap 共用的验证路径保留坏输入反例。
7. Actions 的结果记录 OS、架构、应用 SHA、脚本 SHA、包 SHA-256、安装方式和每项通过/未验证状态。
   只公开合成测试图和脱敏证据。当前 Windows 开发 EXE 与旧 Alpha.8 DMG 都不能代替这些新包。

完成标准：同一个候选源码 SHA 的 Windows x64、macOS arm64、macOS x64 测试附件与独立证据齐全。
某个平台失败则明确保留失败状态，其他平台的成功不能替代它；尤其不将 Intel 自动标为支持。

### D. 发布说明、用户测试与最终合并

更新 `docs/RELEASING.md`、`docs/CANDIDATE_TESTING.md`、`docs/MACOS_DEVELOPMENT.md`、
`docs/CLI_INSTALL.md`，增加 `docs/releases/TEMPLATE.md`。

1. 发布模板只含简中/英文：变化、平台/架构与系统要求、包安装/CLI 安装、升级与状态保留、
   已知限制、签名/公证状态、校验文件、源提交和验证范围。不把未实现能力列为已交付。
2. 测试者在 Windows 和 Mac 各自验证：真实来源模型、正常思考/工具阶段、明确输入/批准与解除、结束/失败、
   重命名、未读/固定/通知去重、从系统界面启动、退出重启和升级保留状态。
3. Mac 单列 Finder 下载后的 Gatekeeper、菜单栏、Retina/多屏、通知焦点和真实 Agent 用例。
   CI 的挂载/复制/运行通过，不替代这些实机结果。
4. 收到维护者测试稳定确认，并且最终 PR head 的 Windows/Security 及对应 Mac 检查通过后，才合并 main。
   确认分支 tip 已在 origin/main、无未提交工作后，再删除本功能 worktree/分支。
5. 发布版本仍需单独明确授权，才同步四处版本、创建新不可变 tag 并生成 Draft。既有 Alpha.8 不移动或覆盖。
   公开后再验证匿名下载与 README 一行入口；不得为了完成测试而提前公开。

## 5. 验证入口与外部条件

文档与合约：`bun run test:docs`、`bun run test:repo`、`bun run test:release`、
`python3 scripts/test-cli-install.py`、`python3 scripts/test-macos-package.py`。
跨平台实际安装：上述 PR preview CI，调用现有 Windows 安装验收和新增 Mac CLI smoke。
后端/前端变更仍跑 `bun run check`、`bun run test:ui-model`、`bun run build`、Rust fmt/clippy/full tests；
候选构建保留 frontend/Rust/secrets 安全门禁，告警按事实记录。

当前执行环境是 Windows/WSL，已确认有 sh/bash/Python，没有 macOS 的 hdiutil/xcrun/plutil。
可在这里实现和做离线回归；真正的 Mac 安装/签名验证必须到上述 macOS runner，实机体验由 Mac 测试者完成。
本方案沿用现有 GitHub Actions 访问，不引入第三方服务或新的用户 API key。
Developer ID/公证需要维护者 Apple Developer 账户及受限 CI 凭据，尚未配置，不作为可自动完成的前提。

最脆弱的前提是公开 Release 已存在且所需架构资产齐全。为避免这个前提阻塞开发，
合并前用精确 SHA 的 CI 测试附件与受控传输验收；公开 CLI 始终拒绝 Draft 和缺资产版本。
GitHub 不可达时脚本应在明确超时后退出，用户仍可按已下载并核验的安装包路径安装。

回退：功能与文档小提交可单独回退；模型字段原本可选，不需迁移/删除用户状态。
安装失败保留或还原旧 app，日志只记录版本/路径/校验结果，不采集对话或凭据。

官方依据：
[Tauri DMG](https://v2.tauri.app/distribute/dmg/)、
[Tauri macOS 签名](https://v2.tauri.app/distribute/sign/macos/)、
[Apple 单应用安全放行](https://support.apple.com/en-us/102445)。
