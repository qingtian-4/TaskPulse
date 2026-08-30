import { STATUS_PRIORITY, type HostHealth, type Status, type TaskSnapshot } from './types';

export function aggregate(tasks: TaskSnapshot[], hosts: HostHealth[] = []): Status {
  const statuses: Status[] = [
    ...tasks.map((task) => task.status),
    ...hosts.filter((host) => host.status === 'disconnected').map(() => 'disconnected' as const),
  ];
  if (statuses.length === 0) return 'unknown';
  return statuses.reduce<Status>(
    (best, status) => (STATUS_PRIORITY[status] < STATUS_PRIORITY[best] ? status : best),
    'unknown'
  );
}

/** Active tasks (not finished / not RECENT). */
export function activeTasks(tasks: TaskSnapshot[]): TaskSnapshot[] {
  // Backend already returns the dynamic attention list. Terminal/unread tasks
  // remain first-class rows until viewed, so the frontend must not hide them.
  return tasks;
}

/** RECENT completed tasks. */
export function recentTasks(tasks: TaskSnapshot[]): TaskSnapshot[] {
  return tasks
    .filter((t) => t.recent || t.status === 'succeeded' || t.status === 'failed')
    .sort((a, b) => (b.finished_at ?? 0) - (a.finished_at ?? 0));
}

/** Local (non-remote) active tasks. */
export function localTasks(tasks: TaskSnapshot[]): TaskSnapshot[] {
  return tasks.filter((t) => !t.remote);
}

/** Remote active tasks grouped by host (insertion order = priority order). */
export function remoteByHost(tasks: TaskSnapshot[]): Map<string, TaskSnapshot[]> {
  const map = new Map<string, TaskSnapshot[]>();
  for (const t of tasks.filter((t) => t.remote)) {
    const host = t.host ?? 'unknown';
    const list = map.get(host) ?? [];
    list.push(t);
    map.set(host, list);
  }
  return map;
}
