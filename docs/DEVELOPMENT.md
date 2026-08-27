# Development

TaskPulse = AgentTray fork（MIT，保留 attribution）。Windows 优先，Linux/macOS 尽量不破坏。

## 环境（本机已验证）

- Windows 10 + git-bash
- Rust：rustup `stable-x86_64-pc-windows-gnu`（**本机无 MSVC**，用 GNU target）
- C 链接器：**zig cc**（`C:\tools\zig-windows-x86_64-0.14.0`），wrapper 在 `C:\tools\zig-cc.cmd`
- bun（`npm i -g --allow-scripts=bun bun`）
- WebView2 runtime（系统已装，Tauri popup 依赖）

```bash
export PATH="$HOME/.cargo/bin:/c/tools/zig-windows-x86_64-0.14.0:$PATH"
export RUSTFLAGS="-C linker=zig-cc"
```

> 如果换用 MSVC 工具链：装 VS Build Tools（C++ workload）后去掉 RUSTFLAGS，`rustup default stable-x86_64-pc-windows-msvc`。

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
- 日志：`RUST_LOG=debug` 前缀运行可看细节（env_logger）

## Git 工作流

- 分支 `feat/taskpulse-mvp`，每个 Phase 一个逻辑 commit；不自动 push / 不开 PR。
- LICENSE（MIT）与上游 attribution 保留。

## 已知限制（MVP）

- `tauri dev` 的交互验证（托盘出现、popup、通知）需要真实桌面会话——headless 环境只能验证到 `cargo test` + `bun run build` 层面。
- tmux 3.0a 无新版 `-F` 特性；pane_title 含 tab 的行会被解析器跳过（字段数校验）。
- SSH 输出超过管道缓冲（~64KB）的极端 pane 集合可能阻塞读取——当前规模无影响。
- 孤儿 hook 文件清理（session-id 交叉校验）暂由 10 分钟 STALE_TTL 覆盖。
- Codex/Hermes/DeepSeek 的端到端验证取决于本机安装情况；adapter 边界、fixture 测试与文档已完成，真实限制在 README/LOCAL_AGENTS.md 记录。
