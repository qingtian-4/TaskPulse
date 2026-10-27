import type { DashboardSettings, MonitorSource, ProviderHealth, SourceKind } from './types';

// Readability is a transport observation, never proof of Agent lifecycle support.
export function sourceReadState(source:MonitorSource, applied:DashboardSettings|null, health:ProviderHealth[], now:number):string {
  const saved=applied?.sources.find(s=>s.id===source.id);
  if(!saved || !applied?.onboarding_complete)return '未保存 · 尚未监控';
  if(!saved.enabled)return '已保存 · 已停用';
  const h=health.find(h=>h.provider===source.id);
  if(!h?.last_attempt)return '已保存 · 等待首次读取';
  if(h.status==='degraded')return '读取异常 · 需检查';
  if(!h.last_success || now-h.last_success>Math.max(30000,saved.poll_interval_ms*3))return '读取结果待更新';
  return '来源读取正常';
}

export function sourceReadTone(source:MonitorSource, applied:DashboardSettings|null, health:ProviderHealth[], now:number):'neutral'|'stale'|'success' {
  const saved=applied?.sources.find(s=>s.id===source.id);
  if(!saved || !applied?.onboarding_complete || !saved.enabled)return 'neutral';
  const h=health.find(h=>h.provider===source.id);
  return h?.status==='connected' && !!h.last_attempt && !!h.last_success && now-h.last_success<=Math.max(30000,saved.poll_interval_ms*3)?'success':'stale';
}

// A visual hint only. Reaching step 4 is NOT host lifecycle certification.
export function sourceSetupStep(hasSource:boolean,draft:boolean,readTestPassed:boolean):number {
  return !hasSource?1:draft?2:readTestPassed?4:3;
}

export function readTestResult(id:string, health:ProviderHealth[], since:number):'pending'|'passed'|'failed' {
  const h=health.find(h=>h.provider===id);
  if(!h?.last_attempt || h.last_attempt<since)return 'pending';
  return h.status==='connected' && (h.last_success??0)>=since?'passed':'failed';
}

const notes:Record<SourceKind,string>={
  doubao:'实验读取豆包原生执行检查点；已验证 Windows 2.26.10。检查点移除只标为待更新，不推断完成；审批提醒尚未适配。',
  codex:'原生标题、项目与 rollout 执行阶段；仅明确结束事件提醒。命令审批需可选的官方观察 Hook，安装后还需在 Codex /hooks 中信任。',
  codex_wsl:'独立读取已运行发行版的 Codex 数据；不唤醒 WSL，不借用 Windows 额度。命令审批需在该发行版安装并信任观察 Hook。',
  remote_codex:'读取已配置 SSH 主机的 Codex 会话；依赖主机可达、权限与远端数据格式。',
  claude_code:'原生会话读取为尽力推断；精确生命周期需自愿配置官方 Hook，与代理模型无关。',
  hermes:'原生标题、项目和运行租约；没有明确结束证据时不猜测任务完成。',
  deepseek_desktop:'读取原生 Desktop / CLI 存储，支持旧版与 v4 日志；状态能力取决于宿主记录。',
  deepseek_wsl:'独立读取已启用 WSL 安装，不与 Windows 桌面会话混合。',
  tmux:'普通 shell 返回仅为推断结束，退出码未知；前台 tp-run 可报告直接子进程退出。',
  hooks:'读取兼容状态文件；状态准确性由写入方决定，不自动安装宿主 Hook。',
  scanner:'只提供受限进程证据；不能用 CPU、静默日志或进程消失证明本轮结束。',
  generic_event:'按适配器逐项查看能力。测试事件接收成功，不等于真实宿主生命周期已验证。',
};
export function sourceCapability(kind:SourceKind):string{return notes[kind];}
