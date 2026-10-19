# TaskPulse 首次公开 Alpha 交付清单

更新：2026-10-19（Asia/Shanghai）。这是发布决策入口，不是历史功能清单。
源码仓库已经公开；**源码公开、草稿生成、安装包公开、稳定支持是四件不同的事**。

## 当前结论

Alpha.6 的构建和独立安装验收已通过，仍为草稿。现在的优先级不是再添加 Agent，
而是把已有的“多任务一眼查看、明确需要处理时提醒”可靠地交付给第一批用户。
本次审计发现了旧依赖告警，当前分支正在修复；不要公开旧草稿冒充修复版。

| 交付项 | 状态与证据 |
|---|---|
| 产品说明 | 中英文 README、来源能力矩阵、安装/贡献指南已具备；不声称万能插件或真实完成百分比 |
| 版权与维护 | MIT 保留 AgentTray 版权与提交历史，TaskPulse 改动单独署名；CODEOWNERS、反馈和安全说明指向本维护者 |
| 基础工程 | Windows quality、锁定依赖、版本/SHA 校验、EXE/MSI 命名、SHA256 与草稿工作流已具备 |
| Alpha.6 安装 | 两种格式各自在全新 Windows runner 通过安装、版本、UI/IPC、重启、同版本重装、卸载 |
| Alpha.6 发布 | 草稿生成成功，尚未公开；不是稳定版 |
| 后续安全修复 | 已在本地更新前端依赖及审计门禁并通过完整验证；不在旧标签/安装包中，必须经新的发布候选验收 |
| 实机与持续使用 | 干净系统、真实新 Agent 事件、跨版本/格式迁移、焦点与长期时延仍未全部验收 |

### Alpha.6 的证据边界

- 源 SHA：`7a1929aaf1dfb98f425d2868c251865e184fefd5`；不可变标签 `v0.2.0-alpha.6`。
- [PR #7](https://github.com/qingtian-4/TaskPulse/pull/7)；
  [发行 38694054589](https://github.com/qingtian-4/TaskPulse/actions/runs/38694054589)。
  2026-10-19 公开页面复核：构建、两个安装矩阵 job、草稿生成均成功。
- 2026-10-15 交接记录已保存的下载核验：MSI evidence `11953903152`、NSIS
  evidence `11953833264`、installers `11953483244`，草稿 ID `413503171`。
  两个 result.json 为 passed，原生 IPC 为 embedded=true、onboarding=false、
  tasks=0、prompts=0；资产和 ZIP digest 已核对。详见[原始记录](PROJECT_REVIEW_2026-10.md#2026-10-15-连接恢复后的最终核验)。
- 本次 GitHub 连接器 transport error、匿名 API 限流，未重新下载私有草稿资产；
  不把之前的下载记录描述为本次下载。公开 Releases 页面仍无已公开版本。
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

当前跟踪文件与本地所有 Git refs 的常见 token/私钥正则初筛无匹配；只输出文件名/
提交 ID，未输出凭据。它不覆盖未知格式、不可达对象或全部个人信息。当前机器
未安装 cargo-audit，也未运行专用历史泄密扫描；这些项目仍为未完成，不能写成全绿。

### 本次本地回归（不是新安装器验收）

`bun install --frozen-lockfile` 无改动；完整 `scripts/verify.ps1 -Frontend` 通过：
审计阈值通过（剩 1 low），Svelte 0 错误/警告，UI 模型 19，发布模型 8（75 断言），
Rust 333 通过/7 opt-in 跳过，fmt/clippy、前端生产构建和内嵌原生构建通过。
隔离 Edge + mocked IPC 的 28 任务 UI 冒烟通过，实测 440×720、可滚动、无横向
溢出，筛选、新项目、已查看和管理页交互正常；截图位于忽略提交的 target 目录。
测试浏览器与 Vite 服务已关闭。没有修改用户监控配置、真实 Agent 会话或 Hook。
该浏览器测试不是新版依赖下的原生通知/安装/焦点验收，不能直接复用 Alpha.6 结论。

## 按顺序完成后再公开安装包

1. **安全和版本**：提交/审查当前改动；补 Rust advisories 与专用 Git 历史泄密审查。
   处理或明确记录残余风险。确定新的预发布版本，不覆盖 Alpha.6 标签或资产。
2. **候选重建**：新候选通过 Windows quality 和两个独立安装器验收，重新核对 SHA、
   `--version`、manifest。旧版本安装通过不能替代依赖更新后的验收。
3. **干净 Windows**：至少一台无开发工具的 Windows VM，记录系统/WebView2 版本、
   在线安装、重启、卸载、用户状态保留。验证缺少 WebView2 和网络受限时的行为。
4. **核心使用路径**：由用户在实际使用的至少两类 Agent 中新建任务，记录开始、
   长时间思考、工具调用、明确结束、重命名与提醒。支持的批准事件要验证批准/解除，
   不支持时准确说明。确认不误报 Guardian、不漏掉范围内新项目，记录样本量和时延。
5. **提醒与恢复**：实际看到弹窗且不抢编辑器焦点；验证关闭不等于已查看、待处理
   请求回查、重启不重复旧完成通知。实测跨版本/安装格式迁移与自定义目录保留。
6. **呈现和发布**：补一段脱敏实录（两个任务并行→一个结束→提醒→返回对话），
   与现有“示意图”明确区分。填写[实机门禁](RELEASING.md#从草稿到可下载-alpha-的人工门禁)，
   核对 unsigned 提示和支持边界后，由维护者决定是否手工 Publish pre-release。

不把未来 run/attach、Claude WSL/SSH、Hermes WSL、动态插件市场列为首个 Alpha
已实现能力。这些缺项可以留在路线图；真实状态和通知可靠性不能靠路线图代替。

每项实测至少记录：日期、候选 SHA/安装器哈希、Windows/Agent 版本、来源与配置范围、
预期/实际结果、脱敏证据路径、通过/失败/未验证。不要在公开 Issue 附完整会话、
命令、密钥或私人服务器配置。
