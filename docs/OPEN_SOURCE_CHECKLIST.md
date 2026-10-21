# TaskPulse 首次公开 Alpha 交付清单

更新：2026-10-22（Asia/Shanghai）。这是发布决策入口，不是历史功能清单。
源码仓库已经公开；**源码公开、草稿生成、安装包公开、稳定支持是四件不同的事**。

## 当前结论

Alpha.7 已在准确源 SHA `b5dc64a760bae916c9d40b147616f9720405887b` 通过 main
质量/安全 CI，并由[发布运行 39636404358](https://github.com/qingtian-4/TaskPulse/actions/runs/39636404358)
生成候选安装包和 Draft。两种格式的独立 CI 安装验收与四项资产 SHA256 核对完成，
详见[候选验收记录](ALPHA7_CANDIDATE_VALIDATION_2026-10-22.md)。尚未公开发布。

第 3 项独立实机验收已按维护者要求暂缓，不计为通过。当前建议保留 Draft；
不要把 CI Windows Server 当成无开发工具的 Windows 10/11，也不要公开旧 Alpha.6
草稿冒充修复版。下一步是维护者核对实际证据与未完成门禁，不再扩大功能范围。

| 交付项 | 状态与证据 |
|---|---|
| 产品说明 | 中英文 README、来源能力矩阵、安装/贡献指南已具备；不声称万能插件或真实完成百分比 |
| 版权与维护 | MIT 保留 AgentTray 版权与提交历史，TaskPulse 改动单独署名；CODEOWNERS、反馈和安全说明指向本维护者 |
| 基础工程 | Windows quality、锁定依赖、版本/SHA 校验、EXE/MSI 命名、SHA256 与草稿工作流已具备 |
| Alpha.7 安装 | NSIS/MSI 各自在独立 Windows runner 通过真实安装、版本、UI/IPC、重启、同版本重装、卸载；不是开发机安装 |
| Alpha.7 发布 | 标签 `v0.2.0-alpha.7` 和 Draft `419811221` 已生成；未公开，无公共安装包下载 |
| Alpha.7 安全修复 | 前端 moderate+、Rust 漏洞/撤回检查、完整历史密钥扫描本地及候选 CI 通过；残余警告保留，见下文 |
| 实机与持续使用 | 暂缓，不计为通过；干净系统、真实新 Agent、跨版本/格式迁移、焦点和长期时延保持未验证 |
| 扩展 Agent 等级 | Kimi/TRAE/ZCode 仍为 fixture-tested，WorkBuddy 候选；安装器 CI 不提升宿主等级 |

### Alpha.6 的历史证据边界（不替代 Alpha.7）

- 源 SHA：`7a1929aaf1dfb98f425d2868c251865e184fefd5`；不可变标签 `v0.2.0-alpha.6`。
- [PR #7](https://github.com/qingtian-4/TaskPulse/pull/7)；
  [发行 38694054589](https://github.com/qingtian-4/TaskPulse/actions/runs/38694054589)。
  2026-10-19 公开页面复核：构建、两个安装矩阵 job、草稿生成均成功。
- 2026-10-15 交接记录已保存的下载核验：MSI evidence `11953903152`、NSIS
  evidence `11953833264`、installers `11953483244`，草稿 ID `413503171`。
  两个 result.json 为 passed，原生 IPC 为 embedded=true、onboarding=false、
  tasks=0、prompts=0；资产和 ZIP digest 已核对。详见[原始记录](PROJECT_REVIEW_2026-10.md#2026-10-15-连接恢复后的最终核验)。
- 2026-10-19 复核受 GitHub 连接器 transport error、匿名 API 限流影响，未重新下载 Alpha.6
  私有草稿资产。2026-10-22 已重新下载并核验的是 Alpha.7；不要混用两个版本的下载记录。
- 签名 NotSigned；跨版本升级、格式迁移、焦点证据为 not_verified。

## 安全审查记录

2026-10-19 对原锁文件执行 `bun audit`：32 条告警（14 high、16 moderate、2 low）。
在现有主版本范围内升级，未切换到 SvelteKit 3。当前锁文件为 Kit 2.70.3、
Svelte 5.57.2、Vite 8.3.4，及修复后的 devalue、PostCSS、nanoid、source-map-js。
复查仅剩下面一条 low；CI/verify/release 的 `bun run audit:frontend` 会阻止
中等及以上告警。阈值通过不等于零告警，也不代替 Rust 或整体安全审计。

剩余：[GHSA-pxg6-pf52-xh8x](https://github.com/advisories/GHSA-pxg6-pf52-xh8x)，
`@sveltejs/kit@2.70.3 -> cookie@0.6.0`，修复版本为 cookie 0.7.0，超出父依赖范围。
当前使用 adapter-static 且关闭 SSR，安装版不运行 SvelteKit Cookie 服务端；这是
基于架构的暴露面判断，不是漏洞已修复的证明。暂不强行覆盖传递依赖或升级框架
主版本。公开下一候选前重新审查此项（最晚 2026-11-19）；若增加 SSR/服务端路由，
必须立即重评，不沿用此判断。

现已补专用审查：cargo-audit 0.22.2 完整复查为 0 漏洞、0 已撤回依赖；仍有
10 项 unmaintained 和 2 项 unsound 警告，未隐藏或标为已修复。Gitleaks 8.30.1
对完整本地 refs/合并差异扫描为 0 命中，不能据此保证未知凭据格式、不可达对象
或全部个人信息安全。工具只在忽略目录中运行，未全局安装。详细版本、公告、
依赖链和覆盖边界见[安全审查记录](SECURITY_AUDIT_2026-10.md)。

### 历史本地回归（不是 Alpha.7 安装器验收）

完整 `scripts/verify.ps1 -Frontend` 通过：审计阈值通过（剩 1 low），Svelte 0 错误/
警告，UI 模型 19，发布模型 8（75 断言），安全模型 6（45 断言），Rust 333 通过/
7 opt-in 跳过，fmt/clippy、前端生产构建和 Alpha.7 内嵌原生构建通过。
隔离 Edge + mocked IPC 的 28 任务 UI 冒烟通过，实测 440×720、可滚动、无横向
溢出，筛选、新项目、已查看和管理页交互正常；截图位于忽略提交的 target 目录。
测试浏览器与 Vite 服务已关闭。没有修改用户监控配置、真实 Agent 会话或 Hook。
补充 Alpha.7 真实 WebView2 提醒验收通过：320×132 深色、长标题/按钮不溢出、
普通结果约 15 秒收起、批准不超时消失、队列/旧回调保护、鼠标悬停暂停、提醒不
夺主面板焦点，未创建假任务或修改未读状态。首次因真实提醒占用而安全拒绝；
原弹窗不再占用后才再次执行。配置已恢复，测试进程和 CDP 已关闭。Windows
通知 API 接受测试调用不等于肉眼看到系统横幅，也不等于外部编辑器焦点或真实
Agent 新事件链路验收。这批历史回归未验证安装器；Alpha.7 后续安装器 CI 见上方
2026-10-22 记录，干净系统与真实长期时延仍未验证。

## 按顺序完成后再公开安装包

1. **安全和版本**：提交/审查 Alpha.7 改动及残余警告；安全扫描已补齐，但扫描
   不是全面渗透测试/隐私认证。候选版本已确定，不覆盖 Alpha.6 标签或资产。
2. **候选重建（Alpha.7 已完成）**：该源提交的 main CI 与两种独立安装器验收通过，
   SHA、`--version`、manifest 已复核。未移动标签；后续代码提交不自动包含在已有安装包中。
3. **干净 Windows（暂缓，不计为通过）**：至少一台无开发工具的 Windows VM，记录系统/WebView2 版本、
   在线安装、重启、卸载、用户状态保留。验证缺少 WebView2 和网络受限时的行为。
4. **核心使用路径（暂缓，不计为通过）**：由用户在实际使用的至少两类 Agent 中新建任务，记录开始、
   长时间思考、工具调用、明确结束、重命名与提醒。支持的批准事件要验证批准/解除，
   不支持时准确说明。确认不误报 Guardian、不漏掉范围内新项目，记录样本量和时延。
5. **提醒与恢复（实机门禁暂缓）**：实际看到弹窗且不抢编辑器焦点；验证关闭不等于已查看、待处理
   请求回查、重启不重复旧完成通知。实测跨版本/安装格式迁移与自定义目录保留。
6. **呈现和发布**：补一段脱敏实录（两个任务并行→一个结束→提醒→返回对话），
   与现有“示意图”明确区分。填写[实机门禁](RELEASING.md#从草稿到可下载-alpha-的人工门禁)，
   核对 unsigned 提示和支持边界后，由维护者决定是否手工 Publish pre-release。

当前只准备维护者决策材料，不执行第 3 项，不自动 Publish；本轮也不改写远端草稿、
资产或既有标签。暂缓验收不是验收通过，也不是已接受全部发布风险。

不把未来 run/attach、Claude WSL/SSH、Hermes WSL、动态插件市场列为首个 Alpha
已实现能力。这些缺项可以留在路线图；真实状态和通知可靠性不能靠路线图代替。

每项实测至少记录：日期、候选 SHA/安装器哈希、Windows/Agent 版本、来源与配置范围、
预期/实际结果、脱敏证据路径、通过/失败/未验证。不要在公开 Issue 附完整会话、
命令、密钥或私人服务器配置。
