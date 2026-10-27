# TaskPulse 品牌集成验收 · 2026-10-27

本次范围：固定双色开口 P、桌面应用/安装器图标资源、中英文 README、Dashboard/消息中心/来源管理，以及真实聚合托盘展示。状态映射、资源来源、Social Preview 手动设置和回滚方式见 [品牌说明](../assets/brand/README.md)。

## 基线

已解压并完整阅读交付包 CODEX_PROMPT.md、README-EXPORTS.md、manifest.json 和 SVG 母版。
附件建议作为设计输入；操作范围以用户要求和仓库 AGENTS.md 为准。
刷新 origin 后 main 为 `fe48e69`，相关 [PR #25](https://github.com/qingtian-4/TaskPulse/pull/25) 已合并；
本地从 `f233621` 开始，包含最新 main，领先 6 个提交。保留当前 codex 分支，无合并/推送/PR/发布。
工作树已有且持续更新的 Doubao 源码、测试、说明与变更记录不属于本次提交；开发期间这些改动由另一工作流提交为 `a75cd26`，本次在其上继续，未覆盖或重复提交。
审查时 GitHub 无打开的 PR。本次未触发云端 CI；历史 PR 安装结果仅作上下文。

## 自动验证

原生测试使用隔离 TASKPULSE_HOME 与合成标准事件，不改真实配置、Hook、监控范围，不发付费 Agent 请求。

| 命令/检查 | 实际结果 |
|---|---|
| `bun run tauri icon assets/brand/brand.svg --output src-tauri/target/brand-kit-20261027/generated` | 成功生成桌面图标；目视检查固定品牌 PNG。字体枚举有一条 malformed-font 警告，母版无字体依赖 |
| `cargo test --manifest-path src-tauri/Cargo.toml --bin agent-tray tray::tests -- --nocapture`（隔离 target） | 12 项通过：多 Agent、五状态、健康退化、未知/启动、匹配请求语义、已查看、重启、静默/恢复、通知去重、图片透明度、重复刷新/旧 revision |
| `bun test scripts/brand.test.ts` | 2 项、62 断言通过；ICO 包含 16/24/32/48/64/256px，目录偏移/长度/32-bit 校验通过；UI 母版与品牌源一致，桌面 PNG 为透明 RGBA，Social Preview 1280×640 |
| `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify.ps1 -Frontend` | 通过：Bun check 0 错误/警告、build、文档/仓库/UI/安全脚本/发布/协议/清理回归；Rust fmt、Clippy `-D warnings`、完整 Rust 494 passed / 23 ignored、custom-protocol build、实际隔离 Integration Bridge 与 MCP smoke |
| `bun run release:check` | 通过，仍为 0.2.0-alpha.8；没有改版本或建立 Release |
| `bun scripts/native-product-smoke.ts --exe src-tauri/target/codex-check/debug/agent-tray.exe --capture` | 通过：真实 Tauri/WebView2；全部五色原生 `set_icon` 成功，含运行+未查看+请求+失败并存时请求优先；27 任务、搜索/筛选/管理/提醒及 DPR 1/1.25/1.5/2 布局检查通过 |
| `bun scripts/native-message-center-smoke.ts --exe src-tauri/target/codex-check/debug/agent-tray.exe --capture` | 通过：四个逻辑窗口尺寸 × 四种 DPR；消息阅读不改 task unread/request pending、匹配解决、隐藏/重开、来源故障恢复、重启不重播通知 |

初次沙箱内 Vite 检查遇到 Windows `spawn EPERM`；正常授权的外部沙箱运行通过，未改工具链或依赖。
完整 `bun audit` 返回 3 条 low（SvelteKit 2 条、cookie 1 条）；moderate+ 门禁通过不等于零风险。
本次没有依赖变更或发布候选，未重新执行 RustSec 和全历史 secrets 审计；不能沿用历史结果作本次安全认证。

本地证据（忽略提交目录）：

- `src-tauri/target/brand-kit-20261027/verify.log`、`native-product.log`、`native-messages.log`。
- `src-tauri/target/ui-smoke/evidence/product-native-2026-10-27T05-26-55-395Z-bcZl9M/`：原始 result/cleanup 与 PNG。
- `src-tauri/target/ui-smoke/evidence/message-center-native-2026-10-27T05-28-00-256Z-iPQQNU/`：原始 result/cleanup 与 PNG。
- 公开更新了 `assets/screenshots/` 的 dashboard-current、message-center-current、sources-current、notice-current；均为合成输入的原生截图。

任务栏抓图尝试读取 UIAutomation：系统 DPI=96，但五次均未找到可见的匹配托盘元素，没有点击溢出区或改系统设置。
因此 **只确认五种图标原生写入成功，不宣称 Windows 任务栏像素实际显示验收通过**；截图探针保留为 `scripts/capture-tray.ps1`。
原先运行的工作区应用未替换；新构建位于 `src-tauri/target/codex-check/debug/agent-tray.exe`，这是开发验证构建，不是公开 portable 发行包。

## 修改文件

- `assets/brand/*`、`src/lib/assets/brand.svg`：母版、状态 SVG、横向字标、Social Preview、manifest/说明。
- `src-tauri/icons/*` 顶层桌面资源：固定品牌 PNG/ICO/ICNS 与五色托盘 PNG；移动平台历史产物未覆盖。
- `src-tauri/tauri.conf.json`：NSIS installerIcon 显式绑定；MSI 继续使用 bundle.icon，upgradeCode/版本未变。
- `src-tauri/src/tray.rs`、`watcher.rs`、`main.rs`：统一快照聚合、主线程写入/去重、首次远端健康占位、启动同步与回归。
- `src/lib/components/BrandMark.svelte`、`Dashboard.svelte`、`MessageCenter.svelte`、`src/routes/manage/+page.svelte`：复用品牌小标记，不改窗口尺寸。
- `README.md`、`README.en.md`、`assets/README.md`、四张 current 截图及本报告：品牌展示与证据边界。
- `scripts/brand.test.ts`、`capture-tray.ps1`、两个 native smoke 与 `package.json`：产物校验、原生托盘写入验证、125% DPR 覆盖。

## 尚需实机/CI验收

- Windows 实际任务栏 100%、125%、150%、200% 缩放，亮/暗任务栏、混合 DPI 跨屏及溢出区域。
- 真实 Agent 并行生命周期、请求解决、源失联恢复与长期运行；本次合成事件与单元测试不能替代。
- 当前提交的 NSIS/MSI 独立 fresh runner 安装、重启、同版本重装、跨版本升级、跨格式迁移及图标缓存显示。
  仓库规定安装器 smoke 仅在 CI 执行，本次不推送或发布，不能沿用 PR #25 的安装结果作为本次通过证据。
- GitHub 深浅主题实际页面渲染与 Social Preview 手动上传。
- macOS 单色模板菜单栏与 Linux 实机显示；当前彩色托盘以 Windows 为验收目标。

应用 identifier、版本、WiX upgradeCode、NSIS 安装模式及用户状态路径均未改动；这是兼容性措施，不是升级验收结论。
