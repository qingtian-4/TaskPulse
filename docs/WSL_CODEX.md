# 本机 WSL Codex

Windows 与 WSL 的 Codex 是独立来源，可以同时监控，不会混用项目路径或账号额度。

## 启用

1. 启动你实际使用的 WSL 发行版，确认其中有 Python 3 和 Codex 会话。
2. 打开 TaskPulse 来源管理，在发现列表添加 **Codex · WSL**，或手动填写发行版名称。
3. 选择项目范围并保存。Linux 项目使用 Linux 路径，例如 `/home/user/project`。

TaskPulse 不会自动启动停止的发行版。没有 Codex 数据的 Docker 内部环境不会作为推荐来源。

## 数据与状态

TaskPulse 在发行版内只读会话元数据，再增量读取日志。不会从 Windows 直接打开 WSL SQLite/WAL，不安装软件或修改 Agent 配置。

标题、项目、模型与执行阶段来自该发行版中的 Codex。思考、工具执行和等待模型响应都属于运行中；只有明确结束事件才结束任务。

WSL 停止、网络路径不可读或采集失败时，保留最后状态并显示待更新。恢复连接后继续跟踪，不把断开当作完成。

## 审批提醒

需要命令审批提醒时，在对应发行版显式安装并信任官方观察 Hook。它不代你批准，也不修改审批策略。详见 [Codex 审批接入](CODEX_APPROVAL_OBSERVER.md)。
