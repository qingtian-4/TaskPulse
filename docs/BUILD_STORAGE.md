# 构建缓存与 smoke 测试目录

TaskPulse 的源码、安装包与本机 Cargo 缓存是不同的体积。2026-10-22 只读检查时，`src-tauri/target` 为 7.84 GiB，`debug` 与 `codex-check` 各约 3.06 GiB，两个开发 EXE 各约 87.5 MiB；这是多轮验证后的本机快照，不是用户安装体积，也不是长期固定上限。

## 开发构建

- 保留已有 `[profile.dev] debug = 0`：减少调试信息，不关闭开发断言与溢出检查；需要源码行号时可临时启用调试信息。Release 配置不随本轮更改。
- 托盘运行时继续使用 `scripts/verify.ps1` 的隔离 `codex-check`。两套 target 用于避免替换正在运行的 Windows EXE；不能为了省空间直接覆盖或删除正在使用的构建树。
- 优先使用 `scripts/disk-report.ps1` 检查。已有 `scripts/clean-build-cache.ps1` 默认 dry-run，删除须显式选择其允许的缓存项。不要直接递归删除整个 target，也不要批量删除工作树、测试证据、安全审计工具或用户 `.taskpulse` 状态。
- 本轮未批量清理历史缓存、旧测试目录或回滚 EXE，因此不声称又把整个 target 压回 1.99 GB。

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
$env:TASKPULSE_SMOKE_EXE = 'D:\Project\agenttray\src-tauri\target\codex-check\debug\agent-tray.exe'
& "$env:LOCALAPPDATA\hermes\node\bun.cmd" run test:smoke-cleanup
```

默认在 Windows 运行 14 项回归、跳过需 EXE 的 1 项；配置上述环境变量可运行全部 15 项。覆盖 Windows junction 不越界、真实 OS 文件句柄延迟释放、删除返回成功后的残留 / 重建、报告/截图留存、失败与重试边界、四个 CLI 的初始化边界及本地 / CI 门禁。真实 CLI 回归验证缺失 EXE、无效 EXE、9337 / 9339 端口占用时的非零退出、失败报告留存与临时目录回收；测试自己占用空闲端口，不附着到其他应用。

2026-10-22 使用当前 GNU 开发 EXE 的实际运行：

- bridge 两轮均通过，临时 EXE 目录清理成功。
- product native 通过，九张 DPR PNG 全部留存，`cleanup.complete=true`，不再留下本轮 WebView2 profile。
- integration native 通过，管理页 PNG 留存，包含合成 Hook 配置、来源移除撤销、并行请求及重启去重；临时目录清理成功。
- attention native 通过，720px 面板、思考不提醒、匹配请求解除、新 turn 撤回旧结果与 quota 测试提醒均通过，目录清理成功。
- 真实托盘 PID 66660 在上述测试期间未停止或重启，未修改真实 Agent 配置。这些是 TaskPulse 的合成生产者 / 原生渲染测试，不是 Kimi、TRAE、ZCode 或 WorkBuddy 宿主认证。

收尾时曾发现第二轮 product 的 rm 返回成功，但 `target/product-native-S0Kh9n` 仍有文件，早期 `cleanup.complete=true` 因此不足以证明回收。正式新增回归先失败，再补齐目录缺席核验。该中间轮及更早的已知残留不自动批量删除。

最终版本四轮退出码均为 0，且在整轮结束后再次独立检查，四个工作目录均不存在：

- `product-native-2026-10-22T03-02-30-177Z-ecBFBO`（9 PNG）；
- `integration-native-2026-10-22T03-02-40-715Z-oMsGoZ`（1 PNG）；
- `lifecycle-native-2026-10-22T03-02-55-937Z-fjHdRh`（2 PNG）；
- `integration-bridge-2026-10-22T03-03-03-784Z-upBVQ0`（result / cleanup）。

均在上述 evidence 根目录下，不提交二进制 / profile 到 Git。15 项回归全通过；Svelte Check 0 错误 / 0 警告、前端 Build、fmt、clippy `-D warnings`、Rust 403 通过 / 12 opt-in 忽略、UI Model 30、Docs 8、Release 8、Security Model 6、Integration Contract 3 通过；Secrets 无 findings。

本轮 `verify.ps1 -Frontend` 在依赖审计等待，随后改为分开验证上述检查。完整 `bun audit` 及 moderate 门禁均有界重试后超时，当前审计结果不完整，不宣称本轮安全门禁通过；没有移除或放宽 CI 的审计门禁。只读/隔离验证不代表安装器或真实 Agent 宿主重新认证。
