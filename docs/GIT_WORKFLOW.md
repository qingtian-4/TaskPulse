# Git 与 PR 工作流

使用短期 `codex/*` 分支，通过 PR 合入 `main`。开始前检查 `git status`，更新远端引用；有其他任务正在开发时使用独立 worktree，不移动或覆盖它的修改。

```sh
git fetch origin
git worktree add -b codex/topic ../taskpulse-topic origin/main
```

已有功能继续在它自己的分支和 worktree 中开发，不重新创建同名分支。

## 提交与审查

1. 只暂存当前任务的明确文件，检查完整 diff，包括新增文件。
2. 运行适用的测试与 [开发检查](DEVELOPMENT.md)。文档/素材变动至少跑 `bun run test:docs` 与 `bun run test:repo`。
3. 以小的逻辑提交保存修改，推送功能分支，创建目标为 `main` 的 PR。
4. 保留最新主分支的 README、品牌与截图。冲突逐项审查，不用旧分支整份覆盖新文档。
5. 核对当前 PR head 的 Windows quality 与 Security；平台或安装改动需要对应的原生/安装验收。

维护者要求先实机测试时，保持 Draft，等待测试稳定确认后才合并。绿色 CI 不等于已经发布安装包或完成所有实机验收。

## 合并与清理

优先使用 merge commit 保留依赖历史。合并后重新 fetch，确认当前分支 tip 已包含在 `origin/main`，工作区无未提交内容，再删除该任务的分支和 worktree。若 tip 已变化，重新检查；本地 main 有独有提交时审查其内容，不能强制 reset 丢弃。

不要改写共享历史或移动已发布标签。删除当前追踪文件不能清除历史中的内容；发现真实凭据暴露时先报告并轮换凭据，历史重写需要单独协调。

## 公开文件范围

`.gitignore` 排除本地计划、审查报告、原始截图、旧演示材料、开发工具配置和构建产物。已经追踪的文件需用 `git rm --cached -- <明确路径>` 移除追踪，本地原件保留。

正式用户指南、接口协议、源码、测试、工作流、锁文件、生产品牌资源和许可声明仍需提交。发布和标签的规则见 [RELEASING](RELEASING.md)。
