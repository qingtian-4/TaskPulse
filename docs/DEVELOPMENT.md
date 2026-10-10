# Development

TaskPulse = AgentTray fork（MIT，保留 attribution）。Windows 优先，Linux/macOS 尽量不破坏。

## 环境（本机已验证）

- Windows 10 + git-bash
- Rust：rustup `stable-x86_64-pc-windows-gnu`（**本机无 MSVC**，用 GNU target）
- C 编译器：**winlibs MinGW-W64**（`C:\tools\mingw64`），含 `gcc/as/windres/dlltool`
- 链接：rustc 对 windows-gnu 默认使用**内置 rust-lld** + `-C link-self-contained=yes`
- bun（`npm i -g --allow-scripts=bun bun`）；WebView2 runtime（系统已装）

构建前设置：

```bash
export PATH="/c/tools/mingw64/bin:$HOME/.cargo/bin:$PATH"
export CC="gcc"
export CXX="g++"
export RUSTFLAGS="-C link-self-contained=yes"
```

> **关键坑（务必遵守）**
> - **不要设 `-C linker=`**：rustc 1.98 的 windows-gnu target 用内置 rust-lld；显式指定 linker（`zig-cc`/`gcc`）反而因无法解析而报 "linker not found"。RUSTFLAGS 只用于 `-C link-self-contained=yes`。
> - `CC=gcc`（winlibs）给 cc crate 编译 C 用；工具链自带的 `self-contained/dlltool.exe` 需要 `as.exe`，所以 **windres/dlltool/as 用 mingw64 的**（放 PATH 首位即可）。
> - 如果换用 MSVC：装 VS Build Tools（C++ workload）后 `rustup default stable-x86_64-pc-windows-msvc`，去掉 RUSTFLAGS。
> - 另备选：`C:\tools\zig-windows-x86_64-0.14.0\zig.exe`（GNU 目标的交叉 C 编译器），但 zig 也缺 dlltool，需配 binutils。

## 常用命令

```bash
# 前端
bun install
bun run check          # svelte-check + tsc
bun run build

# 后端
cd src-tauri
cargo fmt --check
cargo clippy -- -D warnings
cargo test             # 全部单元测试（无需真实服务器，FakeSshExecutor）
cargo check

# 运行（开发）
cd .. && bun run tauri dev        # 或: cd src-tauri && cargo tauri dev
```

## 测试

核心逻辑全部可离线测试：`cargo test`。

Codex WSL 的真实只读采集与原生管理页验收分别使用 `live_wsl_codex_snapshot`、`scripts/native-wsl-codex-smoke.ts`；不发送 Agent 任务，后者临时启用来源后默认恢复设置。安装要求、执行命令和数据降级规则见 [WSL_CODEX.md](WSL_CODEX.md)。

- `providers/remote_tmux/`：tmux/ps 解析、进程树检测、SSH 失败语义、DISCONNECTED 阈值、恢复、并行轮询（FakeSshExecutor 脚本化响应，无需 Server1/Server3）。
- `store/`：EXACT>HOOK>INFERRED 合并、同 pane 去重、RECENT TTL、新运行重置。
- `transition/`：RUNNING→WAITING_INPUT / SUCCEEDED / FAILED / 推断完成；只通知一次；断连/恢复。
- `domain/`：状态优先级、置信度、序列化。
- `heuristics/`：hook 状态 × 扫描数据交叉验证（旧 + 新模型两套测试）。

## 配置

- 远端：`~/.taskpulse/config.toml`（首启不连接远端，按需添加 SSH 别名）
- 新来源配置：同文件的 `[dashboard]`；齿轮管理窗口保存热更新，首次确认前不采集。旧范围首存备份迁移。
- 本地状态目录：`~/.agent-monitor/`（兼容）+ `~/.taskpulse/`
- 最近完成缓存：`~/.taskpulse/recent.json`（版本化 JSON，仅保存 RECENT，原子替换）
- 日志：`RUST_LOG=debug` 前缀运行可看细节（env_logger）

## Git 工作流

- 当前 `codex/*` 分支，小逻辑 commit；不自动 push / 不开 PR。
- LICENSE（MIT）与上游 attribution 保留。

## 已知限制（MVP）

- `tauri dev` 的交互验证（托盘出现、popup、通知）需要真实桌面会话——headless 环境只能验证到 `cargo test` + `bun run build` 层面。本机已实际运行 `agent-tray.exe` 验证远端轮询/检测/通知（见 README）。
- tmux 3.0a 无新版 `-F` 特性；pane_title 含 tab 的行会被解析器跳过（字段数校验）。
- **无需担心 SSH 管道缓冲**：快照输出在忙碌服务器上可达 ~96KB，`SystemSshExecutor` 已改为边读边等（并发读线程），不会死锁。
- 孤儿 hook 文件清理（session-id 交叉校验）暂由 10 分钟 STALE_TTL 覆盖。
- Codex/Hermes/DeepSeek 的端到端验证取决于本机安装情况；adapter 边界、fixture 测试与文档已完成，真实限制在 README/LOCAL_AGENTS.md 记录。

## 真实远端冒烟（可选，需要 SSH 可达 Server1/Server3）

```bash
cd src-tauri && cargo test --bin agent-tray live_remote_snapshot -- --ignored --nocapture
```

会打印当前 Server1/Server3 的真实快照解析结果（运行中的 workload 任务 + tp-run 精确任务 + 主机健康）。

## 任务面板与原生窗口验收

`powershell -ExecutionPolicy Bypass -File scripts/verify.ps1 -Frontend` 使用隔离 target，托盘运行时也可以执行。补充 `bun run test:ui-model` 与 WSL Python 的 `scripts/test-codex-metadata.py`。

`scripts/ui-smoke.ts` 是隔离 Edge CDP（9227）+ Vite（5189）+ 模拟 IPC 的布局/交互验收，不能证明原生按钮、系统窗口或通知真的可用。

`scripts/native-ui-smoke.ts` 连接真正的 Windows WebView2（9337）。先构建前端，再在已停止本项目托盘时，按本机 GNU/MSVC 工具链设置构建：

```powershell
bun run build
$env:TAURI_CONFIG = '{"build":{"devUrl":null}}'
cargo build --manifest-path src-tauri/Cargo.toml --bin agent-tray
Remove-Item Env:TAURI_CONFIG
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9337 --remote-debugging-address=127.0.0.1'
Start-Process -FilePath '.\src-tauri\target\debug\agent-tray.exe' -ArgumentList '--show' -WindowStyle Hidden
Remove-Item Env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
bun scripts/native-ui-smoke.ts
```

该脚本要求已有配置并启用本地 Codex；它会暂时停用/恢复来源、保存同样的项目范围，并发送一次真实通知，**不是只读脚本**。不修改 Agent 会话，也不标记已查看。结果和实际窗口截图保存在忽略提交的 `src-tauri/target/ui-smoke/`。首次保存旧配置会产生 TaskPulse 的迁移备份。完成后通过托盘 Quit 退出调试进程，再不带调试环境变量启动；切勿长期开放 CDP。

原生验收还检查近期/较早关注无重叠、归档中无运行任务、即时开关、不保存草稿时关闭确认，以及刷新等待来源真实返回。运行中的托盘升级会将长期不活跃的旧关注归档，并在 attention v3 迁移前创建 `attention.json.pre-recency.bak`；测试不能为迁就截图而标记任务已查看或清空用户记录。已固定但来源未知的旧标记保留在较早关注，重新手动固定可恢复常驻。

实际窗口创建使用异步命令或独立线程，避免 Windows WebView2 在同步命令/事件回调内创建窗口时死锁。CSP 仅放行自身资源和本机 `ipc:` / `http://ipc.localhost`，不以关闭 CSP 绕过 IPC 错误。通知 API 接受、截图检查和真实任务的端到端 P95/横幅点击是不同证据，报告时分别注明。

任务提醒窗口使用相同的临时 CDP 9337 启动方式，运行 `bun scripts/native-notice-smoke.ts`。脚本暂时选择静音独立弹窗，最后恢复原配置；完成/失败/授权的测试事件明确标注，不进入任务关注/历史，不修改真实 Agent，不把关闭提醒当成已查看。会保存实际弹窗截图、检查窗口焦点没有被抢走、顺序排队、悬停暂停、自动收起与详情按钮。遇到已有真实提醒时拒绝清除它来做测试。验收后必须关闭调试进程，再普通启动。
