# 安装与快速入门

当前版本为 **0.2.0-alpha.7，尚无公开可下载的 Alpha.7 安装包**。请先使用
[源码运行](../README.md#快速开始)。Windows Quality / Security CI 已通过不等于安装包已发布或真实宿主全面验收。

Alpha.7 源码已合入默认 `main`，请按 README 的 `--branch main` 命令克隆；
已有克隆先用 `git status` 检查本地修改，不要为升级覆盖工作区。用 `git rev-parse HEAD` 核对实际提交，
旧提交的 CI 不覆盖新的文档修复或最终 main SHA，详见[候选状态](ALPHA7_CANDIDATE_STATUS.md)。
源码合并、CI 通过、Draft pre-release 与公开可下载是四个不同状态；当前仍请从源码运行。

未来在 [TaskPulse Releases](https://github.com/qingtian-4/TaskPulse/releases) 发布后，
Windows 10/11 x64 可选择 `TaskPulse_<版本>_windows_x64_setup.exe`。
`Source code.zip` 是源码，不是安装器；草稿对普通用户不可见，旧 AgentTray 包不是 TaskPulse。

MSI 是可选安装格式，不要与 NSIS 同时安装。ZIP/Portable 尚未交付。
安装版运行不需要 Rust/Bun/Node；需要 WebView2，缺少时安装器会联网下载运行时。
离线环境应先安装 WebView2；不提供绕过系统安全提示的自动操作。

## 三步开始

1. 从源码启动 TaskPulse，打开来源管理，发现或选择 Agent，先查看支持能力。
2. 配置来源，选择“所有项目的新执行”或设置项目白名单，再保存。WSL/SSH 独立添加，不会
   默认访问停止的发行版或所有服务器，也不自动修改 Agent 配置。
3. 测试读取和提醒，然后在启用范围内执行真实任务。托盘或 `Ctrl+Shift+A` 打开面板。

发现 Agent 只是安装/数据存在的证据；保存只代表配置生效；读取正常不代表已经收到真实结束事件。
测试提醒只验证通知链路，不验收 Agent。新执行的标题、运行阶段、真实结束和请求解除仍需在原 Agent 中核对。

来源页顶部提供“测试读取来源”和“发送测试提醒”。前者等待所选来源本次返回，不使用旧成功替代；
后者仅投递测试提醒。通用来源的“测试事件接收”也不会创建任务。Adapter ID、Hook 预览/撤销、
Manifest 与诊断在高级区域；真实 Hook 写入仍需明确确认。界面操作与筛选说明见 [使用指南](PRODUCT_DASHBOARD.md#alpha7-界面操作)。

思考、工具运行、等待模型响应都属于进行中；任务名称与项目名称分别展示。
本轮结束不等于项目完成。需要授权时去原 Agent 处理，TaskPulse 不替你批准。

关闭弹窗不等于已查看。右上角铃铛是消息中心，角标只计未读消息；实际显示的消息
会标为已读，也可选择“全部标为已读”。阅读消息不查看任务结果，也不批准或解除请求。
仍未解决的请求继续保留在“待处理”和消息中心的请求列表中；来源过期时显示上次记录，
只能核对／刷新，不重新弹出尚未确认的请求。完成和失败在“待查看”中回查，重启不重发旧提醒。

## 找不到任务时

- 先清除界面筛选；检查来源是否启用、项目是否在范围内，尤其是现有白名单。
- Windows、WSL 与 SSH 是独立来源。WSL Codex 需要运行中的发行版和 Python 3。
- SSH 先确认自己的别名可非交互连接；凭据交给系统 SSH，不填写进 Issue。
- “刷新”等待来源真实返回；来源异常／状态待更新不能当作任务结束。
- 原生格式和事件能力有版本差异，见 [支持矩阵](AGENT_COMPATIBILITY.md)。

## 进阶配置

### Claude Code / CC Switch

默认只读 `CLAUDE_CONFIG_DIR` 或 `~/.claude`。切换代理模型后 Agent 身份仍为 Claude Code，
上报模型不保证代表代理背后的实际模型。需要显式事件时，在来源的高级接入设置中预览并确认
TaskPulse Hook；保留其他 Hook、statusLine、代理和模型设置。不要只凭测试提醒宣布 Hook 已实测。

### 远程代码运行

添加自己的 SSH 别名与 tmux 来源，项目范围包含代码目录。在 tmux 中直接运行可以观察进程，
`tp-run` 则记录被监督的前台命令与真实退出码（需要 Python 3）：

```bash
# 在服务器的仓库目录中手动安装 wrapper
mkdir -p ~/.local/bin
install -m 755 scripts/tp-run ~/.local/bin/tp-run
~/.local/bin/tp-run "训练实验" -- python train.py
```

tmux 外的 wrapper 记录仍由 tmux 来源读取，需要将 session/pane 限定留空。前台命令退出不等于
后台、容器或 Slurm 作业完成。完整边界见 [使用指南](PRODUCT_DASHBOARD.md) 与 [架构](ARCHITECTURE.md)。

### 数据与隐私

配置、关注/未读/固定、最近结果和通知去重分别保存在 `~/.taskpulse/` 的 config.toml、
attention.json、recent.json、notifications.json。投递记录 notification-log.json 最多500条，
不证明用户已读。兼容旧 `~/.agent-monitor/*.status`；`tp-run` 使用服务器的
`~/.local/state/taskpulse/runs/`。本机账号额度不是远程账号额度。

默认 `--diagnose` 导出哈希身份、状态和耗时，不导出标题、路径、命令、错误原文或账号额度；
诊断进程关闭其他日志输出，完整错误仍可在本机来源管理中核对。哈希可关联同一任务，仍应在分享前检查。
新 Claude Hook v3 日志只保存工具活动类别和 config root hash；cwd、session/turn/call/request
仍作为本地范围与审批关联元数据保存。旧日志不自动重写，原生任务、关注状态和其他专用诊断可能
包含私人路径或标题，不要把整个状态目录上传到 Issue。v1/v2 Hook 信封继续可读；读取 v3 root 隔离应使用新版本。

## 升级与卸载

备份 `~/.taskpulse/`，从托盘退出后升级。切换 EXE/MSI 安装格式前卸载旧格式。
卸载不删除 TaskPulse 用户状态；可自行保留备份。若曾明确安装 Claude Hook，
先在来源管理中撤销接入，再卸载，避免 Agent 调用不存在的程序。
