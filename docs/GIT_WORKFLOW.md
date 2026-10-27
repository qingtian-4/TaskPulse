# TaskPulse 的 Git 与 PR 入门

## 本轮 README 如何进入 main（2026-10-27）

已通过 GitHub API 核对远端 `main` 为 `fe48e69`。本轮 README 整理开始时，
`codex/p0-remote-acceptance` 为 `887a7f7`，包含 main，另外有 9 个提交，涉及候选文档、
macOS 窗口、MCP、豆包与品牌集成；本轮 README 提交在其上继续。

**推荐通过 PR 合入。PR 是审查入口，merge 是更新 main 的动作，不是二选一。**

```text
base：main                         ← 接收改动
head：codex/p0-remote-acceptance   ← 提供改动
```

先提交并推送功能分支，再在 GitHub 创建上述方向的 PR，审查全部差异，等待 Windows Quality
和 Security 检查通过，然后由维护者合并。合并后 GitHub 默认首页才会展示新版 README。
本次编辑仅创建本地提交，没有替维护者推送、创建 PR、合并或发布。

整分支 PR 会包含上述全部未合入提交。若只想更新首页，应从最新 main 创建专门文档分支，
选择 README 提交并带齐依赖的 Logo、截图和说明文件；不能假设最后一个提交已经自带所有资源。
不要将 main 合入功能分支后，误以为 main 已经收到这些修改。合入源码也不会自动发布新安装包。

## 历史：首页为何仍显示 AgentTray（2026-10-11）

截至 2026-10-11 排查时，仓库 `qingtian-4/TaskPulse` 是独立的 GitHub 仓库
（并非 GitHub 平台标记的 fork），但保留了 AgentTray 的 Git 历史：

- 默认分支 `main` 停在上游的 `9542685`，因此首页 README 和文件版本还是 AgentTray。
- TaskPulse 的改进在 `codex/reliability-ui`，有 61 个尚未进入 `main` 的提交。
- GitHub API 确认已推送的 `d2d463a` 归属 `qingtian-4`，不是邮箱关联错误。
- 推送功能分支不会更新 `main`；黄色 Compare & pull request 是建议创建合并提案。
  当时 `main` 没有分支保护，并非平台强制要求 PR。

这是排查基线，不代表当前远端分支的实时状态。PR 合并后，默认分支首页才会展示
TaskPulse。贡献统计可能有缓存，原作者仍应保留，不需要删除 `.git`、强推或改写作者。

## 分清五件事

| 概念 | 本项目含义 |
|---|---|
| 仓库所有者 | `qingtian-4`，管理 TaskPulse 仓库；不会自动变成历史提交作者 |
| Commit 作者 | 每个提交保存的 author 名称/邮箱；GitHub 根据邮箱关联账号 |
| 贡献者列表 | 统计默认分支的合格提交，不是可以随意改写的“项目主人”字段 |
| 分支 | 不同版本的指针；`main` 是集成基线，功能分支是隔离的改进工作 |
| PR（Pull Request） | 请求将 head 分支的改动合入 base 分支，附带说明、差异、讨论和检查 |

`commit` 保存到本地 → `push` 上传该分支 → PR 提议合并 → 检查和审阅 →
`merge` 更新目标分支。前两个步骤不会自动替代后两个步骤。

### 当时 PR 的方向

```text
仓库：qingtian-4/TaskPulse（你的仓库）
base：main                  ← 接收改动
head：codex/reliability-ui   ← 已完成改动
```

这不是向 `sprklai/agenttray` 发 PR，无须请上游作者批准。维护者可以在自己的仓库
创建、审阅和合并 PR；若以后设置分支保护，应按规则满足检查，不绕过它。

个人项目可自查并合并自己的 PR，但不能用自己的 Approve 满足“需要其他人批准”
的规则。不要在只有一个维护者时盲目开启必须他人批准的保护要求。

GitHub PR 页面：Conversation 看说明，Commits 看提交，Files changed 看差异，
Checks 看自动验证。检查失败或冲突需要先处理，不应只为了换首页而强行合并。

## 在开源项目基础上独立演进

TaskPulse 采用 MIT：允许修改和分发，但保留原始版权与许可声明。
`LICENSE` 保留 SprklAI 的声明，并为 TaskPulse 修改增加自己的声明；README 说明
来源、维护者、改进及限制。原作者贡献仍可显示，这与 TaskPulse 是你的维护项目不冲突。

- `origin` 指向 `https://github.com/qingtian-4/TaskPulse.git`，推送本项目。
- 可选 `upstream` 指向 AgentTray，用于主动获取上游变化，不自动同步或推送。
- `CODEOWNERS` 是当前代码审阅责任，不是版权清单；TaskPulse 使用自己的维护者。
- Issue 模板、联系入口和 Release 名称应指向 TaskPulse，不把责任转给上游。
- 不把父项目的全部提交改署成自己。改名、clone、push 都不会改变原有作者。

添加只读用途的上游（仅在尚无该 remote 时执行）：

```powershell
git remote add upstream https://github.com/sprklai/agenttray.git
# 防止误推送到上游；fetch URL 保留不变。
git remote set-url --push upstream DISABLED
```

“基于 AgentTray 的派生项目”不要求 GitHub 页面必须显示 fork 标签。已有独立仓库
无需重新建仓库或抹去历史，也不需要给上游提交 PR 才能发布自己的 MIT 派生代码。

## 以后每次开发的建议流程

先检查工作区，不覆盖或删除未提交文件；有改动时先安全提交或另行保存。

```powershell
git status
git switch main
git pull --ff-only origin main
git switch -c codex/short-topic

# 修改后运行 scripts/verify.ps1 -Frontend 和 bun run test:ui-model。
# 有真实来源/GUI验证要求时，单独执行并说明证据边界。
git add <本次改动的明确文件路径>
git commit -m "fix: describe the focused change"
git push -u origin codex/short-topic
```

在自己仓库创建 PR，base 选 `main`，head 选该功能分支，填写变更、验证、风险和
迁移说明；通过 CI 后审阅并合并，再拉取最新 `main`，下一项任务另开短期分支。
不要把所有后续开发永久堆进一个巨大的功能分支。

初次整合 TaskPulse 采用 **Create a merge commit**：保留上游和你的逐次开发提交。
Squash 会合成一个提交，Rebase 会改写提交 ID；它们各有用途，但不是修复贡献者
显示的必要动作。不要对已发布的共享分支无约定地改写历史或强推。

`Windows quality` 校验工作流只做锁定依赖安装、Svelte/前端测试构建和 Rust
fmt/clippy/tests/build，不发布安装包，不运行需要真实账号/服务器的 ignored 测试。
通过 CI 不代表所有 GUI、WSL/SSH 或任务通知路径都通过了真机验收。

## 如果贡献者仍未显示

1. 确认改进提交已合入默认分支，而不仅是上传到功能分支。
2. 点开提交，确认 GitHub 作者链接是自己的账号。
3. 若关联错误，检查 `git config user.email` 和 GitHub Settings → Emails；本次已关联，
   不需要改邮箱或重写历史。
4. 等待 GitHub 更新统计；不要通过删除原作者来“修复”展示。

## 官方依据

- [贡献者统计与默认分支](https://docs.github.com/en/repositories/viewing-activity-and-data-for-your-repository/viewing-a-projects-contributors)
- [PR 的工作方式](https://docs.github.com/en/pull-requests/reference/pull-requests)
- [Merge / Squash / Rebase 的区别](https://docs.github.com/en/pull-requests/reference/pull-request-merges)
- [提交邮箱与贡献关联](https://docs.github.com/en/account-and-profile/how-tos/contribution-settings/troubleshooting-missing-contributions)
- [MIT License](https://opensource.org/license/mit)
