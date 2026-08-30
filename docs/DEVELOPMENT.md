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

- `providers/remote_tmux/`：tmux/ps 解析、进程树检测、SSH 失败语义、DISCONNECTED 阈值、恢复、并行轮询（FakeSshExecutor 脚本化响应，无需 Server1/Server3）。
- `store/`：EXACT>HOOK>INFERRED 合并、同 pane 去重、RECENT TTL、新运行重置。
- `transition/`：RUNNING→WAITING_INPUT / SUCCEEDED / FAILED / 推断完成；只通知一次；断连/恢复。
- `domain/`：状态优先级、置信度、序列化。
- `heuristics/`：hook 状态 × 扫描数据交叉验证（旧 + 新模型两套测试）。

## 配置

- 远端：`~/.taskpulse/config.toml`（首启自动生成 Server1/Server3 示例）
- 本地状态目录：`~/.agent-monitor/`（兼容）+ `~/.taskpulse/`
- 最近完成缓存：`~/.taskpulse/recent.json`（版本化 JSON，仅保存 RECENT，原子替换）
- 日志：`RUST_LOG=debug` 前缀运行可看细节（env_logger）

## Git 工作流

- 分支 `feat/taskpulse-mvp`，每个 Phase 一个逻辑 commit；不自动 push / 不开 PR。
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
