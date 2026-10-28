# TP-03 品牌替换验收 · 2026-10-28

本轮从 `origin/main = 1fbbb75` 建立 `codex/tp-logo`，使用独立 worktree：
`D:\Project\agenttray\.worktrees\tp-logo`。主目录仍在 main，构建、依赖及运行测试配置分别隔离。
已完整阅读交付包 CODEX_PROMPT.md、README-EXPORTS.md、manifest.json 和所有 SVG 母版。
用户本次要求优先于仓库默认合并流程：验证后提交 PR，合并由维护者判断；不自动合并或发布。

## 修改范围

- `assets/brand/`：TP-03 母版、五状态 SVG、深浅 README 字标、Social Preview、manifest 与说明。
- `src/lib/assets/brand.svg`：既有 BrandMark 组件的共享资源，覆盖 Dashboard、消息中心和来源管理。
- `src-tauri/icons/` 顶层桌面 PNG/ICO/ICNS 和托盘 PNG：均替换为 TP；未配置的移动平台脚手架产物不参与桌面打包。
- `assets/screenshots/*-current.png`：实际隔离原生测试重新捕获；历史截图和历史报告不重绘。
- 品牌资产测试补充 TP 横笔、跨资源几何、二进制清单与 ICNS 完整性验证。

Tauri 配置、应用身份/版本/安装模式、窗口尺寸、组件布局以及任务采集/状态/通知生产逻辑保持不变。
托盘继续采用“待处理 → 异常/监控退化 → 待查看 → 运行中 → 空闲”，保留浅蓝待查看。
启动或状态不可信仍按已有机制显示待更新，图标切换不新增通知。

## 验证记录

所有运行测试使用独立构建目录与 TASKPULSE_HOME；不改真实 Agent Hook、用户配置或数据。

最终 `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify.ps1 -Frontend` 完整通过：
Bun check/build、文档/仓库/UI/发布/协议/清理回归，Rust fmt、Clippy `-D warnings`、
497 项 Rust 测试通过/24 项外部环境测试忽略、custom-protocol build，以及实际 Bridge/MCP smoke。
最终日志为 `src-tauri/target/tp-kit-20261028/verify-final.log`，保留之前各次失败日志作对照。

原生复现命令（串行运行，使用空闲的 CDP 9337 端口）：

```powershell
bun scripts/native-product-smoke.ts --exe src-tauri/target/codex-check/debug/agent-tray.exe --capture
bun scripts/native-message-center-smoke.ts --exe src-tauri/target/codex-check/debug/agent-tray.exe --capture
```

资产检查 5 项、212 断言通过：全部活动 SVG 使用 TP-03 路径和 11.5 描边、共享 UI 母版一致、
二进制 SHA256 清单完整，Windows ICO 多分辨率有效，macOS ICNS 普通/Retina 表示有效。
运行时图片回归还验证每种颜色保留左伸的白色 T 横笔，旧 P PNG 混入会失败。
深浅字标、五色 16/20/24/32px 静态缩放预览经本地浏览器渲染并目视检查，无裁剪。

首次完整验证的 fmt/Clippy、前端构建及 Rust 497 passed / 24 ignored 均通过，但 Bridge
诊断断言失败。临时诊断副本定位为来源退化、taskCount=0；进程退出码 0、stderr 为空，
全部敏感字段哨兵均未出现，不能将原笼统错误文案解释为隐私泄露。
只缩短测试目录名后，同一二进制通过原有断言；随后将 worktree 从较深的
`src-tauri/target/worktrees/tp-logo` 移到 `.worktrees/tp-logo`，未修改的正式 Bridge 测试通过。
这是本机长路径环境限制的对照证据，本轮没有改生产路径处理或降低测试断言。
移动 worktree 后，Tauri 缓存的生成权限文件仍指向旧绝对路径，首次重建中断；仅清理
该 worktree 的 tauri/窗口插件/应用包缓存后重建。首次失败、临时探针结果和后续正式结果均保留，
不将失败轮次记为成功。

原生产品测试通过 12 组检查，五种托盘状态的 `set_icon` 实际调用成功；任务面板、来源管理、
提醒窗的 DPR 1/1.25/1.5/2 布局检查通过。消息中心原生回归通过，包括消息已读与结果查看/请求解决
相互独立、匹配请求解除、源故障恢复、隐藏/重开与重启不重播通知。
刷新了 Dashboard、消息中心、来源管理 current 截图，目视确认新 TP 标记。
通知截图中原本没有旧 P 标记的部分维持原样。全部输入为隔离合成事件，不作为真实宿主生命周期验收。

任务栏 UIAutomation 抓图在系统 DPI=96 下未找到可见匹配元素；只记录了五色原生写入成功，
**未宣称 Windows 任务栏实际像素验收通过**。未操作用户任务栏溢出区或修改系统显示设置。

本地证据位于 worktree 的 `src-tauri/target/tp-kit-20261028/` 和
`src-tauri/target/ui-smoke/evidence/`；旧日志中的深路径对应迁移前位置。
原生结果目录为 `product-native-2026-10-28T08-22-17-069Z-4zQJDg` 与
`message-center-native-2026-10-28T08-24-42-021Z-GWj6YE`。
完整 `bun audit` 报告 4 项 low（SvelteKit 2、cookie 1、deepmerge 1）；中风险以上门槛通过，未修改依赖。
本轮不是发布候选；本地未另跑 RustSec/全历史 secrets 扫描，PR 的 Security CI 单独检查。

## 仍需实机验收

- Windows 实际任务栏 100%/125%/150%/200% DPI、亮/暗任务栏、跨屏切换、溢出区。
- 新版资源的 NSIS/MSI 独立安装、升级、卸载后重装及图标缓存。未运行安装器，不复用旧版本安装通过结论。
- macOS Finder、Dock、应用切换器、菜单栏在浅/深色与 Retina 下显示；实际 `.app`/DMG 安装升级及重启。
  ICNS 容器校验不等于 Mac 实机验收，本轮 Windows 主机不能验证上述表现。
- 当前菜单栏仍使用彩色状态图；单色 template 图标属于后续设计，本轮不启用 template 标志。
- GitHub Social Preview 需维护者手动上传新版 `assets/brand/social-preview.png`；提交资源不更新仓库设置。

合并后可确认分支 tip 已被远端 main 包含、worktree 无未提交文件，再清理 worktree 和本地/远端分支。
审查期间保留分支与隔离目录；回滚只需撤销本轮资源提交并重建，不迁移或删除用户状态。
