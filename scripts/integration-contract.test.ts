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
