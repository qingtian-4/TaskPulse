# TaskPulse 现状审查与改进建议

> **本文档的读者:在本仓库工作的实现方(Codex)。**
> 由 Claude 于 2026-10-10 在 `codex/reliability-ui` 分支审查产出。
> 内容 = 代码审查 + 本机实机验证 + 对既有 Claude Code 接入方案的可行性评估。
> 每条结论都标了证据等级,请按等级对待,不要把推断当事实执行。

## 证据等级约定

| 标记 | 含义 |
|---|---|
| **[实测]** | 在本机实际运行/验证过,可复现 |
| **[已读]** | 读过代码得出的结论,未运行时验证 |
| **[待验证]** | 来源为搜索摘要;本环境 WebFetch 被网络策略拦截,未能取得官方文档原文。**执行前必须自行确认** |

---

## 1. 审查环境

- 平台:Windows 10 Pro 19045,分支 `codex/reliability-ui`
- 运行中的实例:`src-tauri/target/debug/agent-tray.exe --show`(PID 5380,debug 构建,未开 CDP)
- 已执行的验证:`bun run check`、`bun run test:ui-model`、`agent-tray.exe --test-notification`、桌面截屏、进程窗口枚举
- 未能执行:**CDP 端到端原生验收**(需要先停掉运行中的实例并重新构建)

---

## 2. 延时问题

> **重要前提:以下为静态代码分析的结论,没有做 profiling。**
> 动手前请先用 `agent-tray.exe --diagnose` 采样 10 秒,记录 `last_poll_ms` 与 `first_ready_ms`,把优化前后的数字写进提交信息。下面标 **[已读]** 的项都需要这一步确认。

### P0 — Codex 每次追加都全量重读 + 全量重解析 **[已读]**

`src-tauri/src/providers/codex.rs:319-323` 的缓存以 `mtime`/`len` 为键,而 Codex 运行时每秒写多条事件,**活跃会话的缓存等于失效**;每次 poll 都 `read_to_string` + 逐行 JSON 解析整个 rollout(`codex.rs:305-313`)。文件越大越慢。

时序上更糟:worker 正在 poll 时到达的 notify 要排队等,**实测延时可达单次 poll 时长的 2 倍**。

**建议:改成增量 tail 读取(记住 offset,只解析新增行)。** 这是收益最大的单项优化。

### P0 — DeepSeek 每秒读全部 projection shard + 全量 zstd 解压 **[已读]**

`src-tauri/src/providers/deepseek_harness.rs:226-248` 每次 poll 无差别 `read_dir` 并读取解析**每一个** projection shard,无 mtime 缓存;`parse_session` 全量解压 zstd 日志。缓存键混了 log 与 projection 的 mtime(`deepseek_harness.rs:199-202`),**纯改名也会触发全量解压**。

**建议:shard 加 mtime 缓存;缓存键拆开。**

### P1 — WSL 与进程扫描没有文件监听 → 硬性 2 秒 **[已读,已复核]**

`src-tauri/src/watcher.rs:710-749` 的 `watch_paths()` 里,`DeepseekWsl` 与 `Scanner` **一条路径都没注册**,只能靠 worker 固定间隔:`Scanner` 2 秒(`watcher.rs:652`)、WSL 2 秒(`watcher.rs:671-675`)。

WSL 更差:每次 poll 要起多个 `wsl.exe` 进程(`deepseek_harness.rs:207-289`),进程启动本身上百毫秒。

### P1 — 远端固定 3 秒 + SSH 超时 5 秒 **[已读]**

远端 turn 结束不产生本地文件事件,只能等满一个轮询周期(`watcher.rs:962-965`),慢主机再叠加最多 5 秒 SSH 超时。

### P2 — 主循环上有同步磁盘写 **[已读]**

每约 2 秒一次 emit,`src-tauri/src/attention.rs:408-425` 会把**全部关注记录** pretty-print 成 JSON 并 `flush` + `persist`,**就在负责发 UI 事件的线程上**。`monitoring.rs:503-532` 的 `refresh_known_metadata` 还会克隆整个 catalog。

观察通道是 `sync_channel(8)`(`src-tauri/src/providers/worker.rs:24`)——主循环一卡,worker 就在 `send` 上阻塞、停止轮询,延时进一步尖峰。

**建议:落盘挪到独立线程。**

### P2 — notify 触发时会先发一次"旧数据" **[已读]**

`watcher.rs:1165-1176`:`force` 时立刻 `poll()`,但此刻 worker 刚被唤醒、还没产出,于是先拿旧状态完整跑一遍 merge → attention → emit,100ms 后新数据到了**再跑一遍**。功能正确,但白跑一次全量 IPC,还会用中间态污染 attention tracker。

### 一个需要纠正的常见误判

`watcher.rs:1165` 的 `last_scan.elapsed() >= 2s` **不是延时源**。worker 结果每 100ms 就被 `collect_ready()` 排空,那个闸门只是每 2 秒强制空转一次 poll+emit。不要为了"优化延时"去动它。

---

## 3. UI 问题

| # | 问题 | 位置 | 证据 |
|---|---|---|---|
| 1 | **托盘右键菜单是英文**(`Show / Pin / Hide / Test notification / Quit`),整个应用界面是中文 | `src-tauri/src/main.rs:67-78` | [已读] |
| 2 | **托盘 tooltip 恒为 "TaskPulse"**,不显示计数。`update_icon` 只改图标不改 tooltip | `main.rs:95`、`tray.rs:79-100` | [已读] |
| 3 | **面板失焦即自动隐藏**,点"管理监控来源"会把自己的面板关掉,详情区选择丢失 | `src/lib/components/Dashboard.svelte:136` | **[实测]**:`--show` 启动后枚举窗口,句柄 198676 `Visible=False` |
| 4 | **计数条顺序与分组顺序不一致**:计数条是 `运行 / 需处理 / 待查看`,列表分组是 `需要处理 → 正在运行 → 结果待查看` | `Dashboard.svelte:153` vs `Dashboard.svelte:42` + `:174` | [已读] + 截图可见 |
| 5 | **每行都重复 `来源采集 · 0 秒前`**,28 行全一样,恒定信息占一行高度 | `src/lib/components/TaskList.svelte:16` | [已读] + 截图可见 |
| 6 | **每行 4 行文字**(标题/上下文/活动/采集时间),440×720 面板只能显示约 4 条任务 | `TaskList.svelte` | [已读] + 截图可见 |
| 7 | 每收到一次 `agents-updated` 就额外 `invoke('get_archived_attention')`,每 2 秒一次多余 IPC | `Dashboard.svelte:122` | [已读] |
| 8 | 管理页"添加来源"下拉**默认值是 `Codex · 远程服务器`**(最难、需要手写 SSH 别名的选项) | `src/routes/manage/+page.svelte:22` | [已读] + 截图可见 |
| 9 | 管理页 SSH 别名输入框 placeholder 用的是看起来像真人别名的 `Server1_wanghy`,建议换中性示例 | `manage/+page.svelte:147` | [已读] |
| 10 | 详情面板在 `turn_ended` 时把**最后一条对话消息**标为"最近结果",语义误导(内容是 agent 的寒暄) | `Dashboard.svelte:177` | [已读] + 截图可见 |
| 11 | 长标题提醒卡片里,`测试提醒` 徽章挤到关闭按钮 `×` 边上 | `src/routes/notice/+page.svelte:100-102` | [实测,截图] |
| 12 | 三个组件是死代码,零引用:`AgentRow.svelte`、`AggregatePill.svelte`、`ProjectTree.svelte`;`StatusDot` 只被死代码引用 | `src/lib/components/` | [已读,grep 确认] |
| 13 | 面板首次打开是 440×180 的空窗口,等 SvelteKit 加载完再被 `ResizeObserver` 撑开,有一次白屏闪烁 | `tray.rs:157-183` | [已读] |

### 关于 `DetectionConfidence`

`精确 / Hook / 推断` 这个概念在类型系统里已经存在(`src-tauri/src/domain/task.rs`、`src/lib/types.ts:21`、`types.ts:151-155`),**但当前没有任何活跃界面在显示它** —— `CONFIDENCE_LABEL` 只被死代码 `AgentRow.svelte` 引用。

**结论:复用这个类型,不要再发明第二套"支持等级"词汇。**

---

## 4. 上手门槛与发布阻塞

### 4.1 首次启动看不到任何东西,而 README 让你去点一个不存在的齿轮 **[已读]**

`src-tauri/tauri.conf.json:14` 是 `"windows": []`,唯一的自动开窗挂在 `--show` 上(`main.rs:145-147`)。但 `README.md:51` 写的是"首次启动点击右上角齿轮"——**窗口根本没开**。真正的入口是点托盘图标或 `Ctrl+Shift+A`,而快捷键只写在 `docs/PRODUCT_DASHBOARD.md:7`。

### 4.2 全局快捷键被占用会导致整个应用启动失败 **[已读,未实测复现]**

```rust
app.global_shortcut().register(shortcut)?;   // main.rs:142,用 ? 往上抛
```

外层是 `.expect("AgentTray failed to build")`(`main.rs:180`)。`Ctrl+Shift+A` 被别的软件占用时,setup 返回 Err → panic 退出。用户看到的是"应用闪一下就没了",连托盘都不出现。

**建议:失败降级 + 日志警告,不得 panic。**

### 4.3 品牌与身份仍是上游的 **[已读]**

| 项 | 值 | 位置 |
|---|---|---|
| `identifier` | `com.sprklai.agenttray` | `tauri.conf.json:5`、`main.rs:30` |
| macOS 签名身份 | `Developer ID Application: NSR Technologies Inc. (6BQSGY2B74)` | `tauri.conf.json:43`,CI 亦硬编码 |
| 二进制名 | `agent-tray.exe` | `Cargo.toml:2` |
| 启动失败信息 | `"AgentTray failed to build"` | `main.rs:180` |

**这不只是整洁问题。** 应用提供 `system` 通知模式,Windows 上通知的 AppUserModelID 就是这个 identifier —— 用户收到的系统通知会挂在另一个应用名下。

**`identifier` 发布后不可更改(会变成新应用)。现在改的成本是 0。**

### 4.4 打包后 Hook 安装必然失败 **[已读]**

`watcher.rs:429-455` 从 exe 同级目录找 `scripts/`;但 `tauri.conf.json` 的 `bundle` 段**没有 `resources` 键**(已 grep 确认计数为 0),`scripts/` 不会被打包。

### 4.5 其它 **[已读]**

- **没有任何开机自启**:全仓 grep 无 autostart 相关代码或插件。
- **没有安装包**,只能源码构建,README 未说明。
- **开发命令不可用** **[实测]**:`cargo test --manifest-path src-tauri/Cargo.toml`(CLAUDE.md 里写的命令)在本机先报 `windres: program not found`;补上 `dev.sh` 里的 MinGW 环境变量后又报 `另一个程序正在使用此文件 (os error 32)` —— 因为运行中的 `agent-tray.exe` 锁住了 `src-tauri/target`。
- 文档里的个人痕迹:`docs/ATTENTION_LIST.md:78` 等处用 `Server1` / `Server1_wanghy` 作为示例,建议换中性名。

---

## 5. Claude Code 接入:对既有方案的核查

### 5.1 事实核查(全部通过)

| 既有分析的说法 | 结果 | 证据 |
|---|---|---|
| 本机已有会话日志,含 `sessionId`、`cwd`、`aiTitle` | ✅ 属实 | `~/.claude/projects/d--Project-agenttray/<sessionId>.jsonl` 内有 `{"type":"ai-title","sessionId":…,"aiTitle":…}` |
| 目前只有旧版 Hook + 进程扫描,无独立 Claude Code 采集器 | ✅ 属实 | `SourceKind` 仅 8 种,无 `ClaudeCode`;`TaskSource::ClaudeCode` 存在但只被 scanner 打标签 |
| 用户 `~/.claude/settings.json` 未装 Hook | ✅ 属实 | 该文件仅有 `env` / `includeCoAuthoredBy` / `model` |
| 进程扫描不足以可靠获取名称与生命周期 | ✅ 属实,且更糟 | `scanner/strategies/claude_code.rs:68-103` 三条信号全靠 `window_title` + 子进程数 + CPU%。**VS Code 内嵌终端没有独立窗口标题**,只剩 CPU 启发式,置信度 0.3–0.5 |
| `discover_sources` 只检查已登记来源是否存在 | ✅ 属实 | `monitoring.rs:631-677` 即逐个 `home.join(".codex").exists()` |
| Agent 与模型必须分开,模型不参与身份判定 | ✅ 正确,但理由未说透 | 见 5.3 |

### 5.2 需要修正的方向:技术路线押在了未文档化的接口上

既有方案推荐"生命周期事件 + 原生会话日志"组合,重心在 **hooks + 解析 `.jsonl`**。但:

> **`.jsonl`、`~/.claude/sessions/<pid>.json`、`~/.claude/ide/*.lock` 三者都是未文档化的内部实现。** `[待验证]`

- `.jsonl` 记录格式:文档声明为 internal,可能变更;`ai-title` 在某些情况下不产生。
- `~/.claude/sessions/<pid>.json`:有一个专门的 issue 在要求把它稳定化。
- **`~/.claude/sessions/` 目录内还有 `.key` 文件 —— 那是凭据。任何实现都必须只读 `.json`,绝不读取、索引或复制 `.key`。**
- `~/.claude/ide/*.lock`:有已知的 stale-lock 问题。

### 5.3 模型是**可以**拿到的

实测从本会话 transcript 读到的模型:

```
message.model = "deepseek-flash"
```

**明文写在 assistant 记录里。** 所以详情页**可以**如实显示后端模型,不需要像既有方案那样保守。

理由要说透:**身份只能绑 `sessionId`** —— 因为 C Switch 这类代理是透明的,同一会话中途可以切模型。**"切换模型不产生新任务行"必须写成验收标准。**

### 5.4 文档化的正道是 `statusLine` **[待验证]**

Claude Code 有一个文档化、推送式的接口:`statusLine`。每次 assistant 消息和权限模式变更时,把 JSON 从 stdin 喂给指定命令(约 300ms debounce,支持 `refreshInterval`)。输入含:

`session_id`、`session_name`、`model.id` / `model.display_name`、`cwd`、`workspace.*`、`transcript_path`、`cost.*`、`context_window.*`、`rate_limits.*`、`agent.name`

**这与现有架构天然契合:** `scripts/wrap.sh` 和 `providers/local_file.rs` 做的就是"脚本收到信息 → 写出 `~/.agent-monitor/<name>.status`"。statusLine 桥接脚本可以产出同样的 `.status` 格式,**直接复用 `LocalFileProvider`,Rust 侧几乎不用改**。

**注意两个坑:**
1. 用户可能已有 statusLine(ccusage 等),**必须链式调用**,不能覆盖。
2. 上述字段来自搜索摘要,`[待验证]`。动手前先用本机 `claude` 实测:挂一个把 stdin 转存到文件的探针脚本,看真实字段。

### 5.5 Hooks 在 VS Code 扩展里是生效的 **[待验证]**

扩展与 CLI **共享 `~/.claude/settings.json`**,hooks 是引擎特性而非终端特性。因此 `Stop`、`PermissionRequest`、`Notification`(含 `agent_needs_input` / `agent_completed` matcher)可用。

**但要改用户的全局 Claude 配置** —— 所以 hooks 应定位为**精度升级**,不是接入前提。

### 5.6 对既有方案的四处修正

#### 修正 1:不要做"20 个 Agent 的勾选清单"

既有方案的规则表把首次打开定义成"显示已发现的 Agent,让用户一次确认",完整列表放进"添加 Agent"供搜索。**这与用户诉求相反** —— 用户原话是"不可能一个一个手动的去添加"。做成清单只是把"每次加一个"换成"一次看 20 个"。

而且现有代码方向已经是对的:`manage/+page.svelte:114` 首次设置时会**自动预勾选已发现的本机来源**。

**真正的缺口只有一个:** `discover_sources` 只检查**已登记来源**的数据目录,不会发现**一个全新的 Agent 类型**。

**建议改动范围收窄为:让发现逻辑探测"候选数据目录"(如 `~/.claude/` 出现),作为候选来源冒出来。** 这一个改动拿到大部分收益,不需要任何清单 UI。

#### 修正 2:接入优先级重排

| 优先级 | 手段 | 得到什么 | 用户成本 |
|---|---|---|---|
| **1** | `statusLine` 桥接 → 复用 `LocalFileProvider` | 标题、项目、模型、会话 ID、生命周期 | 改一次 settings.json(可撤销) |
| **2** | 读 transcript 的 `ai-title` 兜底 | 补齐标题 | 零 |
| **3** | `Stop` / `PermissionRequest` hook | 精确"本轮结束"/"需要授权" | 改全局 Claude 配置 |
| **4** | `~/.claude/ide/*.lock` | VS Code 会话的**定位/跳转** | 零 |

**第 4 条既有方案完全没提,但它决定体验闭环。** 实测本机 `~/.claude/ide/22676.lock`:

```json
{"pid":21644,"workspaceFolders":["d:\\Project\\agenttray"],"ideName":"Visual Studio Code"}
```

这是 VS Code 内嵌终端会话**唯一**能做"点一下跳回去"的线索。没有它,任务行的"定位 →"按钮对这些会话必然失效 —— 因为 scanner 依赖的 tty / 窗口句柄根本不存在。

#### 修正 3:"自动启用"规则与现有安全语义冲突

既有方案的规则表写"后续启动了新的、已支持的 Agent → 按首次选择的自动接入偏好启用"。

但现有代码有一条**刻意的**性质:`onboarding_complete=false` 时什么都不采集(`monitoring.rs:222-226`),并有 `reset_baselines` / `new_execution` 一整套防止"把历史当新任务"的机制。

**自动启用必须收窄为:只对用户已按"类型"批准过的来源自动启用,且只纳入当前确实在运行的任务,不回溯历史。**

#### 修正 4:复用 `DetectionConfidence`,不要新造词汇

既有方案提议界面展示三档"已准确接入 / 部分接入 / 仅发现"—— 这与 `DetectionConfidence{Exact, Hook, Inferred}` 一一对应。**直接用现有的**,只做一件事:**把本地来源的支持等级也显示出来**(现在完全没显示)。

---

## 6. 建议的落地顺序

### 阶段一:修"第一次用就跑不起来"(小改动、零风险、收益最大)

1. 首次启动自动开一次面板,并让 README 与实际一致
2. `register(shortcut)` 失败降级,不再 panic
3. 托盘菜单中文化 + tooltip 显示实时计数
4. **改 `identifier`**、移除上游 macOS 签名身份、换掉文档里的个人化示例
5. 删除三个死代码组件

> 第 4 条越早越好:identifier 一旦发布就改不动了。

### 阶段二:延时(动手前先 `--diagnose` 采样留存基线)

6. **Codex 增量 tail 读取**(收益最大的单项)
7. DeepSeek projection shard 加 mtime 缓存,缓存键拆开
8. attention 落盘挪到独立线程
9. WSL / Scanner 补文件监听

### 阶段三:Claude Code 接入

10. statusLine 桥接脚本 + 复用 `LocalFileProvider`(先做探针验证字段)
11. 新增 `SourceKind::ClaudeCode`
    - 涉及:`SourceKind` 枚举、`from_legacy`、`validate` 的固定 ID 规则、`source_id`、前端 `SourceKind` 联合类型、manage 页 `kinds` 数组
12. `discover_sources` 增加候选数据目录探测
13. 复用 `DetectionConfidence` 显示本地来源支持等级
14. `~/.claude/ide/*.lock` → focus 能力(注意 stale lock)
15. hooks(`Stop` / `PermissionRequest`)做精确事件

### 阶段四:产品完整度

16. 开机自启
17. 打包 `scripts/` 为 resource,让 hook 安装器在安装包里可用
18. 英文 README + 统一平台定位 + 修正 CONTRIBUTING

---

## 7. 明确不要做的事

- **不要读 `~/.claude/sessions/*.key`。** 那是凭据。
- **不要未经用户同意修改其全局 Claude 配置**(`~/.claude/settings.json` 的 hooks / statusLine)。必须走 UI 显式确认,且可撤销。若用户已有 statusLine,**必须链式调用而非覆盖**。
- **不要为了"优化延时"去动 `watcher.rs:1165` 的 2 秒闸门** —— 它不是瓶颈。
- **不要新造第二套"支持等级"词汇** —— 复用 `DetectionConfidence`。
- **不要依赖未文档化格式做硬断言。** 依赖内部格式时,每个字段都要可选、容忍未知字段、容忍最后一行被截断、处理 stale 文件(**绝不因为文件存在就认为进程存活**)、格式漂移时**降级为"仅发现",不报错也不误报完成**。
- **不要为迁就截图或测试而标记任务已查看、清空用户记录。**

---

## 8. 必须保持的语义红线

现有代码已明确写下并测试了这些性质,**改造中不得破坏**:

- Agent turn 完成 = `WAITING_INPUT`,**不是** `SUCCEEDED`。
- 外部命令只有拿到真实退出码才能标 `SUCCEEDED`/`FAILED`;**不编造退出码**。
- SSH 超时 ≠ 任务完成。
- 不存凭据(走系统 ssh + `~/.ssh/config`);不 OCR;不读 pane 内容;无云端。
- 来源读取失败时保留上次状态并显示降级,**不把读取失败误判为任务结束**。
- `discover_sources` 探测到的会话 ≠ 关注列表;首次接入不批量导入历史。

---

## 9. 附录:可复现证据

### 9.1 已通过的验证

```
bun run check          → 3843 FILES, 0 ERRORS, 0 WARNINGS
bun run test:ui-model  → 9 pass, 0 fail, 36 expect() calls
agent-tray.exe --test-notification
                       → "Notification accepted by the native Windows/desktop API." exit=0
```

### 9.2 窗口枚举(证明弹窗真实存在且位置正确)

运行中实例的窗口(PID 5380):

```
Handle   Visible  Title                W    H    X     Y
198676   False    TaskPulse            456  728  1195  100
133502   False    TaskPulse · 任务提醒   320  132  1584  892
```

- `TaskPulse` 面板 `Visible=False`(因失焦自动隐藏,见 3.3)
- 提醒窗口停在 **(1584, 892)**,精确等于 `src-tauri/src/task_notice.rs:311` 单测断言的 `corner_position(0,0,1920,1040,1.0) == (1584, 892)`

### 9.3 既有真机验收产物

`src-tauri/target/ui-smoke/` 内有 2026-10-10 10:30 的原生验收结果,`passed: true`,13 项检查全过(完成/失败/授权三种窗口渲染、焦点不被抢、顺序排队、悬停暂停、15 秒收起、长标题不溢出、无 CSP 报错)。

其自报的未验证项(诚实标注,保留):
- `fresh real Agent task finish during acceptance`
- `OS system banner delivery`

**即:提醒弹窗本身是好的,不需要重做。上面 3.11 只是排版微调。**

### 9.4 本机 Claude Code 数据面(用于 5 节)

```
~/.claude/sessions/19860.json
{"pid":19860,"sessionId":"129b29c4-…","cwd":"d:\\Project\\agenttray",
 "status":"busy","name":"agenttray-14","entrypoint":"claude-vscode",
 "version":"2.1.295","peerProtocol":1,"startedAt":…,"updatedAt":…}

~/.claude/ide/22676.lock
{"pid":21644,"workspaceFolders":["d:\\Project\\agenttray"],"ideName":"Visual Studio Code"}

~/.claude/projects/d--Project-agenttray/<sessionId>.jsonl
{"type":"ai-title","sessionId":"…","aiTitle":"当前使用的模型"}
message.model = "deepseek-flash"
```

`~/.claude/sessions/` 中只有**存活**会话的条目(实测只有 1 个文件,且对应 PID 正在运行)。
**`sessions/` 目录同时含 `.key` 凭据文件 —— 见第 7 节。**
