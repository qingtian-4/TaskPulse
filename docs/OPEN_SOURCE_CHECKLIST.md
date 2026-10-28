# 公开发布检查清单

每次按准确的候选源码 SHA 检查。完成记录和原始证据保存在本地或对应 CI artifacts，不把上一次结果当成本次验收。

- [ ] 当前 head 的 Windows quality / Security 通过，适用平台的原生检查完成。
- [ ] 完整前端审计、Rust 漏洞/撤回检查和完整 Git 历史密钥扫描完成；低级别、维护和 unsound 告警已审阅。
- [ ] 版本、不可变标签、源 SHA、manifest 和安装包内容一致。
- [ ] NSIS / MSI 各自在独立全新 runner 通过，Mac 架构分别构建与验收。
- [ ] 干净 Windows 10/11、WebView2 缺失/网络受限、跨版本升级和格式迁移已实测。
- [ ] Mac 下载后的 Gatekeeper、Finder 启动、菜单栏、提醒焦点和真实 Agent 用例已实测。
- [ ] 已核对未读、固定、来源范围和通知去重在升级/重启后保留。
- [ ] README 的平台、下载、CLI 和能力说明与实际产物一致。
- [ ] SBOM、依赖许可证文本和原生组件分发义务已审阅；自动生成清单不等于审查通过。
- [ ] 当前源码不追踪凭据、私有配置、过程报告、原始截图或编译产物。
- [ ] 签名、公证和已知限制如实说明；未签名不能写为已验证发布者。
- [ ] Draft pre-release 经维护者明确确认后再公开，未验证或暂缓的项目不能勾选通过。

详见[发布流程](RELEASING.md)、[候选测试](CANDIDATE_TESTING.md)、[安全反馈](../SECURITY.md)和[公开文件范围](README.md#what-belongs-here)。
