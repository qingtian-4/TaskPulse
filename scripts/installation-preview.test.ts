import {expect, test} from 'bun:test';
import {readFileSync} from 'node:fs';
const read=(name:string)=>readFileSync(new URL('../'+name,import.meta.url),'utf8');
const workflow=Bun.YAML.parse(read('.github/workflows/installation-preview.yml')) as any;

test('installed upgrade acceptance cannot publish or consume production signing secrets',()=>{
  const text=read('.github/workflows/upgrade-acceptance.yml');
  const upgrade=Bun.YAML.parse(text) as any;
  expect(upgrade.on).not.toHaveProperty('pull_request_target');
  expect(upgrade.permissions).toEqual({contents:'read'});
  expect(text).not.toMatch(/secrets\.|contents: write|gh release|git push|git tag|action-gh-release/);
  expect(upgrade.jobs.security.if).toContain('head.repo.full_name == github.repository');
  expect(upgrade.jobs.upgrade.needs).toBe('security');
  expect(upgrade.jobs.upgrade.strategy.matrix.include).toHaveLength(4);
  for(const step of upgrade.jobs.upgrade.steps){
    expect(step['continue-on-error']).not.toBe(true);
    if(step.uses)expect(step.uses).toMatch(/@[a-f0-9]{40}$/);
    if(step.uses?.startsWith('actions/checkout')){
      expect(step.with.ref).toBe('${{ github.event.pull_request.head.sha }}');
      expect(step.with['persist-credentials']).toBe(false);
    }
    if(step.uses?.startsWith('actions/upload-artifact')){
      expect(step.with.path).toBe('dist/upgrade-evidence/');
      expect(step.with.name).toContain('github.event.pull_request.head.sha');
    }
  }
});

test('preview runs for same-repository PR updates, with immutable head and read-only permissions',()=>{
  expect(workflow.on.pull_request.types).toContain('synchronize');
  expect(workflow.on).not.toHaveProperty('pull_request_target');
  expect(workflow.permissions).toEqual({contents:'read'});
  expect(workflow.jobs.prepare.if).toContain('head.repo.full_name == github.repository');
  const prepare=workflow.jobs.prepare.steps;
  expect(prepare.find((s:any)=>s.uses?.startsWith('actions/checkout')).with.ref).toBe('${{ github.event.pull_request.head.sha }}');
  const pin=prepare.find((s:any)=>s.id==='source');
  expect(pin.env.PR_HEAD_SHA).toBe('${{ github.event.pull_request.head.sha }}');
  expect(pin.run).toContain("head != os.environ['PR_HEAD_SHA']");
  expect(pin.run).toContain("re.fullmatch('[a-f0-9]{40}'");
  for(const [name,job] of Object.entries(workflow.jobs) as [string,any][]){
    expect(job.permissions?.contents??'read',name).toBe('read');
    expect(job).not.toHaveProperty('secrets');
    expect(job['continue-on-error'],name).not.toBe(true);
    for(const step of job.steps??[]){
      expect(step['continue-on-error'],name).not.toBe(true);
      if(step.uses?.startsWith('actions/checkout')){
        expect(step.with['persist-credentials'],name).toBe(false);
        if(name!=='prepare')expect(step.with.ref).toBe('${{ needs.prepare.outputs.commit }}');
      }
      if(step.uses?.startsWith('actions/upload-artifact'))expect(step.with.name).toContain('needs.prepare.outputs.commit');
      if(step.uses)expect(step.uses).toMatch(/@[a-f0-9]{40}$/);
    }
  }
  const body=read('.github/workflows/installation-preview.yml');
  expect(body).not.toMatch(/contents: write|gh release|git push|git tag|action-gh-release|secrets\./);
});

test('preview builds every package and keeps fresh installer runner and security prerequisites',()=>{
  expect(workflow.jobs.security.with.ref).toBe('${{ needs.prepare.outputs.commit }}');
  for(const job of ['windows','macos'])expect(workflow.jobs[job].needs).toContain('security');
  expect(workflow.jobs.windows.steps.some((s:any)=>s.with?.args?.includes('--bundles nsis,msi'))).toBe(true);
  expect(workflow.jobs.windows.steps.some((s:any)=>s.run?.includes('--untagged --pr-preview'))).toBe(true);
  const smoke=workflow.jobs['installer-smoke'];
  expect(smoke.strategy.matrix).toEqual({installer:['nsis','msi'],round:[1,2]});
  expect(smoke.strategy['fail-fast']).toBe(false);
  expect(smoke.steps.some((s:any)=>s.run?.includes('windows-cli-smoke.ps1'))).toBe(true);
  expect(smoke.steps.some((s:any)=>s.run?.includes('installer-smoke.ps1 -Assets'))).toBe(true);
  expect(smoke.steps.some((s:any)=>s.run?.includes('gerardog.gsudo'))).toBe(true);
  expect(workflow.jobs.macos.strategy.matrix.include).toEqual([
    {os:'macos-15',platform:'macos-arm64',target:'aarch64-apple-darwin',arch:'arm64',bundles:'app,dmg'},
    {os:'macos-15-intel',platform:'macos-x64',target:'x86_64-apple-darwin',arch:'x64',bundles:'app,dmg'},
  ]);
  expect(workflow.jobs.macos.steps.some((s:any)=>s.run?.includes('--pr-preview'))).toBe(true);
  expect(workflow.jobs.macos.steps.some((s:any)=>s.run?.includes('macos-cli-smoke.sh'))).toBe(true);
  const macArtifact=workflow.jobs.macos.steps.find((s:any)=>s.uses?.startsWith('actions/upload-artifact'));
  expect(macArtifact.if).toBe("always() && steps.stage.outcome == 'success'");
  const original=Bun.YAML.parse(read('.github/workflows/release.yml')) as any;
  expect(original.jobs.draft.if).toBe("${{ needs.preflight.outputs.tag != '' }}");
  expect(original.jobs.preflight.steps.find((s:any)=>s.id==='version').run).toContain('candidate --commit "$CANDIDATE_COMMIT"');
  expect(original.jobs['installer-smoke'].strategy.matrix).toEqual({installer:['nsis','msi'],round:[1,2]});
});

test('controlled download probes call production validators and keep transport overrides test-only',()=>{
  const win=read('scripts/windows-cli-smoke.ps1');
  for(const value of ['..\\install.ps1','Invoke-TaskPulseInstall','Test-TaskPulseDownload','pr-preview',"$env:GITHUB_ACTIONS -ne 'true'", "-Version $Version"])
    expect(win).toContain(value);
  const mac=read('scripts/macos-cli-smoke.py');
  for(const value of ["script = repo / 'install.sh'", "'/bin/bash', str(script)", 'preview-rejected', 'first-install','replacement-rollback-simulated','download_quarantine_verified'])expect(mac).toContain(value);
  const production=read('install.sh');
  for(const value of ['TASKPULSE_TEST_RESPONSE','TASKPULSE_TEST_API','--skip','eval ','xattr -d','sudo ','pkill '])expect(production).not.toContain(value);
  expect(production).toContain('xattr -w com.apple.quarantine');
  expect(read('install.ps1')).not.toContain('TASKPULSE_TEST');
});


test('public installation acceptance is read-only, stable-only and tests anonymous production bootstraps',()=>{
  const text=read('.github/workflows/public-install-smoke.yml');
  const publicFlow=Bun.YAML.parse(text) as any;
  expect(publicFlow.permissions).toEqual({contents:'read'});
  expect(publicFlow.on.release.types).toEqual(['published']);
  expect(publicFlow.on).not.toHaveProperty('pull_request_target');
  expect(publicFlow.jobs.install.strategy.matrix.include).toHaveLength(3);
  expect(publicFlow.jobs.install.env.RELEASE_TAG).toContain('inputs.tag');
  expect(text).not.toMatch(/secrets\.|contents: write|gh release|git push|git tag/);
  for(const step of publicFlow.jobs.install.steps){
    if(step.uses)expect(step.uses).toMatch(/@[a-f0-9]{40}$/);
    expect(step['continue-on-error']).not.toBe(true);
    if(step.run)expect(step.run).not.toContain('${{');
  }
  const probe=read('scripts/public-install-smoke.py');
  expect(probe).toContain('/releases/latest');
  expect(probe).toContain('-DownloadOnly');
  expect(probe).toContain('installer-smoke.ps1');
  expect(probe).not.toMatch(/TASKPULSE_TEST_|Authorization|xattr.*-d/);
});
