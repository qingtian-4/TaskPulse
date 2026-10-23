# Repository hygiene audit — 2026-10-23

起点：`5e9c083f1e6816c7680030c374cd34294ca1237b`，分支 `codex/desktop-discovery-visibility`。先读取 status、完整 ls-files、已跟踪且被忽略文件、排序清单与 count-objects；随后快进复用 PR #10 的 `eed1655008aa4f81c0bb7ffe7a5a1f1a0dc37bd9`，没有重新实现 README。

PR 后 305 个跟踪文件，共 8,702,000 bytes；最大的文件是上游 GIF（5,072,975 bytes）。Git loose objects 8.82 MiB，pack 6.15 MiB；不是工作树大小，不能相加宣称清理收益。起点与 PR 后 `git ls-files -ci --exclude-standard` 均为空。

## 分类与实际操作

| 分类 | 范围 | 理由 / 操作 |
|---|---|---|
| KEEP | `src/`、`src-tauri/src/`、Cargo/Bun 锁文件、配置、构建脚本 | 完整产品与可复现依赖；无模块删除 |
| KEEP | `integrations/`、`scripts/*.test.*`、原生 smoke、Python 采集脚本 | 协议、fixture、可靠性与发布检查；不能按名字判断为临时文件 |
| KEEP | `.github/`、许可证、贡献与安全政策、`.claude/commands/release.md` | CI、治理、明确授权的发布流程 |
| KEEP | 原生 PNG、Tauri 全平台图标、两个 SVG | PNG 有来源记录；图标属于构建资源；旧 SVG 在素材索引中明确标为历史 |
| KEEP | `assets/legacy-agenttray/*` | 旧 GIF 约 5 MB；已独立存放并保留版权来源，不用于当前产品展示。删除工作树文件不能缩小历史 |
| KEEP | 日期验收、candidate/release 文档、`docs/proposals/` | 保留生命周期、通知、WSL、安装器和 UI 关键证据；测试/发布文档仍引用 |
| KEEP | `docs/internal/build-spec-v2.md` | 已在历史区，保留原始规格语境 |
| ARCHIVE | 下表 8 个路径 | 全文扫描所有 Git 跟踪文本，记录引用后 `git mv`；更新相对链接、代码路径及当前脚本导航，添加历史提示 |
| REMOVE FROM TRACKING | 无 | 未找到可再生成且无依赖的已跟踪运行输出；没有执行 `git rm --cached` 或删除本地文件 |

| 原路径（均在 `docs/` 下） | 新路径（均在 `docs/` 下） | 原引用位置 |
|---|---|---|
| `ARCHITECTURE_AUDIT.md` | `internal/reports/ARCHITECTURE_AUDIT.md` | ARCHITECTURE |
| `PROJECT_REVIEW_2026-10.md` | `internal/reports/PROJECT_REVIEW_2026-10.md` | OPEN_SOURCE_CHECKLIST、RELEASE_READINESS |
| `REVIEW_RESPONSE.md` | `internal/reports/REVIEW_RESPONSE.md` | CLAUDE.md |
| `SECURITY_AUDIT_2026-10.md` | `internal/reports/SECURITY_AUDIT_2026-10.md` | 两份 README、CONTRIBUTING、candidate、release、UI/visual 文档、项目审查 |
| `RELIABILITY_OPTIMIZATION_2026-10.md` | `internal/reports/RELIABILITY_OPTIMIZATION_2026-10.md` | AGENT_COMPATIBILITY |
| `compatibility.md` | `internal/upstream/compatibility.md` | internal/build-spec-v2 |
| `use-cases.md` | `internal/upstream/use-cases.md` | 无 |
| `maintainers/releasing.md` | `internal/upstream/releasing.md` | scripts/README（当前导航改为 docs/RELEASING.md） |

未触碰起点用户修改：`src-tauri/Cargo.toml` 的 `[profile.dev]`，以及未跟踪的 `AGENT_INTEGRATION_EXECUTION_PLAN.md`、`AGENT_INTEGRATION_REVIEW_2026-10-21.md`、`REVIEW_HANDOFF.md`。它们不属于本轮提交。

## 隐私与历史边界

归档不会移除 Git 历史中的内容。旧审查中的示例机器、路径与历史结果不是当前验收；分享诊断前仍须脱敏。未做历史重写、GC、clean、缓存删除、Tag/Release 变更。若未来要求减小 clone 历史，应单独设计 LFS/新历史迁移，列出版权与下载兼容性影响并获授权。

新增 `test:repo` 将检查所有跟踪 Markdown 本地链接/锚点、SVG 本地素材引用、跟踪忽略冲突和大型视频；它不验证外部网页或私人状态目录。
