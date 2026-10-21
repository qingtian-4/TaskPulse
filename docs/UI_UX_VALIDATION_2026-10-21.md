# 第二轮 README 与 UI/UX 交付验证

> 本文件保留第二轮 `602d74a` 阶段证据。后续视觉升级与当前截图见[第三轮验收](VISUAL_POLISH_VALIDATION_2026-10-21.md)；不要把下文旧 EXE hash 当作当前构建。

日期：2026-10-21。版本 `0.2.0-alpha.7`，分支 `codex/desktop-discovery-visibility`，基线 `c0f5f2f`。本轮只改产品文档、前端与验收脚本，没有改 Provider、生命周期、通知账本、Checkpoint 或缓存算法。

## 已交付

| 范围 | 改动 | 不改变的语义 |
|---|---|---|
| 中英文首页 | 从技术/历史长说明改为产品 → 原生界面 → 场景价值 → 源码开始 → 支持矩阵 → 架构/扩展 → Roadmap → 贡献/致谢 | 无公开 Alpha.7 安装包；保留上游作者、MIT 与 Git 历史 |
| 任务面板 | 两行标题、Agent/项目/位置层次、阶段/结果摘要、可组合搜索与未查看、组内键盘导航、聚焦/悬停样式、有限高度滚动 | 思考/模型等待仍运行；来源故障不伪装失败；查看详情不确认已读 |
| 来源管理 | 四步接入导航、发现候选折叠、已保存/草稿列表、来源读取与全局测试提醒分开、逐项能力与版本、真实功能测试按钮 | 测试事件不创建任务；测试提醒不验收真实 Agent；项目白名单、并发编辑保护和未保存退出确认保留 |
| 高级接入 | Adapter、版本记录、Manifest、诊断、导出与 Hook 差异折叠；孤立来源 Hook 登记独立保留 | 每次写入/撤销仍需预览与明确确认；切换来源重新建立编辑器，旧来源预览不残留到新来源 |
| 素材 | 三张当前原生截图；历史素材归档并注明来源；中英文共用一致图片 | 不冒用上游图、不以示意图/浏览器 mock 代替产品画面，不提高任何宿主验证等级 |

使用操作见 [面板指南](PRODUCT_DASHBOARD.md#alpha7-界面操作)，启动见 [安装指南](INSTALL.md)，真实支持边界见 [兼容矩阵](AGENT_COMPATIBILITY.md)。

## 自动化检查

Windows 10 x64，GNU/MinGW 开发工具链，Tauri 2 / 原生 WebView2。验证目标没有运行中的用户托盘，未覆盖活跃 EXE。使用：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify.ps1 -Frontend -TargetDir src-tauri/target
bun scripts/native-product-smoke.ts --exe src-tauri/target/debug/agent-tray.exe --capture
bun scripts/native-integration-smoke.ts --exe src-tauri/target/debug/agent-tray.exe --notices
bun audit
bun run audit:rust
bun run audit:secrets
```

- Svelte Check：0 错误、0 警告；前端 Build 通过。
- UI model：24 项通过、0 失败，包含搜索/未读交集、跨来源健康隔离、手动读取等待本次成功、不以旧成功冒充刷新。
- 文档链接/素材测试 3 项（含 PNG 尺寸与中英文引用）、版本检查、Release 测试 8 项、安全测试 6 项、integration contract 测试 3 项、缓存工具回归通过。
- Rust fmt、Clippy `-D warnings` 通过；Rust 测试 **390 通过、0 失败、12 ignored**。本轮没有远程/WSL Provider 改动，没有调用真实宿主付费任务或修改用户环境。
- 实际 EXE stdin 桥接 smoke 通过，包含独立测试通道、超限拒绝、隔离路径与锁等待。

### 原生验收与公开截图

`native-product-smoke.ts` 自动复制构建到唯一临时目录，独立 `TASKPULSE_HOME`、来源与 CDP；输入通过真实标准事件接收端，经过既有 Provider → Store → Attention → Transition 流水线。没有覆盖浏览器 IPC 数据或改写画面。仅在布局、隐私和功能断言全部成功后，复制截图到公开 assets。

覆盖：搜索与空结果、全局清除限制、未查看交集、明确请求优先、思考阶段、选择不已读、真实“已查看”动作、长列表滚动、来源保存/读取/测试事件通道、能力降级文案、原生测试通知与 rendered 投递证据。通用来源仍展示无宿主实机版本，不伪称 Codex/Claude 原生输入。

原生 integration + notices smoke 另覆盖沙箱内 Hook 指纹冲突和明确确认、Stop 安全、并行请求、社区描述导入、短 turn 顺序、移除来源后显式撤销、重启不回放通知、长通知标题、点击/队列/15 秒与持续请求。沙箱 Hook 不是实际 Agent 配置。

图片与输入说明见 [素材说明](../assets/README.md)。屏幕直接捕获，无后期拼接、重画或替换文字；通知图是明确标记的测试提醒。

本机最终证据（`src-tauri/target/` 被 Git 忽略，不作为公开下载资产）：

- 产品 UI 与截图：`product-native-yWep28/result.json`；27 条任务全部保留，面板高度 720px，无横向溢出；搜索 + 详情同时打开仍保留可用列表与页脚，空结果和方向键断言通过。
- 接入 + 通知：`integration-native-3m6Pca/result.json`、`ui-smoke/native-notice-result.json`；通知 320 × 132，长标题按钮未溢出，外部编辑器焦点明确未验证。
- EXE 桥接：`integration-bridge-weDzvG`。
- 实际验收 EXE SHA-256：`54b328bc92a04470f7b310e94f6fb3ea5def97973c43f8c5aa3f5a5719fe0f22`。这是 GNU 开发构建，不是已验收 MSVC 安装包。

按要求形成四个逻辑提交：README/文档、Dashboard、来源管理、素材与最终验收；没有重写或压缩上游 Git 历史。

## 安全与未交付范围

- 前端 moderate+ 门禁通过；完整 `bun audit` 仍有 **1 项 low：cookie 0.6.0 / GHSA-pxg6-pf52-xh8x**（SvelteKit 依赖链），没有以门禁通过宣称零风险。
- cargo-audit 无阻断漏洞、无 yanked，registry 检查完整；RustSec DB `4b587f00996ebe5c05a73eaef105ef241e876414`，1348 条公告。保留 **10 unmaintained + 2 unsound** 警告，平台/feature 影响与上游跟进仍见 [安全记录](SECURITY_AUDIT_2026-10.md)。未使用忽略/baseline。
- Gitleaks 完整 Git 历史扫描通过、0 findings，最终提交后复扫同一门禁；不替代未跟踪文件或图片人工隐私审阅。
- 未验收：Kimi/TRAE/ZCode/WorkBuddy 真实宿主 Hook、长期真实工作负载时延、锁屏环境下外部编辑器焦点保持、Windows MSVC 安装器与跨格式迁移、Linux/macOS GUI、系统通知横幅。四款仍按 fixture-tested / 候选能力标注。
- 没有公开 Alpha.7 安装包；没有 push、PR、合并、tag 或发布；未安装真实 Hook、停止用户 Agent、清空用户状态、重置通知账本或修改代理/审批策略。
- 用户原有 Cargo.toml 的 profile.dev 修改及三份未跟踪审查/任务书文档保持原样，不纳入本轮提交。

本报告是当前本机证据，不是新 CI 运行或“所有 Agent 稳定支持”的认证。
