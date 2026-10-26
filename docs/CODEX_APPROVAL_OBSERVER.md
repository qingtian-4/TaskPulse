# Codex 审批事件接入

普通 Codex rollout 不一定记录审批请求。2026-10-26 对当前 VS Code / Ubuntu 会话的只读核对未发现 `requestApproval`；不能根据命令参数、工具耗时或静默时间模拟审批。

当前安装的 Codex `0.164.0-alpha.11.1` 将 Hooks 列为 stable。官方 `PermissionRequest` 在即将询问批准时触发；没有决策输出时，原审批流程继续。参见 [官方 Hooks 文档](https://learn.chatgpt.com/docs/hooks)。

`scripts/codex-observer.py` 仅观察 `PreToolUse`、`PermissionRequest`、`PostToolUse`、`Stop`、`Interrupt`、`SessionEnd`。它不输出批准/拒绝、上下文或提示词，不修改审批策略。只保存会话/轮次 ID、关联指纹、未决请求及序号。没有唯一关联的请求不会凭任意工具结果解除。无法读取事件时保留待确认状态。

安装必须由用户明确确认。在需要监控的 Codex 所在环境运行安装器，将脚本复制到 `~/.taskpulse/codex-observer.py`，向 `$CODEX_HOME/hooks.json`（默认 `~/.codex/hooks.json`）合并六种事件配置。保留其他 Hook，修改前备份原文件；移除时只删除这些精确命令。

```sh
python3 scripts/install-codex-observer.py
# 撤销接入，保留其他 Hook 和备份：
python3 scripts/install-codex-observer.py --remove
```

脚本使用 `CODEX_HOME`（默认 `~/.codex`）下的 `taskpulse-events`；Windows 和 WSL 原生来源分别读取各自的事件文件，不互相混用。安装后还需在 Codex 的 `/hooks` 中审阅并信任具体定义，重新打开会话以加载配置；不得绕过 Hook 信任检查。正在显示、发生在安装前的审批不能追溯补造事件。

验证包括独立临时目录中的并行请求关联、未匹配结果、轮次切换、损坏数据及敏感内容不落盘。真实验收仍需在原 Agent 中产生一次新的审批，确认 TaskPulse 弹窗、原界面批准/拒绝及清除流程。
