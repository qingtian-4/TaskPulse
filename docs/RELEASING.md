# Windows Alpha 发布流程

正在准备的下一候选为 `0.2.0-alpha.8`，包含当前审计修复和独立 Apple Silicon DMG 验收。
四处应用版本与 MSI 数值版本已同步；下文 Alpha.7 的标签、CI 和人工门禁属于历史证据，
不能用于证明 Alpha.8 已通过。新包仍先作为候选供维护者下载测试，公开发布前记录本次准确 SHA 的结果。
macOS 使用 [独立工作流](../.github/workflows/experimental-platforms.yml)，预发布标签默认选择
Apple Silicon；手动运行可选择 Intel/Linux。DMG、架构专用校验和和 manifest 与 Windows 文件分开，
只有该平台的全部检查成功才交付安装包。ad-hoc 签名不等于 Apple Developer ID 或公证，
原生窗口存在、首次启动/重启检查也不等于真实宿主交互；见 [Mac 验收说明](MACOS_DEVELOPMENT.md)。

CI 的 Actions 固定为完整提交 SHA，Bun / Rust 固定为经过验证的版本；升级通过普通变更审查完成。Windows 资产另外包含 `SBOM.cdx.json`（CycloneDX 1.6）、`DEPENDENCIES.json` 和 `THIRD_PARTY_NOTICES.txt`，由 `bun scripts/dependency-notices.ts dist/release x86_64-pc-windows-msvc` 从锁定依赖与本地许可证生成。清单注明源提交、锁文件哈希和范围，包含构建工具，不代表所有操作系统或原生捆绑组件已审完。公开发行前逐项处理 `reviewRequired`，核对许可证文本及原生组件的分发义务；生成成功本身不等于许可证审查通过。

当前候选为 `0.2.0-alpha.7`；不可变标签 `v0.2.0-alpha.7` 已指向
`b5dc64a760bae916c9d40b147616f9720405887b`。main 质量/安全 CI 和
[发布运行 39636404358](https://github.com/qingtian-4/TaskPulse/actions/runs/39636404358)
均通过，生成的 Draft pre-release 仍未公开。资产校验、独立安装器 CI 与限制见
[候选验收记录](ALPHA7_CANDIDATE_VALIDATION_2026-10-22.md)。第 3 项独立实机验收
已按维护者要求暂缓，不计为通过；不因此跳过下方人工门禁。

版本与包名不沿用上游稳定发布语义；内部 executable 仍为 `agent-tray`，产品名/安装包为 TaskPulse。

## 发布前

1. 在 codex/* 分支完成修改、验收、提交，经 PR、Windows quality 和 Security checks 合入 main。
   保留上游版权和提交历史，不直接覆盖默认分支或强推。
2. 同步 `package.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock` 与
   `src-tauri/tauri.conf.json`；更新 `docs/releases/<版本>.md` 和 CHANGELOG。
3. 执行 `bun run release:check`、`bun run test:release`、完整质量检查和原生验收。
   **不能用版本输入仅修改 Release 标题。**
4. 确认远端 main 包含对应提交并取得维护者明确确认后，对明确的 main SHA 创建不可变标签。
   以下仅为后续新版本模板；Alpha.7 已有标签，不要重复创建、移动或覆盖它：

```bash
git tag -a <next-pre-release-tag> <verified-main-sha> -m "TaskPulse <next-version>"
git push origin refs/tags/<next-pre-release-tag>
```

不要移动已经发布的标签。失败时修复新版本，或只重跑同一个未发布标签的构建。
生成 Release 的手动运行仍必须提供已经存在的标签，不能用 main 作为版本。
`scripts/release.sh` 现在只是只读检查入口，不再替维护者提交/打标签/推送。

仅构建 CI 附件时，在 `Windows Alpha release` 的 `commit` 输入填写完整的当前
main SHA（40 位）；它优先于 `tag` 输入。preflight 必须确认 checkout、origin/main
和输入完全一致，随后执行相同的安全检查、MSVC 构建及 NSIS/MSI 独立安装验收。
该模式跳过 Draft job，不创建 Tag 或 Release；manifest 的 `tag=null`、
`build_kind=commit-candidate` 和完整源 SHA 区分同版本的开发构建。
四处应用版本及 MSI 映射仍须一致；这不是新版本发布，也不证明旧 Alpha.7 的升级兼容。

仅修复验收探针时，CI 附件模式可指定 `probe_commit`：必须是当前工作流自身的完整
SHA，并同时提供有效的 main `commit`；标签发布拒绝该覆盖。产品构建仍使用 main SHA，
只有安装验收脚本使用该探针提交，证据分别记录产品 manifest 与 `probe_commit`。
NSIS/MSI 各运行两轮，每轮是独立 fresh runner，不能将重跑失败 job 计为连续通过。
探针只在 IPC 前等待已复现的 CDP 导航上下文销毁；真实 IPC、JS 异常和业务断言失败不重试。

## 工作流交付

`preflight → Rust/Git 安全检查 → Windows 完整质量门禁/MSVC 构建 → 每种安装格式独立 runner 验收 → 草稿预发布`。
普通 build jobs 仅有 contents:read；只有最后草稿 job 有 contents:write。
版本/标签/SHA 不一致、缺失/重复安装包、校验失败、安装验收失败都阻止草稿生成。

文件统一为 `TaskPulse_<版本>_windows_x64_setup.exe`、
`TaskPulse_<版本>_windows_x64.msi`，附平铺 `SHA256SUMS.txt` 和源 SHA manifest。
没有验证 Portable，不发布改名的 Debug EXE。代码签名与 SHA256 是不同保证；
当前没有配置签名证书，发布说明必须诚实说明未知发布者提示。

安装器以正常安装权限运行；原生应用的启动、IPC 和重启验收通过 CI 专用 gsudo
子进程降到普通用户权限。WebView2 150+ 在提权进程中忽略环境调试参数，不能把
调试端口连接超时直接断言为应用故障，也不能因此跳过 UI/IPC 门禁。此做法与
[Tauri 官方验收](https://github.com/tauri-apps/tauri/blob/dev/.github/workflows/test-api-e2e.yml)
一致；不降级运行时、不改产品 CSP/安全策略、不在用户电脑安装 gsudo。
失败时上传分阶段结果、应用 stdout/stderr、最后一次端口查询与页面信息；即使
首个检查失败，result.json 也必须存在。安装器和启动等待都有时限。

NSIS 和 MSI 使用 fail-fast=false 的独立 runner 矩阵，各自从干净配置开始，证据
分别为 windows-installer-evidence-nsis/msi。Tauri 安装器会保留目录偏好，MSI 可
从注册表复用上次 NSIS 路径；在同一环境先装 NSIS 再装 MSI 是跨格式迁移测试，
不能冒充两个独立的首次安装。不要为测试删除用户偏好。两种格式全部通过才允许
草稿；跨格式目录/状态迁移和自定义目录优先级仍需单独验收。

安装器和版本命令统一由持有原生 Process 句柄的 `installer-process.ps1` 等待并
读取真实退出码；不能把 Start-Process 快速退出后返回的 null ExitCode 当成失败
或成功。`installer-process.test.ps1` 在 Windows CI/本机检查快速零/非零退出、
输出和超时，不执行任何安装器。版本输出和退出码均保存在证据中。

自有进程检查使用文件句柄解析的最终路径，而非仅 GetFullPath 字串；Windows
TEMP 常含 8.3 别名，同一文件的长短名称不能误报成无关进程。不允许退化为按
进程名停止。IPC 证据只允许一个已验证结果对象；异步 .NET 调用必须抑制辅助
返回值。重启前等待旧 CDP 端点关闭，避免误连上一次浏览器实例。

### MSI 数字元信息

MSI 不接受 `alpha.N`。应用、标签与四文件仍保持同一 SemVer；WiX 单独使用
`major.minor.patch.build`，build=alpha 1000/beta 2000/rc 3000 + 序号（0–999），
例如 alpha.2 为 `0.2.0.1002`。release:check/preflight 校验映射，manifest 记录
msi_product_version；安装器验收读 MSI Property 和应用 --version，两者不混为一谈。

Windows Installer 的升级比较忽略第四字段；这个映射不是跨预发布升级顺序的保证。
必须实际验收同核心版本升级/重装以及稳定版本迁移，不能因为 build 数字递增就
勾选升级通过。参见 [Tauri WiX 配置](https://v2.tauri.app/reference/config/#wixconfig)
与 [Microsoft ProductVersion](https://learn.microsoft.com/en-us/windows/win32/msi/productversion)。

Linux/macOS 改为独立的手动 experimental artifact 工作流，不依赖 Apple 证书，
不阻断 Windows；这些构建不等于安装/签名/公证或稳定支持验收。

## 从草稿到可下载 Alpha 的人工门禁

- [x] 发布候选依赖审计：执行 `bun audit` 并记录全部告警，`bun run audit:frontend`
  阻止中等及以上告警；`bun run audit:rust` 和 `bun run audit:secrets` 另行门控。
  Alpha.7 main 和标签运行通过；仍有前端 1 low、Rust 10 maintenance / 2 unsound 告警，
  见[安全审查](internal/reports/SECURITY_AUDIT_2026-10.md)。不能把阈值通过称为零风险。
- [x] Alpha.7 准确候选提交 `b5dc64a` 的 main 质量/安全检查及两种安装器独立 CI 验收通过。
  见[发布运行](https://github.com/qingtian-4/TaskPulse/actions/runs/39636404358)和
  [原始证据索引](ALPHA7_CANDIDATE_VALIDATION_2026-10-22.md)，不复用 Alpha.6 结果。
- [ ] 干净 Windows 10/11 VM：无开发工具，检查 WebView2 有/无与网络受限场景。
- [ ] 上一可安装版本升级：来源、白名单、未读、固定、去重状态保留。
- [ ] EXE/MSI 格式迁移：卸载后目录偏好、显式自定义目录优先级、用户状态保留。
- [ ] 真 Agent 新任务：开始、思考、明确结束、失败、重命名、真实批准与解除。
- [ ] 多任务与失联：不漏掉范围内新执行、不把源错误/内部 Guardian 当完成。
- [ ] 长运行、多个提醒、重启恢复回查；记录事件到 UI/通知的 P50/P95 与样本数量。
- [x] 明确 unsigned 边界；安装器均为 `NotSigned`，四项资产 SHA/大小与 Draft 一致，
  应用 `0.2.0-alpha.7` / MSI `0.2.0.1007` 已核对。SHA256 不等于签名或维护者风险接受。
- [ ] 记录系统/Agent 版本、日期、证据及未验收限制，再手动 Publish pre-release。

上述干净系统、升级/格式迁移与真实 Agent 项目目前暂缓，不计为通过。尚无新的焦点、
长运行或真实事件延迟样本；测试 fixture 和 CI 初始空状态重启不能补足它们。
当前决策是保持 Draft，未公开；Publish 需要维护者另行明确确认。

自动化同版本重装不是跨版本升级，GUI 渲染不是焦点验证，单元测试不是实机事件。
任何未完成项要记录原因和范围；不自动发布稳定版，也不把缺少证据写成通过。

2026-10-19：旧草稿包含本次审计发现的旧依赖。Alpha.7 分支的安全修复不回写旧标签/
安装包；新候选必须使用新标签重新验收。在完成
[开源交付清单](OPEN_SOURCE_CHECKLIST.md) 前，不公开旧草稿来冒充修复后的版本。
