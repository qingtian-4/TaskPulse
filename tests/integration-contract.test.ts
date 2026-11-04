import { test, expect } from 'bun:test';
import { readdir } from 'node:fs/promises';
test('bundled integration contracts never advertise native validation or final Stop',async()=>{
  for(const path of (await readdir('integrations')).filter(p=>p.endsWith('.json')&&!p.endsWith('.schema.json'))){
    const m=await Bun.file('integrations/'+path).json();
    expect(m.manifest_version).toBe(1);
    expect(m.adapter_id).toMatch(/^[a-z0-9_-]+$/);
    expect(m.events.Stop).not.toBe('turn.completed');
    expect(m.events.UserPromptSubmit).not.toBe('turn.started');
    expect(m.command).toBeUndefined();expect(m.install_path).toBeUndefined();
    for(const capability of Object.values(m.capabilities) as any[]){expect(capability.validation).not.toBe('native-tested');expect(capability.tested_versions).toEqual([]);}
  }
});
test('protocol examples and independent versioned schema share the producer boundary',async()=>{
  const schema=await Bun.file('integrations/event-v1.schema.json').json();
  expect(schema.additionalProperties).toBe(false);expect(schema.properties.schema_version.const).toBe(1);
  for(const name of ['start','approval','complete']){
    const event=await Bun.file(`integrations/examples/${name}.json`).json();
    expect(schema.properties.kind.enum).toContain(event.kind);expect(event.schema_version).toBe(1);
    expect(event.source_instance_id).toBeUndefined();expect(event.evidence).toBeUndefined();
  }
});
test('new checks are included in local verification and Windows CI',async()=>{
  const verify=await Bun.file('scripts/verify.ps1').text();const quality=await Bun.file('.github/workflows/quality.yml').text();
  expect(verify).toContain('test:integration-contract');expect(verify).toContain('integration-bridge-smoke.ts');
  expect(quality).toContain('test:integration-contract');expect(quality).toContain('integration-bridge-smoke.ts');
});

test('each registered app command has an explicit local capability', async () => {
  const main = await Bun.file('src-tauri/src/main.rs').text();
  const handler = main.split('.invoke_handler(tauri::generate_handler![')[1].split('])')[0];
  const registered = handler.trim().split(/\s*,\s*/).map(path => path.split('::').at(-1));
  const { permission } = Bun.TOML.parse(await Bun.file('src-tauri/permissions/app-commands.toml').text()) as {
    permission: {identifier: string; commands: {allow: string[]}}[];
  };
  const rules = new Map(permission.map(p => [p.identifier, p.commands.allow]));
  expect(registered.sort()).toEqual([...rules.values()].flat().sort());
  expect(registered).not.toContain('install_hooks');
  expect(registered).not.toContain('focus_terminal');

  const allowed = new Map<string, Set<string>>();
  for (const file of await readdir('src-tauri/capabilities')) {
    const capability = await Bun.file('src-tauri/capabilities/' + file).json();
    for (const id of capability.permissions) {
      if (!rules.has(id)) continue;
      expect(capability.remote).toBeUndefined();
      expect(capability.local).not.toBe(false);
      expect(capability.windows).toHaveLength(1);
      const [window] = capability.windows;
      expect(['popup', 'management', 'task-notice']).toContain(window);
      const commands = allowed.get(window) ?? new Set<string>();
      for (const command of rules.get(id)!) commands.add(command);
      allowed.set(window, commands);
    }
  }
  for (const command of [
    'save_monitoring_settings', 'save_preferences', 'set_autostart',
    'install_application_update', 'configure_claude_events',
    'integration_import', 'integration_preview', 'integration_apply', 'integration_test',
  ]) {
    expect([...allowed].filter(([, commands]) => commands.has(command)).map(([window]) => window)).toEqual(['management']);
  }
  expect([...allowed.get('task-notice')!].sort()).toEqual([
    'ack_task_notice', 'dismiss_task_notice', 'get_monitoring_settings',
    'get_task_notice', 'next_task_notice', 'open_task_notice',
  ]);
});
