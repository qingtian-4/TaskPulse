# 安装预览验收记录（2026-10-28）

范围：PR #29 的模型信息、双语入口与双平台安装。保留同一工作树，已执行
`git fetch origin`、`git merge origin/main`，接入 TP-03 Logo（main `8793ee3`）。
旧 Alpha.8 标签包与 Windows 开发 EXE 不作为本次安装包证据。

## 证据填写规则

精确源码 SHA 从每次 manifest 与 Actions checkout 读取，最终 PR head 的链接与结果保存在
[PR #29](https://github.com/qingtian-4/TaskPulse/pull/29)。任何代码变更后都重新核对最终 head，
不能沿用旧绿色状态。附件带完整 SHA，仅为 `build_kind=pr-preview`、`tag=null` 的测试产物。
发布流程未放宽 main/tag 校验，未创建或公开 Release。

| 范围 | 本机能验证的内容 | CI / 实机记录 |
|---|---|---|
| 双语文档 | 链接、品牌素材、安装选择与证据边界 | 对照当前 README |
| macOS CLI | 真实脚本 + 离线系统/传输替身，校验/锁/回滚反例 | 原生 arm64/x64 仍待最终 head CI |
| Windows CLI | 原生 PowerShell，离线 release/下载反例 | 真包受控传输及原安装器 CI 待最终 head |
| Windows EXE/MSI | 本地不安装或卸载维护者环境 | 四个独立 runner（格式 × 两轮），结果待记录 |
| Mac 两种 DMG | WSL 无 hdiutil，不能声称实测 | 原生 DMG/签名/启动/重启与真实 CLI 待 CI |
| Mac Gatekeeper | 添加并核对 quarantine，无移除/绕过 | CI 记录 assessment；Finder 首次打开待实机 |
| 跨版本升级 | 离线失败回滚；CI 模拟旧 metadata 仅测替换事务 | 真旧版升级、格式迁移待实机 |
| 状态保留 | CI 隔离 marker 和来源配置；不触碰真实状态 | 未读/固定/去重跨版本保留待实机 |
| 真实 Agent | 已有本机模型读取与原生合成 UI 记录保留 | 本次包逐平台真实事件待实机 |
| 匿名公开安装 | Draft / preview 必须拒绝 | 需单独授权发布后验收 |

## 本轮本机验证结果

- Windows 隔离 target：fmt、clippy `-D warnings`、完整 Rust 502 通过、27 项 opt-in 忽略；custom-protocol 构建通过。
- 前端：check 无错误/警告，43 项 UI/品牌测试、build 通过；文档 8、仓库 4、发布 15、预览契约 3 项通过。
- macOS bootstrap：Linux/旧 Bash 离线 22 项通过，含 Rosetta、双语、资产/校验、锁、并发外部目录、回滚、状态保留等；不等于 Mac 安装通过。
- macOS staging：3 项通过，旧 manifest 默认兼容与 preview 标记分别覆盖。
- Windows PowerShell：真实 bootstrap 下载验证探针在隔离 MZ fixture 上通过，未运行安装器。
- 当前 TP-03 构建的原生模型 WebView2 验收通过，440/320 宽、720/420 高，无溢出/裁切，模型切换保持身份和计时。
- 完整 frontend audit：4 low；moderate 以上门禁通过。Rust audit 重跑 registry 检查完整并通过，保留 10 unmaintained、2 unsound 警告；全历史 secrets 扫描通过。
- 安全/架构与四角度对抗审查发现的 quarantine、目录所有权、失败附件保留及 PowerShell 参数覆盖均已修复并验证。

这些是源码与隔离开发环境证据；本次 PR 包、Finder 下载、真实升级和用户 Agent 的当前包实机结果仍按各自栏位填写。

## Windows 实机记录

状态：**未验证**。填写系统 build、芯片、应用版本/完整 SHA、脚本 SHA、安装包 SHA-256、
NSIS/MSI 类型、起始版本、目标版本、包/CLI 安装、系统启动、退出重启、卸载保留、跨格式迁移。
不要删除用户安装偏好来制造新安装通过。

## Mac 实机记录（arm64 / x64 分开）

状态：**两种架构均未验证**。每种分别记录 macOS、芯片、app/脚本/包 SHA、安装方式及升级起点。
Finder 下载与 Gatekeeper 提示、菜单栏、Dock、Retina/多屏/Space、通知焦点、点击跳转、退出重启、
真实未读/固定/去重保留逐项写通过/失败/未验证。ad-hoc 和未公证是现状，不当作 Developer ID。

## 真实 Agent 用例（每个平台单独记录）

记录 Agent 名称/版本与来源配置范围，不公开提示词、路径或凭据：

1. 新 turn 的真实标题、报告模型/未知值和开始证据；同项目多任务可用不同模型。
2. 思考、工具、等待模型响应保持进行中；采集刷新/改名不重置本轮时间。
3. 明确输入/批准及匹配 request/call 的解除；工具许可参数或静默不是批准证据。
4. 真实结束/失败、未读、固定、通知去重；改名不重建身份或再通知。
5. 从系统界面打开、退出重启与正常升级后复核；测试提醒不等于真实来源验收。

维护者稳定确认：**待确认**。最终 head 的 Windows Quality / Security 和两种 Mac 检查通过，
并收到维护者确认后才合并。确认分支 tip 已包含于 origin/main 且无未提交工作，再清理工作树/分支。
