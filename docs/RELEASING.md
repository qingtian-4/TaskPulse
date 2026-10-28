# Windows Alpha 发布流程

TaskPulse 的发布流程生成 **Draft pre-release**，公开发布由维护者单独确认。源码合并、CI 通过、测试附件和公开 Release 是不同状态。Linux/macOS 保持独立实验构建，不自动继承 Windows 的支持或验收结论。

## 发布前

1. 在功能分支完成审查、测试和适用实机验收。当前 PR head 的 Windows quality / Security 必须通过；维护者要求测试后合并时保持 Draft。
2. 四处版本保持一致：`package.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock`、`src-tauri/tauri.conf.json`。同步 WiX 数值版本、CHANGELOG 和 `docs/releases/<版本>.md`，参考[说明模板](releases/TEMPLATE.md)。
3. 运行 `bun run release:check`、`bun run test:release`、完整质量检查、`bun audit`、`bun run audit:frontend`、`bun run audit:rust` 和 `bun run audit:secrets`。
4. 审阅完整审计结果：阈值通过不等于零风险；Rust 注册表检查不完整必须阻止候选发布。维护/unsound 告警保留并审阅。密钥扫描覆盖完整 Git 历史且输出脱敏。
5. 核对准确的 main SHA，并取得维护者对版本和发布动作的明确确认后，创建新的不可变预发布 tag。不要移动旧标签或覆盖已公开资产。

`scripts/release.sh` 是只读检查入口，不代替维护者提交、打标签或推送。手动运行 release.yml 也必须提供已存在的预发布标签；不得只覆盖 Release 标题的版本号。

## 工作流交付

`preflight → 安全检查 → Windows 质量检查与 MSVC 构建 → 独立安装器验收 → 草稿预发布`。

Actions 固定完整提交 SHA，Bun/Rust 固定版本。常规 jobs 只有 `contents: read`，最后草稿 job 才有发布所需写权限。版本/标签/SHA 不匹配、缺包、重复包、校验失败或安装验收失败都阻止草稿生成。

Windows 文件名：

- `TaskPulse_<版本>_windows_x64_setup.exe`
- `TaskPulse_<版本>_windows_x64.msi`
- `SHA256SUMS.txt`、`release-manifest.json`
- `SBOM.cdx.json`、`DEPENDENCIES.json`、`THIRD_PARTY_NOTICES.txt`

依赖清单由 `scripts/dependency-notices.ts` 按锁文件及实际平台生成。公开前逐项处理 `reviewRequired`，核对原生组件和许可证分发要求。不要把 Debug EXE 改名当作 Portable 发布。SHA-256 不是发布者签名；当前未配置 Windows 签名证书，说明必须保留未知发布者提示。

## 合并前预览与 main 候选

`installation-preview.yml` 对同仓库 PR 的准确 head SHA 构建，只输出 CI artifacts。manifest 标记 `build_kind=pr-preview`、`tag=null`，不创建 Tag 或 Release。公共 CLI 拒绝这些预览；测试传输替身只在 CI 进程内生效，不给生产脚本增加跳过校验或任意镜像入口。

`release.yml` 的 `commit` 输入接受完整的当前 main SHA，优先于 tag。preflight 核对 checkout、origin/main 与输入完全一致，然后运行与标签模式相同的安全、构建和独立安装验收。输出 `build_kind=commit-candidate`、`tag=null`，跳过 Draft job。这不是新版本，也不证明旧包升级兼容。

仅修复验收探针时可以提供 `probe_commit`：它必须等于当前工作流的完整 SHA，并同时提供有效 main `commit`；标签发布禁止覆盖。产品源码和探针源码分别记录。不能把预览路径变成绕过正式 main/tag 校验的入口。

## Windows 安装器验收

- NSIS/MSI 使用 `fail-fast=false` 的独立 fresh runner 矩阵，每种格式两轮。各自产生 evidence artifact，两种格式全部通过才允许草稿生成。
- Tauri 安装器会保留安装目录偏好。同一环境顺序安装 NSIS/MSI 属于跨格式迁移，不能冒充两个独立首次安装；不要删除偏好来让测试通过。
- 安装器保持正常安装权限；已安装应用的启动、IPC 和重启探针通过 CI 专用 gsudo 在 medium integrity 运行。WebView2 150+ 在提权进程中可能忽略调试开关，不能为此弱化 CSP 或跳过原生验收。
- 使用 `installer-process.ps1` 持有真实 Process 句柄并读取退出码，有界等待。快速退出后的 null ExitCode 不是成功或失败证据。
- 所有停止/清理操作只针对当前 CI 拥有的程序，按文件句柄解析的最终路径比较，包含 Windows 8.3 别名；不按进程名批量停止。
- 原生 IPC 验证返回一个结果对象，不接受 .NET 异步辅助返回值形成的数组。真实 IPC/业务断言失败不自动重试；仅等待已复现的导航上下文销毁。
- 失败保留分阶段 result/error、应用输出、端口和页面信息；重启前等待旧 CDP 端点关闭。非安装回归由 `installer-process.test.ps1` 和发布测试集覆盖。

### MSI 数字元信息

应用/tag/四处版本使用同一 SemVer。MSI 使用 `major.minor.patch.build`，build 为 alpha 1000、beta 2000 或 rc 3000 加序号（0–999）。例如 `0.2.0-alpha.8` 映射为 `0.2.0.1008`。

Windows Installer 升级比较忽略第四字段，因此数字递增不能替代跨预发布升级验收。版本检查、manifest 和实际安装的 `--version`/MSI Property 分别核对。

## macOS 与命令行安装

macOS ARM/Intel 在对应原生 runner 构建；DMG、架构专用 manifest/校验和及许可证清单使用不同文件名，避免覆盖 Windows 资产。ad-hoc 签名不等于 Developer ID 或 Apple 公证，详见 [Mac 开发指南](MACOS_DEVELOPMENT.md)。

`install.ps1` 和 `install.sh` 消费同一批公开 Release 产物，默认稳定版，预发布显式选择。资产命名或格式变化必须同步脚本和 `bun run test:release`。离线测试不等于真实公开下载安装；发布后人工核对匿名 `--download-only`/`-DownloadOnly` 路径。

## 从草稿到可下载 Alpha 的人工门禁

按[公开发布清单](OPEN_SOURCE_CHECKLIST.md)完成干净机器、真实 Agent、升级、焦点与长运行测试。每项记录候选 SHA、包哈希、OS/Agent 版本、步骤和实际结果。暂缓、不适用和未验证要明确记录，不能写成通过。

维护者确认后才手动 Publish pre-release。发布说明仅简体中文/英文，包含变化、系统要求、安装/升级、签名状态、已知限制和测试范围。应用缺少的能力留在[路线图](ROADMAP.md)，不写成已交付。
