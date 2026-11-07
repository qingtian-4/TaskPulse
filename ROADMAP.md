# TaskPulse roadmap / 路线图

优先保证“明确请求 → 提醒 → 返回宿主 → 匹配解除”的可靠性，再扩展来源和平台。以下为优先方向，不承诺交付日期。

Prioritize reliable requests, reminders, host return and matching resolutions before expanding agents and platforms. These are priorities, not delivery dates.

| 优先事项 / Priority | 目标 / Outcome |
| --- | --- |
| 原生来源 / Native sources | 扩大 Kimi、Qoder 的版本实测；适配 TRAE 可公开读取的数据；保持 Codex、Claude Code、DSH 的明确请求语义。Broaden verified formats without guessing lifecycle events. |
| 远程来源 / Remote sources | 完善 Codex SSH、WSL 的连接恢复和项目范围；具备可靠只读接口后再支持 Claude SSH。Reliable recovery and scopes before adding remote readers. |
| 返回任务 / Return | 经过身份验证的宿主返回，无可靠接口时保留详情与复制。Verified host return with honest fallbacks. |
| 平台交付 / Delivery | Windows ARM64、签名、macOS 公证、安装升级、高 DPI、多屏和睡眠恢复。Signing and runtime coverage across supported architectures. |
| 资源与可靠性 / Reliability | 长时间运行、大数据下保留读取预算、未读、固定和未解决请求。Bounded collection and durable attention under sustained load. |
| 后续探索 / Later | 桌面闭环稳定后评估移动端、手表与手环提醒。Explore mobile and wearables after the desktop workflow is dependable. |

## 质量目标 / Quality targets

- 固定回放零误报、零漏报，重启完整恢复。Fixed replay: zero false positives/negatives and full restart recovery.
- 真实样本按 Agent、版本、平台计量，每组至少 100 个正例与 100 个对照；误报、漏报各 ≤1%。Truth is independently confirmed in the host. Unsupported capabilities are excluded rather than scored as zero misses.
- 从事件可观测到界面实际渲染：事件路径 P95 ≤2 秒，轮询路径 ≤两次轮询间隔 +2 秒。Measure actual rendering, not parser completion.
- 宿主返回成功率、包含 WebView 子进程的资源开销、MSI UAC 与平台实机检查单独记录。Replay and short smoke runs do not establish these results.

[当前能力与平台范围](docs/AGENT_COMPATIBILITY.md) · [贡献指南](CONTRIBUTING.md)
