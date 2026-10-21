# TaskPulse

**少切窗口，少丢专注。多个 Coding Agent，任务状态一眼可见。**

[English](README.en.md) · [快速开始](docs/INSTALL.md) · [支持范围](docs/AGENT_COMPATIBILITY.md) · [贡献](CONTRIBUTING.md) · [反馈](https://github.com/qingtian-4/TaskPulse/issues)

TaskPulse 是 Windows 优先的桌面托盘应用，不是 VS Code 插件。它把不同 Agent、项目和运行位置的任务放进动态关注列表：谁还在运行、谁需要你回答、哪些结果还没看。任务本轮结束或明确需要输入时，独立弹窗提醒你，不用一直把面板置顶。

> **当前版本：0.2.0-alpha.7。尚无公开可下载的 Alpha.7 安装包，请从源码运行。** Windows Quality / Security CI 与安装包公开发布是不同环节；通过 CI 不等于完成干净系统、安装器或长期真实任务验收。

## 看看它如何工作

当前产品使用紧凑的深色任务面板、独立来源管理窗口和窄条通知。原生 WebView2 截图及输入说明见 [展示素材](assets/README.md)；不使用上游 AgentTray 的旧界面充当 TaskPulse 截图。

![TaskPulse Alpha.7 原生关注面板](assets/screenshots/dashboard-alpha7.png)

<details>
<summary>查看来源管理与独立提醒</summary>

![TaskPulse Alpha.7 原生来源管理](assets/screenshots/sources-alpha7.png)

![TaskPulse Alpha.7 原生测试提醒](assets/screenshots/notice-alpha7.png)

</details>

截图为 Windows 原生 Tauri / WebView2 实际渲染，输入为隔离的公开**合成标准事件**，通知图为明确标记的测试提醒。不是浏览器 mock、真实 Agent 对话或四款宿主实机支持证明。当前 UI 为中文。[复现与验证记录](docs/VISUAL_POLISH_VALIDATION_2026-10-21.md)

## 把注意力留给工作

- **同时看多个 Agent**：标题、项目、Agent 和执行活动分开展示，不把目录名当任务名。思考和等待模型响应仍属于运行。
- **只关注值得看的任务**：新执行自动加入；本轮结束、失败和未解决请求保留到你查看。闲置历史不挤占面板，重要会话可以固定。
- **离开面板也能收到提醒**：区分本轮结束、失败、明确审批和提问。关闭提醒不等于已查看，更不会自动批准命令。
- **环境分开管理**：选择本机、WSL、SSH 来源及项目范围；旧项目重新执行后可再次出现，不必逐次添加任务。
- **不确定就明确说出来**：失联、坏行和追赶显示待更新，不根据低 CPU、静默或工具返回猜测完成；没有步骤数据就不编造百分比。

适合一个 Agent 评审代码、另一个修测试、服务器 tmux 同时跑实验的工作流。TaskPulse 帮你发现何时需要切回去，不替代原 Agent 的对话或审批界面。

## 安装与快速开始

目前请使用源码；[Releases](https://github.com/qingtian-4/TaskPulse/releases) 没有公开的 Alpha.7 安装包。`Source code.zip` 不是安装器，旧 AgentTray 安装包也不是 TaskPulse。未来 Windows x64 安装器的验收与命名规则见 [发布说明](docs/RELEASING.md)。

准备 Rust 1.89+、Bun 和 [Tauri 平台依赖](https://v2.tauri.app/start/prerequisites/)；Windows 标准开发环境包括 Microsoft C++ Build Tools 和 WebView2。

Alpha.7 源码已合入默认 `main`，以下命令从 `main` 运行。源码合并不代表安装包已经发布；提交 SHA、CI 和尚未完成的验收见[候选状态](docs/ALPHA7_CANDIDATE_STATUS.md)。

```powershell
git clone --branch main --single-branch https://github.com/qingtian-4/TaskPulse.git
cd TaskPulse
bun install --frozen-lockfile
bun run tauri dev
```

1. 在来源管理中发现或选择 Agent，查看它能提供哪些状态。
2. 配置来源与项目范围，点击“保存并生效”。新来源默认关注所有项目的新执行，已有白名单不会被扩大。
3. 测试读取和提醒，然后在原 Agent 中开始任务。**测试提醒成功不代表真实生命周期已经验收。**
4. 点击托盘或按 `Ctrl+Shift+A` 查看面板；读完结果后标记已查看。回答或审批请返回原 Agent。

配置、Claude Code / CC Switch、服务器 `tp-run`、隐私与排障见 [安装指南](docs/INSTALL.md)、[使用指南](docs/PRODUCT_DASHBOARD.md) 和 [开发环境](docs/DEVELOPMENT.md)。WSL / SSH 单独启用，不会自动唤醒停止的发行版或连接所有服务器。

## Agent 支持矩阵

“已实现”指仓库能力，不保证覆盖每个版本。安装发现、读取成功、通知测试与真实任务验收分别记录。WSL 指 Windows TaskPulse 观察 Linux 内的数据。

| Agent / 来源 | 本机 | WSL | SSH | 重要边界 |
|---|---|---|---|---|
| Codex | 原生适配 | 独立来源 | 原生会话采集 | 只有显式审批事件才提醒授权；大日志首次追赶可能较慢 |
| Claude Code | 原生记录；可选显式 Hook | 无专用适配 | 无专用适配 | 代理模型不改变 Agent 身份；Hook 须确认安装 |
| Hermes | SQLite / 运行租约 | 无专用适配 | 无专用适配 | 租约消失不是完成，缺少结束证据时保留不确定状态 |
| DeepSeek Harness | Desktop / CLI | 独立来源 | 无专用适配 | 兼容旧版 / v4 压缩日志与原生标题投影 |
| Gemini CLI | 兼容 Hook / 进程扫描 | 无专用适配 | 无专用适配 | 不是完整原生会话适配 |
| Kimi Code CLI / TRAE 国内 IDE / ZCode | **部分映射，fixture-tested** | 通用传输未实现 | 通用传输未实现 | 无宿主 Hook 实机验收；Stop 不代表最终结束 |
| WorkBuddy Desktop | **候选入口，无事件适配** | 未实现 | 未实现 | 不把 CodeBuddy CLI 当作桌面支持 |
| 自定义 / 社区 Agent | 本机 v1 事件 / 声明式 Manifest | 未实现 | 未实现 | self-reported 证据，不等同官方原生支持 |
| 普通代码运行 | 通用工作流待完善 | 无专用 Provider | tmux / `tp-run` | 前台退出码不等于后台、容器或 Slurm 作业已完成 |

逐项能力、真实版本与未支持项见 [兼容矩阵](docs/AGENT_COMPATIBILITY.md)。检测到进程不意味着能读取它的对话、标题或审批状态。

## 架构与扩展

```text
Provider → 范围过滤 / 标准化 / 去重 → TaskStore
         → AttentionTracker → TransitionEngine → 托盘 / UI / 提醒
```

Rust / Tauri 2 负责采集与状态，Svelte 5 负责呈现。稳定 session / turn 身份与标题、执行阶段、投递分开：改名不新建任务，工具完成不等于一轮结束。

没有自建云端账户或遥测服务。配置与关注记录在 `~/.taskpulse/`；诊断和活动可能包含路径或命令，分享前应检查。不自动安装真实 Hook、改代理/模型或放宽审批策略。本地 Codex 额度仅代表本机账号，不是任务进度或远程额度。

扩展优先使用明确生命周期事件，其次是可验证的只读记录。参阅 [架构](docs/ARCHITECTURE.md)、[接入指南](docs/AGENT_INTEGRATION_GUIDE.md) 和 [v1 协议](docs/AGENT_INTEGRATION_PROTOCOL.md)。没有通用第三方可执行插件加载器。

## Roadmap

- Alpha.7：继续收集真实版本的状态、提醒与长期运行证据，完善 Windows 安装器验收。
- 接入体验：减少首次配置成本，逐款验收 Kimi / TRAE / ZCode / WorkBuddy，不将候选描述等同支持。
- 跨环境：补齐 Claude / Hermes 传输及本机、WSL、SSH 统一代码运行入口。
- 可选信息：让账号额度查询与展示可以单独控制。

这些是后续方向，不是本版承诺。有界日志、检查点恢复与缓存优化见 [可靠性记录](docs/RELIABILITY_OPTIMIZATION_2026-10.md)，发布门槛见 [交付清单](docs/OPEN_SOURCE_CHECKLIST.md)。

## Contributing / Credits

欢迎提交缺陷、适配器和文档改进。请先读 [CONTRIBUTING.md](CONTRIBUTING.md)；注明系统、Agent 版本与来源类型，删除私有会话或凭据。开发与原生验收见 [AGENTS.md](AGENTS.md)。

派生自 [sprklai/agenttray](https://github.com/sprklai/agenttray)，保留上游作者、版权与 Git 历史，采用 [MIT License](LICENSE)。感谢上游奠定托盘、终端聚焦与 Hook 桥接基础。
