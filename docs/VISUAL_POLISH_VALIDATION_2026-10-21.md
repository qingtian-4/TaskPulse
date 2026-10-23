# 第三轮：桌面视觉升级验收

日期：2026-10-21，分支 `codex/desktop-discovery-visibility`，版本保持 `0.2.0-alpha.7`，基线 `602d74a`。本轮是视觉与交互打磨，不是全仓重构或 Agent 接入验收。

## 实际交付

1. 共享设计系统：`src/app.css` 新增 surface、语义色、间距/圆角、弱阴影、button/chip/form/fold 与可见焦点；详见[设计约定](DESIGN_SYSTEM.md)。
2. Dashboard / TaskList：紧凑五格概览，明确请求入口、计数与激活态；任务采用状态芯片、轻量边缘强调，标题/Agent/项目/Host/活动分层。保留任务数量、未读/固定/不确定性、搜索与筛选；卡片详情、聚焦宿主、已查看不混为一个动作。
3. 来源管理 / IntegrationEditor / HookCleanup：来源选择导航、四步提示、读取/测试/待宿主验收分离、能力边界卡片、统一表单和高级折叠。已有 Hook 预览、明确同意、撤销和未保存编辑保护保留。
4. 通知：320 × 132px 原尺寸，弱侧边状态线、语义图标、标题和按钮统一。只改展示，不改 15 秒、持续请求、轮转、已读与测试事件语义。
5. [三张截图](../assets/README.md)已替换为当前原生 WebView2 捕获，公开合成输入仍清楚标注。README 仅同步验证链接，没有大规模重写。

没有新增 Provider、插件框架、外部字体、网络素材或全屏控制台。没有修改 Rust 采集、AttentionTracker、生命周期、Checkpoint、缓存、代理、模型或审批设置；用户原有 Cargo.toml 修改与三份未跟踪文档不纳入提交。

## 测试结果

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify.ps1 -Frontend -TargetDir src-tauri/target
bun scripts/native-product-smoke.ts --exe src-tauri/target/debug/agent-tray.exe --capture
bun scripts/native-integration-smoke.ts --exe src-tauri/target/debug/agent-tray.exe --notices
bun audit
bun run audit:rust
bun run audit:secrets
```

- 前端 Check：0 错误、0 警告；Build 通过。
- UI model / source setup / notice / visual：28 项通过，覆盖不确定性优先、思考/模型等待仍运行、来源读取隔离、接入步骤不等于验收，以及常用前景/背景 4.5:1 对比度。
- 文档/PNG 测试通过；fmt、Clippy `-D warnings` 通过；Rust 390 项通过、0 失败、12 项按既有 opt-in 要求忽略。Release、安全、integration contract、缓存工具和真实 EXE stdin 桥接回归通过。
- 原生 Product smoke 覆盖搜索、空态、全局清除、需处理筛选、未查看、方向键、思考详情、选择不已读、27 条任务滚动、保存/读取/事件通道测试和通知 rendered；没有导入测试任务到真实用户状态。
- 原生 integration + notice smoke 覆盖沙箱同意/指纹冲突/撤销、独立请求、短 turn 顺序、重启去重、普通提醒计时与悬停、持续请求与轮转、长标题排版和详情点击。

本机证据保留在 Git 忽略的 `src-tauri/target/`：

- `product-native-ouOMq5/result.json`：三页共 9 组 DPR 检查、两种紧凑逻辑窗口和失败结果实际交互均通过；倍图位于该目录，公开截图仅使用 1× 版。
- `integration-native-GvZRaS/result.json`、`ui-smoke/native-notice-result.json`：接入与通知回归；普通提醒的原生投递记录测得约 15 秒收起，确认请求保留，长标题未挤压按钮。焦点实测为 `null`，不算通过。
- `integration-bridge-WjyJhl`：全量验证中的隔离 EXE 桥接结果。
- 最终原生验收构建 SHA-256：`06d0f00a3987e7b8af5f9aa36e45c86cf6dd5b062e7f9d1221d7beaece6e8090`。这是 GNU 开发构建，不是 MSVC 安装包。

安全扫描：前端 moderate+ 门禁通过，完整报告仍有 **1 项 low**（cookie 0.6.0）；RustSec 数据库 `4b587f00996ebe5c05a73eaef105ef241e876414`（1348 条）查询完整、无阻断漏洞或 yanked，仍有 **10 unmaintained + 2 unsound** 警告，未通过忽略规则隐藏。既有平台/feature 风险说明见[安全记录](internal/reports/SECURITY_AUDIT_2026-10.md)。Gitleaks 在最终提交后复扫完整 Git 历史，不冒称覆盖未跟踪文件或图片。

### 缩放证据边界

三页分别在原生 WebView2 CDP `deviceScaleFactor=1 / 1.5 / 2` 下，以固定逻辑窗口检查无横向溢出、状态/标题重叠、可见控件遮挡，PNG 倍图保留在本机 target 证据目录。滚动区外的控件需滚动进入视图，不冒称可在视图外点击。

另加测 DPR 2 下 440 × 480 的 Dashboard（打开详情仍保留任务与页脚）和 640 × 600 的来源页（纵向滚动，不横向溢出）。这是模拟缩放 / 紧凑逻辑窗口证据，**不是**修改 Windows 全局显示设置或跨不同 DPI 显示器拖动实测。后者仍需人工验证；没有为完成测试自动改变用户桌面设置。

### 人工验收尚待进行

- Windows 实际显示比例 100% / 150% / 200% 与跨屏迁移；系统额外文字放大、高对比度和屏幕阅读器。
- 外部编辑器焦点保持：当前隔离锁屏验收明确未验证，不把空值算作通过。
- Kimi / TRAE / ZCode / WorkBuddy 真实宿主 Hook 与长期真实任务时延；仍保持 fixture-tested / 候选支持等级。
- MSVC 安装器、干净系统、跨格式迁移、Linux/macOS GUI 与系统横幅；本轮不是发布候选安装包验收。

未 push、PR、合并、tag 或发布；不安装真实 Hook、不停止真实 Agent、不删除用户状态。原生测试只操作唯一隔离目录与其进程，测试结束后退出。
