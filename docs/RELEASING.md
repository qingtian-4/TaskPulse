# Windows Alpha 发布流程

版本为 `0.2.0-alpha.3`，标签为 `v0.2.0-alpha.3`。版本与包名不沿用上游稳定发布
语义；内部 executable 仍为 `agent-tray`，产品名/安装包为 TaskPulse。

## 发布前

1. 在 codex/* 分支完成修改、验收、提交，经 PR 和 Windows quality CI 合入 main。
   保留上游版权和提交历史，不直接覆盖默认分支或强推。
2. 同步 `package.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock` 与
   `src-tauri/tauri.conf.json`；更新 `docs/releases/<版本>.md` 和 CHANGELOG。
3. 执行 `bun run release:check`、`bun run test:release`、完整质量检查和原生验收。
   **不能用版本输入仅修改 Release 标题。**
4. 确认远端 main 包含对应提交后，对明确的 main SHA 创建不可变标签：

```bash
git tag -a v0.2.0-alpha.3 <verified-main-sha> -m "TaskPulse 0.2.0-alpha.3"
git push origin refs/tags/v0.2.0-alpha.3
```

不要移动已经发布的标签。失败时修复新版本，或只重跑同一个未发布标签的构建。
手动运行 Release 工作流也必须提供已经存在的标签，不能用 main 作为版本。
`scripts/release.sh` 现在只是只读检查入口，不再替维护者提交/打标签/推送。

## 工作流交付

`preflight → Windows 完整质量门禁/MSVC 构建 → 独立 runner 安装验收 → 草稿预发布`。
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

- [ ] CI：两个真实安装器安装、版本、内嵌 UI/IPC、重启、同版本重装、卸载通过。
- [ ] 干净 Windows 10/11 VM：无开发工具，检查 WebView2 有/无与网络受限场景。
- [ ] 上一可安装版本升级：来源、白名单、未读、固定、去重状态保留。
- [ ] 真 Agent 新任务：开始、思考、明确结束、失败、重命名、真实批准与解除。
- [ ] 多任务与失联：不漏掉范围内新执行、不把源错误/内部 Guardian 当完成。
- [ ] 长运行、多个提醒、重启恢复回查；记录事件到 UI/通知的 P50/P95 与样本数量。
- [ ] 通过签名或明确 unsigned 边界；核对每个资产 SHA、实际版本、发布说明。
- [ ] 记录系统/Agent 版本、日期、证据及未验收限制，再手动 Publish pre-release。

自动化同版本重装不是跨版本升级，GUI 渲染不是焦点验证，单元测试不是实机事件。
任何未完成项要记录原因和范围；不自动发布稳定版，也不把缺少证据写成通过。
