# Open-source readiness — 2026-10-23

## 范围与结论

复用并审查 [PR #10](https://github.com/qingtian-4/TaskPulse/pull/10)，从真实 HEAD `5e9c083` 快进到展示提交 `eed1655` 后，在现有 codex 分支完成四阶段工作。没有新增产品功能或重写状态机；源码变动仅修正文档注释。原有 Cargo.toml dev profile 和三份未跟踪用户文档保留且不提交。

**建议继续开放源码评审和有明确限制的源码试用，暂不进入可公开下载的 Alpha 安装器发布。** [审查](CODEBASE_REVIEW.md) 未确认 P0，但仍有需处理/接受的 P1，尤其损坏状态保全、local worker 配置 generation、Claude activity 隐私/磁盘增长、SSH 输出预算与通知持久化到投递的崩溃窗口。干净用户与真实 Agent 长时验收仍未闭合。本轮未 push、创建/更新 PR、合并 main、打 Tag、修改 Draft Release 或公开发布。

## 文件与文档结果

- [文件分类与全部 8 项移动](REPOSITORY_HYGIENE.md)：全部使用 git mv；0 删除、0 取消跟踪。完整源码、兼容层、恢复机制、测试、原生证据及上游版权素材保留。
- [架构](../ARCHITECTURE.md)：实际入口/窗口、注册表、每来源 worker、两条数据路径、session/turn/request、Store/Attention、通知去重、Journal/Checkpoint/Outbox、WSL/SSH、持久化预算及安全边界。
- 中英文 README 延续 PR #10 的产品优先结构；补精确版本、main clone、fixture-tested、普通代码运行限制、Linux/macOS experimental 与本地数据隐私。默认分支由 GitHub repository metadata 核实为 main；[公开 Releases](https://github.com/qingtian-4/TaskPulse/releases) 查询仍未见公开发布，未将 Draft 当公开安装器。
- SVG 不再暗示所有采集均独立 worker 或所有来源均有 durable outbox；注明 ack after durable state flush，不证明可见投递。使用独立 Edge headless profile 实际渲染并目视检查，XML 解析通过；预览保留在忽略目录，不当作产品截图。
- 逐张检查三张 PNG，没有私人路径、服务器、对话；保持原文件不重拍。录屏工具/交互桌面能力未确认，没有 MP4/GIF 成品。保留 [storyboard、原生复现与附件方案](../DEMO_RECORDING_PLAN.md)，不把浏览器 mock 或连续 PNG 拼图当录屏。

## 已执行验证

| 检查 | 结果 / 证据边界 |
|---|---|
| Bun check | 0 errors / 0 warnings；最初 sandbox 的 Vite spawn EPERM 后在授权执行环境成功 |
| Bun production build | 成功，Svelte static 输出；构建工具性能提示不是产品错误 |
| Bun docs | PR #10 最初 4 fail；修复链接、版本、证据文案、clone 契约后 8 pass |
| Bun 全文件集 | 10 个文件：76 pass / 1 skip / 0 fail；该 skip（真实 CLI 失败证据/清理）指定 TASKPULSE_SMOKE_EXE 后单独补跑 1 pass，共 77 项均执行通过 |
| 全仓链接/素材/跟踪 | 新增 test:repo 4 pass，覆盖所有跟踪 Markdown/SVG；0 ignored-tracked、0 大型视频；链接检查含锚点，不检查外部 URL 可用性 |
| Rust fmt / Clippy `-D warnings` | 隔离 codex-check 目标通过 |
| Rust 全量 target | 423 passed、0 failed、15 ignored（438 definitions）；包含 Codex / WSL / SSH fixtures、协议/reducer/journal/outbox、Attention/通知重启与关联 |
| custom-protocol build | 隔离 target 成功；没有替换正在运行的托盘 EXE |
| 实际 Integration Bridge | 完整 verify 最后一步通过：redirected stdin、validator、silent Hook、identity/size/private payload rejection、单独 test channel、Unicode/space 路径、真实 OS lock timeout；debug EXE，不冒称 GUI-subsystem release 验收 |
| Python WSL discovery | 5 pass，离线 fixture |
| Python remote Codex metadata | 初跑 1 failure/3 cleanup errors；补 source 兼容断言、显式关闭 fixture SQLite、增加 native origin 回归后 6 pass；未改生产采集器 |
| Python WSL metadata | 6 pass，离线 fixture（锁、坏库、旧 index、可信 stat、有限重试） |
| native-product smoke | 通过；7 项产品检查，Dashboard/Manage/Notice 各 DPR 1/1.5/2，共 9 组布局；27 项长列表、搜索/筛选、viewed、来源保存/读取、test notice |
| native-integration smoke `--notices` | 通过；实际 sandbox config preview/conflict/confirmation、Stop safety、独立 approval、Unicode/space path shell forwarding、community manifest、短 turn/去重、重启 viewed/pending、真实 notice timer/click/sticky 回归 |
| `bun audit` 完整报告 | 1 low：cookie 0.6.0 / GHSA-pxg6-pf52-xh8x；moderate+ 门禁通过，不能称无漏洞 |
| `audit:rust` | vulnerability/yanked 门禁通过、registry complete；10 unmaintained + 2 unsound，见下表 |
| `audit:secrets` | checksum-pinned gitleaks 全 Git 可达历史，0 findings；不代表私人工作目录没有敏感内容 |

最终全套 `verify.ps1 -Frontend` 返回 0，包含缓存脚本、moderate+ frontend audit、check、全部指定 Bun suites、release:check、3 组 Python fixtures、build、fmt、Clippy、423 个 Rust tests、custom-protocol build 和实际 Bridge。早期 sandbox 失败与补文档前的暂时断链不计为通过。长日志仅留在 `src-tauri/target/repo-readiness/verify-final.log`、bun-final.log、cleanup-cli-final.log。

本地原生证据（不提交运行输出）：

- `src-tauri/target/ui-smoke/evidence/product-native-2026-10-23T09-37-48-659Z-r4wLI9/result.json` 与 cleanup.json / DPR PNG。
- `src-tauri/target/ui-smoke/evidence/integration-native-2026-10-23T09-38-01-612Z-CdI0xy/result.json` 与 cleanup.json。
- `src-tauri/target/ui-smoke/native-notice-result.json`：由隔离 integration smoke 触发；external editor focus explicitly NOT verified。
- `src-tauri/target/ui-smoke/evidence/integration-bridge-2026-10-23T09-46-35-758Z-x2YJpP/result.json` 与 cleanup.json。
- `src-tauri/target/security-audit/rust-summary.json`、secrets-summary.json：仅保留脱敏摘要；不上传原始报告。

## 安全扫描保留项

RustSec DB commit `f7b2f6bc663452200bbe639ebb75e0c762b3422b`，工具 cargo-audit 0.22.2；gitleaks 8.30.1。

| 类型 | 包 / 版本 / advisory |
|---|---|
| unmaintained | fxhash 0.2.1 / RUSTSEC-2025-0057；proc-macro-error 1.0.4 / RUSTSEC-2024-0370 |
| unmaintained | unic-char-property 0.9.0 / RUSTSEC-2025-0081；unic-char-range 0.9.0 / RUSTSEC-2025-0075；unic-common 0.9.0 / RUSTSEC-2025-0080；unic-ucd-ident 0.9.0 / RUSTSEC-2025-0100；unic-ucd-version 0.9.0 / RUSTSEC-2025-0098 |
| unmaintained | zbus_names 4.3.1 / RUSTSEC-2026-0371；zvariant 5.10.0 / RUSTSEC-2026-0370；zvariant_derive 5.10.0 / RUSTSEC-2026-0372 |
| unsound | glib 0.18.5 / RUSTSEC-2024-0429；rand 0.7.3 / RUSTSEC-2026-0097 |

未引入 advisory ignore/baseline 或自动升级依赖。Warnings 需要 target/调用可达性与升级评审；门禁通过不代表已完成该评审。

## 未执行或不能据此声称通过

| 项目 | 原因 / 后续 |
|---|---|
| 实际新 Codex/Claude/Hermes/DSH/Kimi/TRAE/ZCode/WorkBuddy 任务、审批与结束 | 未启动真实 Agent、付费任务或修改真实 Hook；原生合成输入不能代替宿主版本验收 |
| live SSH snapshot / remote approval | 未确认测试服务器可达且适合采集；完整 Rust fake-executor 与 Python schema 回归已执行 |
| live WSL snapshot/discovery 与 WSL native scope smoke | 本轮未启用/操作真实来源；离线 continuity 与 Linux metadata fixtures 已覆盖关键回归，不宣称 native host 验收 |
| external editor focus、真实 OS mixed DPI、跨显示器迁移 | isolated native 脚本明确不证明这些能力；CDP DPR 是模拟，不能替代 OS 设置切换 |
| fresh NSIS/MSI 与跨格式升级/卸载 | 安装器 smoke 仅在 disposable CI runner；本轮没运行安装器、删除偏好或覆盖候选包 |
| 长时真实运行、fault injection、所有状态文件损坏恢复 | 本轮回归不是 soak test；审查列出具体缺口，没有运行行覆盖率 |
| MP4 / GIF 成品 | 未确认可靠录屏工具；没有伪造、没有把大影片提交 Git；附件发布需后续明确授权 |
| GitHub Actions 新提交执行 | 没有推送；本地测试不能声称新的远端 CI 已通过 |

## 本地提交与工作树

已完成阶段提交：`19d93a1`（hygiene）、`18753d3`（architecture）、`4be6aa6`（presentation）。第四阶段添加全仓链接/素材/跟踪门禁、修复并接入 Python fixture、扩展开发分支 PR 质量/安全触发。最终 SHA 与工作树以交付时 `git log` / `git status --short` 为准，避免文档自引用提交 SHA。
