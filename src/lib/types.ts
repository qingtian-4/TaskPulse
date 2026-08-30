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
  | 'codex'
  | 'hermes'
  | 'deepseek_harness'
  | 'remote_tmux'
  | 'local_file'
  | 'claude_code'
  | 'gemini';

export type Confidence = 'exact' | 'hook' | 'inferred';

export interface TerminalInfo {
  kind: string;
  focus_id: string;
  outer_id: string;
  label: string;
  window_title?: string;
}

export interface TaskSnapshot {
  id: string;
  name: string;
  source: Source;
  status: Status;
  host?: string;
  project?: string;
  message?: string;
  started_at?: number; // epoch ms
  updated_at: number; // epoch ms
  finished_at?: number; // epoch ms
  pid?: number;
  exit_code?: number;
  confidence: Confidence;
  remote: boolean;
  stale?: boolean;
  recent?: boolean;
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

export interface HostHealth {
  host: string;
  status: 'connected' | 'disconnected';
  last_seen?: number;
}

// Attention priority: FAILED > WAITING_INPUT > DISCONNECTED > RUNNING > SUCCEEDED/IDLE > UNKNOWN
export const STATUS_PRIORITY: Record<Status, number> = {
  'failed':         0,
  'waiting_input':  1,
  'disconnected':   2,
  'running':        3,
  'succeeded':      4,
  'idle':           4,
  'unknown':        5,
};

export const STATUS_LABEL: Record<Status, string> = {
  'failed':         '失败',
  'waiting_input':  '等待输入',
  'disconnected':   '已断开',
  'running':        '运行中',
  'succeeded':      '已完成',
  'idle':           '空闲',
  'unknown':        '未知',
};

export const STATUS_COLOR: Record<Status, string> = {
  'failed':         '#dd4f4f',
  'waiting_input':  '#e8c840',
  'disconnected':   '#d46a6a',
  'running':        '#4a90d9',
  'succeeded':      '#78b644',
  'idle':           '#78b644',
  'unknown':        '#555555',
};

export const SOURCE_LABEL: Record<Source, string> = {
  'codex':            'Codex',
  'hermes':           'Hermes',
  'deepseek_harness': 'DeepSeek',
  'remote_tmux':      'Tmux',
  'local_file':       'Local',
  'claude_code':      'Claude',
  'gemini':           'Gemini',
};

export const SOURCE_COLOR: Record<Source, string> = {
  'codex':            '#6fbe5a',
  'hermes':           '#d97757',
  'deepseek_harness': '#4a90d9',
  'remote_tmux':      '#8a7fd9',
  'local_file':       '#888888',
  'claude_code':      '#d97757',
  'gemini':           '#4a90d9',
};

export const CONFIDENCE_LABEL: Record<Confidence, string> = {
  'exact':    '精确',
  'hook':     'Hook',
  'inferred': '推断',
};

/** Maps terminal kind → short display name. null = use terminal.label instead. */
const TERMINAL_KIND_DISPLAY: Record<string, string | null> = {
  tmux:           'tmux',
  screen:         'screen',
  zellij:         'zellij',
  neovim:         'nvim',
  vscode:         'VS Code',
  jetbrains:      'JetBrains',
  kitty:          'kitty',
  x11_generic:    null,
  macos_app:      null,
  windows_native: null,
  unknown:        null,
};

export function formatTerminalChip(terminal: TerminalInfo | null): string | null {
  if (!terminal) return null;
  const kindDisplay = TERMINAL_KIND_DISPLAY[terminal.kind] ?? null;
  const label = terminal.label || null;

  if (!kindDisplay && !label) return null;
  if (!kindDisplay) return label;
  if (!label || kindDisplay.toLowerCase() === label.toLowerCase()) return kindDisplay;
  return `${kindDisplay} · ${label}`;
}

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
  status: 'connected' | 'degraded';
  last_success?: number;
  message?: string;
}
