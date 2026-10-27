# TaskPulse 品牌资源

来源：维护者确认的 `TaskPulse-Logo-Kit.zip`，design-handoff-2026-10-27。
`brand.svg` 是固定的双色开口 P 母版；其几何形状未修改。
SVG 状态图只改变 P 的环颜色，应用/安装器/窗口图标始终使用固定品牌色。

README 的 `<picture>` 使用 `wordmark-dark.svg` / `wordmark-light.svg`，浅色为回退。
Dashboard、消息中心和来源管理通过 `BrandMark.svelte` 使用同一母版的副本
`src/lib/assets/brand.svg`，由 Vite 打包，避免依赖安装目录之外的文件。

## GitHub Social Preview（手动）

仓库维护者打开 **Settings → General → Social preview → Edit → Upload an image**，
上传本目录的 `social-preview.png`（1280×640），保存后检查仓库链接预览。
提交此文件不会自动更新 GitHub 设置；本次没有修改仓库设置。

## 桌面应用与安装器

使用仓库锁定的 Bun/Tauri CLI：

```powershell
bun run tauri icon assets/brand/brand.svg --output src-tauri/target/brand-generated
```

审核生成结果后，将顶层桌面 PNG、ICO、ICNS 复制到 `src-tauri/icons/`；
不要覆盖无关移动平台产物。当前 Windows ICO 由该命令生成，NSIS/MSI 使用
`tauri.conf.json` 既有 `bundle.icon`，NSIS `installerIcon` 也显式指向同一 ICO；identifier、upgradeCode、版本、安装模式、
WebView2 策略和用户配置路径保持原样。生成时 Windows 字体枚举可能警告，
品牌母版没有文本/字体依赖。macOS 彩色应用 ICNS 同步更新；菜单栏单色模板另行设计。

托盘使用交付包原始 32px RGBA PNG，映射为 `tray-working`、`tray-needs-input`、
`tray-error`、`tray-idle` 和 `tray-review`，全部编译时嵌入。
`tray-offline` / `tray-starting` 兼容资源采用红色，启动文字注明状态待更新。
Windows Shell 负责把 32px 图像适配任务栏尺寸；不新增 DPI 轮询或系统缩放设置修改。
16/20/24/32px 显示效果及跨屏缩放仍须实际任务栏验收，不能以 WebView DPR 模拟替代。

## 聚合规则

| 顺序 | 图标 | 判定 |
|---|---|---|
| 1 | 橙黄 | 新鲜、来源未退化的明确待处理请求（approval/question/quota） |
| 2 | 红 | 未查看的任务失败、来源异常/断连、stale/unknown/未分类等待、首次采集中 |
| 3 | 浅蓝 | 已结束且任务结果未查看，包括明确 turn end 和现有非 Agent tmux shell-return 推断 |
| 4 | 青绿 | 可信进行中；thinking/tool/responding/awaiting model/tool 均保持运行 |
| 5 | 灰 | 已发布的关注快照没有以上事实；没有启用来源时表示当前无监控工作 |

所有数量保留在 tooltip；任务失败与来源异常分别计数。橙黄请求优先于其他任务失败，
但其自身来源退化/过期时转为状态待更新，不能声称仍是新鲜审批。
已查看但仍固定的历史失败不持续触发红色。任务结果 unread 与通知 seen 独立；
阅读消息、查看结果均不能解决请求。较早关注/隐藏/忽略遵守现有 AttentionTracker 规则。

`tray.rs` 只读取已完成 scope/store/attention 的 DashboardSnapshot，拒绝落后 revision，
仅在图标或 tooltip 改变时写入原生托盘，写入失败不缓存成功。没有新增通知、轮询、
任务状态迁移或 Agent Hook。来源的现有 stale/健康判定负责不确定状态；不增加基于静默的推断。

## 验收与回滚

本次证据与未执行项见 [品牌集成验收记录](../../docs/BRAND_VALIDATION_2026-10-27.md)。
回滚时撤销本次品牌集成提交并重建应用；无需回滚/删除任何用户状态或修改 Agent 配置。
安装器升级和 Windows 图标缓存必须在独立验收机器检查，不以开发机手动清缓存冒充升级通过。
