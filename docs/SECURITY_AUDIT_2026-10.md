# Alpha.7 候选安全检查记录

日期：2026-10-19（Asia/Shanghai）。适用本地候选分支，不适用 Alpha.6 的旧安装器。
安全扫描有覆盖边界；这里记录的是明确检查结果，不是“项目绝对安全”的认证。

## 可重复运行

```powershell
bun run audit:frontend
bun run audit:rust
bun run audit:secrets
bun run test:security
```

`audit:frontend` 阈值为 moderate，完整低等级报告另用 `bun audit`。Rust 和 Git
扫描的 Windows/Linux x64 工具从官方 Release 下载，版本固定并验证 SHA256 后
执行，不安装到全局、不上传源码。报告保存在忽略提交的
`src-tauri/target/security-audit/`；不要上传原始命中上下文。

工具：cargo-audit 0.22.2、Gitleaks 8.30.1。
[RustSec 工具](https://github.com/rustsec/rustsec/releases/tag/cargo-audit/v0.22.2)与
[Gitleaks 发布](https://github.com/gitleaks/gitleaks/releases/tag/v8.30.1)的 Windows/
Linux 校验值固定于 `scripts/security.ts`。校验和保障字节一致，不等于发布者签名。

## Rust 结果与修复

RustSec 数据库提交 `999ec89d5d3f10f20be14750ec8c2d8b44e0ac9b`，数据库更新时间
2026-10-17，1338 项公告。本次完整复查已成功返回，包含撤回状态查询；没有使用
ignore、no-yanked、no-fetch 或 stale 降级参数。先前的网络失败如实阻止过门禁，
不是通过排除失败查询取得成功。

初次匹配 4 条漏洞记录，对应两个公告在 quick-xml 0.37.5/0.38.4 上的重复匹配：

- [RUSTSEC-2026-0194](https://rustsec.org/advisories/RUSTSEC-2026-0194.html)：属性重复检查的二次时间复杂度。
- [RUSTSEC-2026-0195](https://rustsec.org/advisories/RUSTSEC-2026-0195.html)：命名空间解析内存分配无界。

旧 Windows 链为 notify-rust/直接依赖 → tauri-winrt-notification 0.7.2 →
quick-xml 0.37.5；另有 plist 1.8.0 → quick-xml 0.38.4。已升级 notify-rust 4.18.2、
tauri-winrt-notification 0.8.1 和 plist 1.10.0；旧 XML 版本已移除，保留的版本为
已修复的 0.41.0。没有强行覆盖不兼容的底层 crate 范围。

另更新 anyhow 1.0.104、event-listener 5.4.2、rand 0.8.6/0.9.3，修复对应内存安全
告警；fastrand 2.4.0 已撤回，替换为 2.5.0。复查结果：**0 匹配漏洞，0 已撤回
依赖，进程返回 0，撤回查询无错误**。这不等于所有风险都已消除。

### 残余警告（未标记为修复）

- 10 项 unmaintained：fxhash、proc-macro-error、5 个 unic 系列 crate，以及
  zbus_names、zvariant、zvariant_derive。它们是维护风险，仍需跟进 Tauri/通知链
  上游；没有用忽略配置隐藏。
- RUSTSEC-2024-0429 / glib 0.18.5：属于 Linux GTK 路径，不在 Windows 依赖树。
  它不是“整个项目不受影响”；Linux 仍为 experimental，不能宣称稳定安全支持。
- RUSTSEC-2026-0097 / rand 0.7.3：来自 selectors → phf_codegen → phf_generator
  构建链。实际 feature 树未启用公告要求的 `log` feature；未强行将老父依赖改为
  rand 新主版本。此暴露面判断不等于 crate 已修复，改变 feature/上游后须重审。

Rust 门禁阻止漏洞、已撤回依赖、工具失败、损坏报告及查询不完整；维护/unsound
警告在摘要中完整保留，仍需人工风险评估，而不是被 CI 的绿色状态背书。

## Git 历史与前端

Gitleaks 在完整非浅克隆上使用 `--all --full-history --diff-merges=first-parent`，
覆盖本地 refs 和合并差异，默认规则、递归解码，不使用 baseline/忽略文件，也不
接受行内 allow 注释。日志与报告 100% 脱敏，公开摘要只含 rule/file/line/commit。
初次覆盖 274 个可达提交（工具处理 272 个有差异提交），0 命中；提交本轮改动后
必须再次扫描。它不覆盖未知凭据格式、不可达对象、所有私人数据或未跟踪文件。

前端上轮 32 条告警已降为 1 条 low：Kit 2 → cookie 0.6.0，详见
[开源交付清单](OPEN_SOURCE_CHECKLIST.md#安全审查记录)。本轮不掩盖该项，也不把
原生静态应用没有 Cookie 服务端等同于漏洞修复。公开下一候选前重审，最晚
2026-11-19；若增加 SSR 或服务器功能，立即重评。

## 交付边界

安全检查和原生构建通过不等于安装器验收。Alpha.7 需要准确候选 SHA 的 Windows
quality、Security checks 及独立 NSIS/MSI 安装验收；旧标签不移动。新安全工作流
仅 contents:read，不创建 Issue、不上传扫描报告，也不会自动 Publish。
干净 Windows/WebView2、跨版本/格式迁移、真实 Agent 新事件、编辑器焦点和长时间
时延仍应按[发布门禁](RELEASING.md)验收。
