# 构建缓存与 smoke 测试目录

源码、分发安装包与本机 Cargo 缓存体积不同。缓存包含多轮编译和独立验证目标，不代表用户安装体积。

## 开发构建

- 保留已有 `[profile.dev] debug = 0`：减少调试信息，不关闭开发断言与溢出检查；需要源码行号时可临时启用调试信息。Release 配置不随本轮更改。
- 托盘运行时继续使用 `scripts/verify.ps1` 的隔离 `codex-check`。两套 target 用于避免替换正在运行的 Windows EXE；不能为了省空间直接覆盖或删除正在使用的构建树。
- 优先使用 `scripts/disk-report.ps1` 检查。已有 `scripts/clean-build-cache.ps1` 默认 dry-run，删除须显式选择其允许的缓存项。不要直接递归删除整个 target，也不要批量删除工作树、测试证据、安全审计工具或用户 `.taskpulse` 状态。

## Smoke 的统一生命周期

`integration-bridge-smoke.ts`、`native-product-smoke.ts`、`native-integration-smoke.ts`、`native-attention-regression-smoke.ts` 共用 `scripts/smoke-workspace.ts`：

1. 只为本轮创建 `target/<测试名>-<随机后缀>`，从初始化、复制 EXE、端口检查开始即进入统一错误边界。
2. 成功写 `result.json`；初始化或断言失败时保证有 `failure.json`，不覆盖脚本已经收集的详细失败诊断。
3. 只通过测试持有的 ChildProcess 句柄停止自己创建的进程，不按名称终止真实托盘或 WebView2。
4. 把 result / failure 以及本轮根目录下的所有 PNG（包括九张 DPR 图）复制到 `target/ui-smoke/evidence/<测试名>-<UTC时间>-<随机后缀>/`。不导出 profile、配置、事件 journal、其他任意 JSON 或符号链接。
5. 只有进程退出、证据复制完成且目标路径确认仍是 target 的直接子目录，才清理本轮目录。Windows 临时锁使用显式 250ms 间隔、约 10秒有界重试，不依赖 Bun 是否实现 `fs.rm.maxRetries`。返回成功后还核对立即消失及连续 1秒的缺席；若仍有残留或被延迟写入重建，继续重试，而不是仅凭 rm 返回值标为完成（最后的静默确认最多额外 1秒，文件系统 I/O 本身不提供硬超时）。
6. 证据复制失败、没有报告、进程无法退出或清理最终失败，均返回非零退出码，输出保留目录的绝对路径；不把清理异常静默算成功。复制成功时另写 `cleanup.json`，记录 `complete`、`retained` 与已保存文件。

验收应同时检查脚本退出码和 `cleanup.json`；`result.json.passed` 只是功能断言，不覆盖后续清理失败。断电、OS 强制终止或运行时崩溃无法保证执行清理，不会因此自动清扫别人的目录。

## 验证命令与实测

```powershell
& "$env:LOCALAPPDATA\hermes\node\bun.cmd" run test:smoke-cleanup
# 可选：对已构建的隔离 EXE 验证真实 CLI 提前失败路径。
$env:TASKPULSE_SMOKE_EXE = (Resolve-Path 'src-tauri\target\codex-check\debug\agent-tray.exe').Path
& "$env:LOCALAPPDATA\hermes\node\bun.cmd" run test:smoke-cleanup
```

默认在 Windows 运行 14 项回归、跳过需 EXE 的 1 项；配置上述环境变量可运行全部 15 项。覆盖 Windows junction 不越界、真实 OS 文件句柄延迟释放、删除返回成功后的残留 / 重建、报告/截图留存、失败与重试边界、四个 CLI 的初始化边界及本地 / CI 门禁。真实 CLI 回归验证缺失 EXE、无效 EXE、9337 / 9339 端口占用时的非零退出、失败报告留存与临时目录回收；测试自己占用空闲端口，不附着到其他应用。
