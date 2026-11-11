// TaskPulse unified task model — mirrors src-tauri/src/domain/task.rs

export type Status =
  | 'running'
  | 'waiting_input'
  | 'succeeded'
  | 'failed'
  | 'idle'
  | 'disconnected'
  | 'unknown';

export type Source =
  | 'native_agent'
  | 'doubao'
  | 'codex'
  | 'hermes'
  | 'deepseek_harness'
  | 'remote_tmux'
  | 'local_file'
  | 'claude_code'
  | 'gemini'
  | 'generic_event';
// Brand identity is data, not a growing Rust/TypeScript brand enum.

export type Confidence = 'exact' | 'hook' | 'inferred';

export interface TerminalInfo {
  kind: string;
  focus_id: string;
  outer_id: string;
  label: string;
  window_title?: string;
}

export interface TaskSnapshot {
  agent_id?:string; agent_name?:string; evidence_channel?:string; outcome?:string; pending_confirmed_at?:number; integration_epoch?:string; integration_sequence?:number;
  source_instance?: string;
  model?: string;
  reasoning_effort?: string;
  internal?: boolean;
  project_path?: string;
  tmux_session?: string;
  tmux_pane?: string;
  tmux_target?: string;
  waiting_reason?: 'turn_ended' | 'approval' | 'question' | 'quota_exhausted' | 'unknown';
  input_request_id?: string;
  input_request_ids?: string[];
  attention_since?: number;
  id: string;
  name: string;
  source: Source;
  status: Status;
  host?: string;
  project?: string;
  message?: string;
  activity?: string;
  phase?: 'thinking' | 'tool' | 'responding' | 'awaiting_model' | 'awaiting_tool';
  started_at?: number; // epoch ms
  updated_at: number; // epoch ms
  finished_at?: number; // epoch ms
  pid?: number;
  exit_code?: number;
  confidence: Confidence;
  remote: boolean;
  stale?: boolean;
  recent?: boolean;
  unread?: boolean;
  pinned?: boolean;
  turn_key?: string;
  native_unread?: boolean;
  terminal?: TerminalInfo | null;
  can_focus?: boolean;
  cpu?: number;
  cli?: string;
  session_id?: string;
  quota_5h_used_pct?: number;
  quota_5h_resets_at?: number;
  quota_1w_used_pct?: number;
  quota_1w_resets_at?: number;
  hook_event?: string;
  hook_matcher?: string;
}

export type SourceKind = 'kimi_code' | 'qoder' | 'trae' | 'opencode' | 'workbuddy' | 'zcode' | 'doubao' | 'codex' | 'codex_wsl' | 'claude_code' | 'hermes' | 'deepseek_desktop' | 'deepseek_wsl' | 'remote_codex' | 'remote_claude_code' | 'tmux' | 'hooks' | 'scanner' | 'generic_event';
export interface MonitorSource {
  id: string; name: string; kind: SourceKind; enabled: boolean; location: string;
  projects: string[]; excluded_projects: string[]; tmux_sessions: string[]; tmux_panes: string[]; poll_interval_ms: number;
  integration?:{agent_id:string;adapter_id:string;transport:string;host_version?:string;accept_since_ms:number};
}
export interface DashboardSettings {
  preferences?: import('./preferences').Preferences;
  version: number; onboarding_complete: boolean; sources: MonitorSource[];
  auto_discover_local?: boolean; auto_discovery_seen?: string[]; auto_check_updates?: boolean;
  ssh_paused_sources?: string[];
  notifications: boolean; sound: boolean; default_view: 'tasks' | 'projects';
  recent_days?: number; // 0 keeps all ages; legacy configs default to 7.
  notification_mode?: 'popup' | 'system' | 'both';
}
export interface TaskNotice {
  id: number; title: string; body: string; created_at: number; task_id: string | null; test: boolean;
  context?: { task_name: string; can_focus_host?: boolean; source: Source; agent_name?:string; project: string | null; host: string | null } | null;
  auto_dismiss_ms?: number | null;
}
export interface NoticeState { current: TaskNotice | null; pending: number }
export interface DeliveryRecord { at:number; channel:string; stage:string; notice_id:number | null; task_id:string | null; request_id:string | null; title:string; test:boolean }
export type MessageKind = 'approval' | 'question' | 'quota' | 'completed' | 'failed' | 'interrupted';
export interface MessageEntry {
  id:string; at:number; seen:boolean; kind:MessageKind;
  task_id:string; task_name:string; source:Source; agent_name?:string; project?:string;
  state:'pending'|'unconfirmed'|'resolved'|'result'; can_reopen:boolean;
  request_id?:string; turn_key?:string;
}
export interface MessageCenterState { revision:number; unread:number; entries:MessageEntry[]; error:string|null }
export interface SourceDiscovery { id: string; location: string; data_location_detected: boolean | null; detail: string; candidate?: MonitorSource }
export interface SessionPage { items: Array<{ task: TaskSnapshot; reason: string }>; total: number }

export interface HostHealth {
  host: string;
  status: 'connected' | 'disconnected';
  last_seen?: number;
}

export const SOURCE_LABEL: Record<Source, string> = {
  'native_agent':     '其他 Agent',
  'doubao':           '豆包',
  'generic_event':    '其他 Agent',
  'codex':            'Codex',
  'hermes':           'Hermes',
  'deepseek_harness': 'DeepSeek',
  'remote_tmux':      'Tmux',
  'local_file':       'Local',
  'claude_code':      'Claude',
  'gemini':           'Gemini',
};

export const CONFIDENCE_LABEL: Record<Confidence, string> = {
  'exact':    '精确',
  'hook':     'Hook',
  'inferred': '推断',
};

/** Human duration from an epoch-ms timestamp until now (or until `until`). */
export function formatElapsed(startMs: number | undefined, untilMs: number = Date.now()): string {
  if (!startMs) return '';
  const secs = Math.max(0, Math.floor((untilMs - startMs) / 1000));
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m ${secs % 60}s`;
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  return `${h}h ${m}m`;
}

/** "7m ago" from an epoch-ms timestamp. */
export function formatAgo(ms: number | undefined, nowMs: number = Date.now()): string {
  if (!ms) return '';
  const secs = Math.max(0, Math.floor((nowMs - ms) / 1000));
  if (secs < 60) return `${secs} 秒前`;
  if (secs < 3600) return `${Math.floor(secs / 60)} 分钟前`;
  if (secs < 86400) return `${Math.floor(secs / 3600)} 小时前`;
  return `${Math.floor(secs / 86400)} 天前`;
}

export interface ProviderHealth {
  provider: string;
  status: 'connected' | 'paginating' | 'degraded';
  last_success?: number;
  last_attempt?: number;
  task_observations?: Record<string,{observed_at:number;state:string}>;
  message?: string;
}
export interface CodexAccountQuota {
  quota: {
    used_5h_pct: number;
    resets_5h_at: number | null;
    used_1w_pct: number;
    resets_1w_at: number | null;
  } | null;
  observed_at: number | null;
  error: string | null;
  stale: boolean;
  refreshing: boolean;
}

export interface UpdateStatus {phase:string;current_version:string;version:string|null;notes:string|null;checked_at:number|null;downloaded:number;total:number|null;message:string|null;can_install:boolean}

export type SourceReadState = 'disabled' | 'waiting' | 'healthy' | 'paginating' | 'partial' | 'failed' | 'stale';
export interface SourceSnapshot {
  id: string; kind: SourceKind; location: string; enabled: boolean; read_state: SourceReadState; data_detected: boolean | null;
  discovered_sessions: number; current_tasks: number;
}
export interface DashboardView {
  counts: {ongoing:number;action:number;results:number;failed:number;uncertain:number};
  sources: SourceSnapshot[];
  unconfirmed_task_ids: string[];
}
export interface DashboardSnapshot {
  revision:number;tasks:TaskSnapshot[];hosts:HostHealth[];providers:ProviderHealth[];view:DashboardView;
}
