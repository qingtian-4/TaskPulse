# macOS 开发与测试方案

核对日期：2026-10-26。首轮目标为 Apple Silicon / arm64。本文是实施步骤与验收计划，不表示 macOS 已完成适配、构建或真实宿主验收。

## 当前基础与建议

TaskPulse 已有 macOS 扫描、AppleScript 聚焦、macOS 打包配置，以及[实验构建工作流](../.github/workflows/experimental-platforms.yml)。它可以手动选择分支，在 GitHub 的 macOS/Ubuntu runner 上尝试构建 macOS ARM/Intel、Linux 包。现有[macOS 检查](../.github/workflows/scanner-portability.yml)只编译，不证明应用可运行。

2026-10-26 核对到的旧 Release 为 Windows Alpha.7 草稿，资产没有 DMG。本轮修复已同步至候选分支，并准备 Alpha.8 的独立安装包验收。以 Actions 对应的完整提交与最终证据为准；不要移动现有 Alpha.7 标签。

推荐继续维护同一仓库、同一套任务状态模型和 Svelte UI，只把进程发现、聚焦、原生窗口、权限和通知等差异放在平台实现中。通用修改可在 Windows 上完成；Mac 打包在 macOS runner 完成，菜单栏和真实 Agent 体验在 Mac 实机确认。首轮故障密集时在 Mac 克隆一次、直接调试；之后测试者只下载安装包。

| 角色 | 日常操作 |
|---|---|
| 开发者 | 在短期功能分支修改、测试、提交；平台问题必要时在 Mac 本机用 Codex 调试，验收后合回共同主线。 |
| GitHub Actions | 对准确提交运行检查，使用对应操作系统构建，输出带版本、架构、SHA 和校验和的测试包。 |
| Mac 测试者 | 下载 arm64 DMG，安装、运行固定用例，反馈构建信息和复现步骤；无需安装 Rust/Bun 或每次 clone。 |

## 路线 A：下载测试包（常规测试推荐）

macOS 紧凑布局候选（2026-10-27）把主面板从 412×400–620 调整为约
371×360–558 逻辑像素，提醒从 320×132 调整为 300×124。正文保留字号，
通过缩短工具栏、统计卡和任务卡留白适配；Retina 屏幕仍由系统换算物理像素。
Windows 默认尺寸保持原样。该修改属于 Alpha.8 之后的提交候选，下载时请核对
Actions 的完整提交 SHA；旧 `v0.2.0-alpha.8` 标签和 DMG 不会被替换。

前提是候选代码已经同步至 GitHub，且该提交的 macOS 构建成功；目前不能把本地 Windows EXE 当成 Mac 安装包。

1. 登录有仓库读取权限的 GitHub 账号，打开仓库 Actions。
2. 选择 **Experimental Linux and macOS bundles**，在 Run workflow 中选择包含目标修复的分支，`platform` 选 `macos-arm64`。可用 `commit` 固定完整 SHA；预发布标签默认只构建 Apple Silicon，Intel/Linux 保持手动选择。
3. 对照运行记录的完整提交 SHA，等待 `macos-arm64` 构建成功。
4. 在该次运行的 Artifacts 下载 `experimental-macos-arm64-<SHA>`；解压后找到 DMG。
5. 用 `shasum -a 256 -c macos-arm64-SHA256SUMS.txt` 校验，再打开 DMG，把 TaskPulse 拖到 Applications，从 Finder 启动。更换版本前用托盘菜单退出原实例，备份既有状态；不要为测试删除真实未读或来源配置。
6. 在“关于 TaskPulse”复制版本、SHA 和路径，确认确实运行了目标构建。

Actions 附件要求登录并拥有读取权限，且有保留期限，不适合作为长期公开下载页。预览版经基本验收后可另行发布明确标记的 GitHub pre-release；尚未发布的 draft 不等于公众可下载。参见 [GitHub 附件下载](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/download-workflow-artifacts)和 [Tauri DMG](https://v2.tauri.app/distribute/dmg/)。

新流程将 Apple Silicon 固定为 `macos-15` / `aarch64-apple-darwin`，Intel 为 `macos-15-intel` / `x86_64-apple-darwin`，均在相同架构执行测试。Mac 包使用 ad-hoc 签名，不含 Apple 公证。CI 验证 DMG 挂载、复制后的应用签名/架构/版本、资源、首次启动、二次激活和重启，证据单独下载；这些检查不证明 UI 点击、Gatekeeper、通知焦点或真实 Agent 审批通过。签名来源见 [Tauri 官方说明](https://v2.tauri.app/distribute/sign/macos/)。

首次打开若提示开发者无法验证，在确认下载来源及校验和后，可按 [Apple 的单应用放行步骤](https://support.apple.com/en-us/102445)，到“系统设置 → 隐私与安全性 → 仍要打开”确认。不要全局关闭 Gatekeeper。若提示文件损坏或不出现放行入口，请保留提示并反馈，不用删除安全属性来冒充正常安装通过。

首轮使用经过验证的 ad-hoc 签名实验包，并明确未公证；当前工作流已配置这一签名步骤。它不等于 Apple 身份签名，仍可能出现系统确认。面对公众的顺畅下载体验应配置 Developer ID 签名和 notarization；不用关闭系统 Gatekeeper。证书通过受限 CI secrets 提供，不进入源码或聊天。参见 [Tauri macOS 签名说明](https://v2.tauri.app/distribute/sign/macos/)。

## 路线 B：Mac 本地开发（首轮排查推荐）

先确保目标分支已包含本地新提交。Mac 安装 Xcode Command Line Tools、Rust/rustup 和 Bun；本仓库当前 CI 固定 Rust 1.98.0、Bun 1.4.0。仅开发桌面应用时不必安装完整 Xcode，见 [Tauri 环境要求](https://v2.tauri.app/start/prerequisites/#macos)。

```sh
# 首次安装系统编译工具，完成安装后再继续
xcode-select --install

# 确认系统和架构（Apple 芯片原生终端应显示 arm64）
sw_vers
uname -m

# 在安装 rustup 与 Bun 后执行；先确认远端分支含有目标修复
git clone --branch codex/p0-remote-acceptance https://github.com/qingtian-4/TaskPulse.git
cd TaskPulse
git switch -c codex/macos-alpha
git rev-parse HEAD

rustup toolchain install 1.98.0 --component rustfmt --component clippy
rustup override set 1.98.0
bun install --frozen-lockfile
bun run check

# 使用独立 TaskPulse 状态目录，首次明确选择要监控的本机来源
TASKPULSE_HOME="$PWD/src-tauri/target/mac-dev-profile" bun run tauri dev
```

这里应使用 `bun run tauri dev`；根目录 `dev.sh` 是当前 Windows/MinGW 环境专用。UI 保存后由开发服务器刷新；Rust 改动会重新编译，不必每改一处就生成和安装 DMG。Codex 只需打开这个同一仓库并遵守 AGENTS.md，无需另建一份独立 Mac 产品。

改动完成后执行检查，再打包：

```sh
bun run check
bun run test:ui-model
bun run build
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --bin agent-tray -- -D warnings
cargo test --locked --manifest-path src-tauri/Cargo.toml --bin agent-tray
bun run tauri build --target aarch64-apple-darwin --features custom-protocol --bundles app,dmg -- --locked
```

最后一条必须在 Mac 执行。开发模式的表现与 Finder 打开的打包应用分别验收，不能用开发模式通过代替安装包验证。

## 首轮适配与验收顺序

1. 固定 macOS runner 与显式 arm64 target，先跑编译和单元测试，再修复平台问题；Intel 随后验证，Linux 暂保实验。
2. 核对菜单栏图标、Dock 行为、弹出方向、浅/深色、Retina、多显示器、Space/全屏、失焦关闭与固定窗口。
3. 检查 macOS 窗口线程规则和无抢焦点提醒。Windows WebView2/CDP 脚本不能直接证明 macOS WKWebView 行为。当前 Tauri 官方推荐 WebdriverIO 的 `@wdio/tauri-service`，embedded 模式可覆盖 Windows/Linux/macOS，可用于统一页面操作、IPC 和日志检查；直接驱动传统 tauri-driver 则仍只支持 Windows/Linux。建议只在测试构建启用嵌入式测试插件，不随正式包开放控制接口；系统权限、焦点和菜单栏仍单列实机验收。该自动化尚未接入本项目。[官方测试说明](https://v2.tauri.app/develop/tests/webdriver/)
4. 验证从 Finder 启动时能找到 Codex/SSH/Python 等实际需要的工具。GUI 应用不继承 shell 初始化文件的 PATH，终端启动成功不能覆盖该场景。[官方说明](https://v2.tauri.app/distribute/dmg/)
5. 先跑独立配置的测试提醒，再明确接入一个真实本机 Agent，覆盖开始、思考、明确输入/审批、匹配解除、结束、失败和重命名。Windows 的 WSL Hook 不会自动变成 Mac Hook，不能自动安装或修改审批策略。
6. 点击跳转触及 Automation/Accessibility 时按实际功能请求和验证权限，逐项审查 entitlements。再验证退出重启后的未读/置顶/去重保留，以及正常升级保留状态。

每次反馈只需系统版本、芯片、构建 SHA、操作步骤、期望/实际结果和脱敏截图。后续可增加一键复制诊断、固定的合成测试场景以及仅导出安全字段的验收报告；这些入口尚未实现，不能声称已经可用。

## 分发与 Linux 路线

近期先完成 **Windows x64 + macOS arm64**，macOS Intel 随后提供独立包。Linux 先选定 Ubuntu 桌面环境，用 DEB/AppImage 验证托盘、通知、Wayland/X11 和聚焦，再决定 RPM 等格式；拥有安装包不等于支持所有发行版。

交付分成三个阶段：开发者本地热重载 → 指定提交的 Actions 测试包 → 签名/验收后的 pre-release。稳定后再做应用内更新和可选 Homebrew Cask。Tauri updater 需要自己的更新包签名，与 Apple 签名/公证不同；本项目当前没有安装 updater。[官方 updater](https://v2.tauri.app/plugin/updater/)

查阅的开源实践：

- [RustDesk 构建工作流](https://github.com/rustdesk/rustdesk/blob/master/.github/workflows/flutter-build.yml)按平台/架构运行不同构建任务，macOS 包含 ARM/Intel、DMG、签名和公证，另有 nightly/更新元数据流程。可借鉴平台矩阵和分发分层，无需照搬其全部复杂度。
- [LocalSend 发布工作流](https://github.com/localsend/localsend/blob/main/.github/workflows/release.yml)复用各平台构建并汇总为草稿；[贡献说明](https://github.com/localsend/localsend/blob/main/CONTRIBUTING.md)明确包管理器维护者及尚未由流水线覆盖的手动构建。跨平台项目也可以逐步补齐自动化。
- [Tauri 官方 GitHub 模板](https://v2.tauri.app/distribute/pipelines/github/)展示 macOS ARM/Intel、Windows、Linux 构建矩阵，适合沿用本项目现有技术栈。
