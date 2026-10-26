# Alpha.8 Windows / Apple Silicon 候选验收

日期：2026-10-27（Asia/Shanghai）。结论：两平台候选包及自动化验收已完成，可供维护者下载实机测试。
Release 仍为 Draft pre-release，未公开发布，未宣称稳定支持或完成真实 Mac Agent 验收。

## 准确源码与下载

- [PR #25](https://github.com/qingtian-4/TaskPulse/pull/25) 已合并。
- 不可变标签：`v0.2.0-alpha.8`。
- 两平台源提交：`fe48e69b9864eeaddcd68e84a4928c6b4176500c`。
- 四处应用版本：`0.2.0-alpha.8`；MSI ProductVersion：`0.2.0.1008`。
- [Alpha.8 候选 Release](https://github.com/qingtian-4/TaskPulse/releases/tag/untagged-bd2771b9fa02f8dc065b)：
  ID `424516447`，Draft / pre-release，13 项资产；需登录维护者账号查看草稿。
- [Windows 附件](https://github.com/qingtian-4/TaskPulse/actions/runs/40360274223/artifacts/12668203350)：EXE/MSI 二选一。
- [Apple Silicon 附件](https://github.com/qingtian-4/TaskPulse/actions/runs/40360274417/artifacts/12668502437)：
  解压后进入 `dist/macos`，选择 `TaskPulse_0.2.0-alpha.8_macos_arm64.dmg`。

Mac DMG、校验和和 manifest 已追加到同一草稿，Mac 依赖清单与许可证文件加 `macos-arm64-` 前缀，
避免覆盖 Windows 文件。附加前核对两个工作流成功、源 SHA、归档 digest、DMG digest 和草稿状态；
上传返回的 digest 与本地文件一致。操作没有公开 Release，也没有移动旧标签。

## 测试与审查

本轮提交 `3cdd2b9` 增加原生 Mac 打包/验收与版本同步；`3bb3441` 修复 Mac 严格检查发现的六项
旧写法和注释问题，并补齐下载指南。没有降低 clippy 要求或绕过失败。

- 本地 Windows：`verify.ps1 -Frontend`、原生 WebView2 产品验收和 DMG 打包脚本回归通过。
  Mac 专属规范修复后再次通过 Rust 全量测试、fmt、clippy 和构建；文档变更另通过文档/仓库检查。
- [准确 main 的 Windows quality](https://github.com/qingtian-4/TaskPulse/actions/runs/40358427661)
  与[安全检查](https://github.com/qingtian-4/TaskPulse/actions/runs/40358426007)均通过后才创建标签。
- [标签 Windows 发布流程](https://github.com/qingtian-4/TaskPulse/actions/runs/40360274223)通过：
  Rust 480 项通过、21 项 opt-in 忽略；前端、格式、clippy、发布规则、事件接收及安全门禁通过，
  使用 MSVC 生成实际 NSIS/MSI 安装器。
- [标签 Mac 流程](https://github.com/qingtian-4/TaskPulse/actions/runs/40360274417)通过：
  Rust 467 项通过、16 项 opt-in 忽略；前端、格式、clippy、发布规则、观察 Hook 脚本和安全门禁通过。
  `macos-15` runner / `aarch64-apple-darwin`，实际系统 macOS 15.7.9 / arm64。

Windows 原生产品测试使用隔离合成事件，覆盖不同窗口大小、DPR 1/1.5/2、筛选、未读/固定、
来源管理和独立测试提醒；不改变真实宿主配置，不冒充真实 Agent 生命周期或混合 DPI 实机测试。

## 安装验收证据

Windows 四轮分别使用全新 runner，全部通过安装、实际安装路径的版本/原生 UI 与 IPC、
重启、同版本重装、卸载及状态保留检查。所有原始 ZIP 已按 GitHub digest 下载核验，
四份 `result.json` 均为 `phase=passed`，不是仅依据绿色工作流图标。

| 格式 / 轮次 | Job | 证据 artifact |
|---|---|---|
| NSIS 1 | 121711386966 | 12668393162 |
| NSIS 2 | 121711386963 | 12668821535 |
| MSI 1 | 121711386968 | 12669230992 |
| MSI 2 | 121711386988 | 12668147776 |

四份结果均为 `embedded=true, onboarding=false, tasks=0, prompts=0`，
`restart/reinstall/uninstall=true`；`cross_version_upgrade/format_migration/focus=not_verified`。
安装器签名均为 `NotSigned`。

Mac 证据 artifact `12669585330`：DMG 只读挂载、复制到隔离安装目录、严格 ad-hoc 签名、
bundle 身份、arm64 架构、版本及资源验证通过。首次启动/重启分别存在 412×400 的原生可见窗口，
存活 10 秒，二次启动激活已有实例且不产生第二个 GUI writer，显式空来源配置未被改写。
证据明确保留 `ui_clicks_verified=false, gatekeeper_download_verified=false, real_agent_events_verified=false`。
该候选未 Apple 公证。首轮分支 Mac 验收也通过，但最终交付仅使用标签 SHA 的产物。

## 安装文件校验

从最终 Actions ZIP 提取的文件与源 manifest、平铺校验和及 Release asset digest 一致。

| 文件 | 字节 | SHA256 |
|---|---:|---|
| `TaskPulse_0.2.0-alpha.8_windows_x64_setup.exe` | 3730487 | `bdc9e1af742486ea2ce649b1b9c0198f7a55964685a213d313a258b1d40537fa` |
| `TaskPulse_0.2.0-alpha.8_windows_x64.msi` | 5263360 | `b8ce51e42ef86a883455afc61684f132656e2bd347446cb85b7ebfb9d1b147c6` |
| `TaskPulse_0.2.0-alpha.8_macos_arm64.dmg` | 5287117 | `cd1063b7e79d62d7b15a6415933fc4512e5e4d52bb0d8cba9853e055e9a13b5e` |

Windows 包 artifact `12668203350` 的 ZIP SHA256 为
`2be14f05052366c2f9a53cc826e7e21edaf0fd0fd7f538c5f349f97ce53ecf7e`；
Mac 包 artifact `12668502437` 为
`f39056b52768b769ca43b47234e6d45424362ab0346db177f5b6dcf1545d7f34`。
校验和识别文件，不代表开发者身份签名或用户已读。

## 残余问题与公开发布门禁

- 前端完整 `bun audit` 仍有 3 low：SvelteKit 两项和 cookie 一项；moderate+ 门禁通过不等于零风险。
- Rust 漏洞/撤回检查完整通过，保留 10 maintenance 与 2 unsound 警告（glib 0.18.5、rand 0.7.3）；
  完整历史密钥扫描通过。未使用 advisory 忽略基线。
- Windows 依赖清单 396 项、29 项需复核；Mac 382 项、38 项需复核，包含缺少许可证文本的条目。
  SBOM / notices 生成成功不等于分发义务已审完，公开前仍需补齐文本和审查原生组件范围。
- 用户 Mac 的菜单栏、真实提问/审批与匹配解除、提醒焦点、Finder 下载后的 Gatekeeper、
  长时间运行与升级保留状态，需按[候选测试指南](CANDIDATE_TESTING.md)实测。
- 干净 Windows 10/11、WebView2 缺失/网络受限、跨版本/格式迁移仍未完成；CI Windows runner 不能代替这些场景。

因此保留 Draft。公开预览版及后续稳定支持应以本次证据与剩余实机/许可证结果作决定，
不复用旧 Alpha.7 验收结论。Linux/Intel 本轮未执行，不提高支持等级。
