# TaskPulse 安全演示录屏脚本（尚未录制）

本文件是**待执行的录制方案**，不是已经存在的演示视频。录制只应使用原生 Windows Tauri/WebView2 程序，且必须注明合成数据与真实宿主的证据边界。

PR #10 提供的是 storyboard，没有可执行录屏器。本轮保留已有三张原生截图；未提供 MP4/GIF 成品。当前终端没有发现 `ffmpeg` 可执行文件，也未确认可靠的可交互桌面采集能力，因此不以 CDP 连续截图拼成所谓屏幕录制。

## 可复现的原生界面检查

在 Windows PowerShell 中执行；不会改真实 Agent 设置或安装真实 Hook：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify.ps1 -Frontend
bun scripts/native-product-smoke.ts --exe src-tauri/target/codex-check/debug/agent-tray.exe
bun scripts/native-integration-smoke.ts --exe src-tauri/target/codex-check/debug/agent-tray.exe --notices
```

两个原生脚本必须顺序执行，临时 CDP 9337 被占用时会拒绝附着其他应用。脚本复制构建到自有目录，创建独立 TASKPULSE_HOME，只启用公开合成 Generic Event 来源，保留 result/failure/PNG 到被忽略的 `src-tauri/target/ui-smoke/evidence/`，仅清理自身临时目录。无 `--capture` 时不会替换仓库公开 PNG；该选项会覆盖三个素材，只应在人工审阅后显式使用。沙箱中的 manifest 配置回归不是安装真实宿主 Hook。

已存在的 PNG 与尺寸/输入说明见 [素材来源](../assets/README.md)。原生 smoke 不生成影片，不证明外部编辑器焦点、真实 Agent 版本支持或真实 OS 混合 DPI。运行时的提权/锁屏状态可能让 WebView2 CDP 不可用，应保存失败证据，不能弱化 CSP 或跳过 native checks。

## 推荐录制（45–60 秒，16:9，1080p）

| 时间 | 画面 | 讲述/字幕 |
|---|---|---|
| 0–6s | 原生 TaskPulse 托盘与 Dashboard | 多个 Coding Agent，一处了解进展 |
| 6–16s | 隔离的合成任务进入 Running，显示项目/来源 | 查看谁在执行，不再来回切窗口 |
| 16–27s | 插入一条明确的 Question/Approval 标准事件 | 只有确切请求才提醒，不猜测审批 |
| 27–40s | 独立原生提醒与“需处理”，打开详情 | 返回原 Agent 处理，TaskPulse 不替你批准 |
| 40–50s | 标记已查看，显示另一个 TurnEnded | 完成与等待输入是不同状态 |
| 50–60s | 来源管理与品牌/架构画面 | 本机、WSL 和 SSH 各按支持范围独立接入 |

## 录制约束

1. 默认用隔离的 `TASKPULSE_HOME`、公开 fixture 和 WebView2 原生窗口。不访问或变更真实 Hook、认证、模型、代理及用户会话；不得启用真实用户的私有来源。
2. 只使用明确标记为 **Synthetic demo / 合成演示** 的任务名称、项目和数据。视频内显著注明“真实应用界面，合成事件输入；不代表真实宿主验收”。
3. 若另做真实 Agent 演示，须先人工审核界面、文件路径、终端、通知及会话文本，并在用户授权下录制；真实任务与合成演示分别命名。
4. 禁止在视频中显示凭据、私有 SSH 主机、真实用户名、邮箱或私人日志。不要以开源仓库的旧 AgentTray GIF 充当 TaskPulse。
5. 推荐 GIF：只导出不超过 12 秒的精简静音流程，压缩后目标 2–5 MB。完整版使用 MP4（放 Release、GitHub 附件或经确认的外部平台），README 优先保留静态 PNG 和链接，避免大体积 GIF 增加仓库历史。
6. 原生截图已存在于 `assets/screenshots/`，可直接用于 README；生成新的截图应保留实际采集工具、窗口比例和输入来源的 provenance 说明。

## 真实安装包公开前还必须完成

至少一台干净 Windows 10/11 机器的 NSIS/MSI 体验；两种已安装 Coding Agent 的实际新任务与提醒场景；长时运行中的 WSL 降级与恢复；重启后不重复提醒；明示 unsigned 安装提示和支持范围。测试通过才考虑公开下载。

跟踪：`docs/OPEN_SOURCE_CHECKLIST.md` 和 `docs/RELEASING.md`。

## 媒体交付与 Release 附件

录制前人工确认交互桌面可用，关闭其他私人窗口/系统通知，仅选择 TaskPulse 窗口或明确裁剪区。45–60 秒视频全程显示“真实应用界面，合成事件输入”；允许分别出现多品牌，但必须写明品牌是合成标签，不是该宿主产生的任务。现有 native-product 的通用事件品牌保持诚实，不改称真实 Codex/Claude 输出。

后续可使用人工选择窗口的系统录屏器或 OBS 保存至被忽略的 `recordings-local/`。原始视频与压缩 MP4 均不进入 Git；制作 ≤12 秒、≤5 MB 的 GIF 摘要前先人工审查字幕、窗口路径和每个通知。若已安装 FFmpeg，可采用以下离线压缩模板（它本身不负责屏幕采集）：

```powershell
ffmpeg -i recordings-local/taskpulse-raw.mp4 -an -c:v libx264 -crf 24 -preset slow -pix_fmt yuv420p -movflags +faststart recordings-local/taskpulse-demo.mp4
ffmpeg -ss 6 -t 12 -i recordings-local/taskpulse-demo.mp4 -filter_complex "[0:v]fps=10,scale=720:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=80[p];[b][p]paletteuse" recordings-local/taskpulse-demo.gif
```

附件提案：`TaskPulse_<version>_windows_synthetic-demo.mp4`、GIF、SHA256 与 provenance 文本，记录构建 SHA、OS/WebView2 版本、合成输入和未验证事项。仅在维护者授权后上传至明确的 Release/附件；不覆盖现有 Draft、Tag 或资产。README 保留轻量 PNG，并在成品审核后加入附件链接。本轮没有上传或发布媒体。
