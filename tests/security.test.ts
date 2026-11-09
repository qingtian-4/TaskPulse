import { expect, test } from 'bun:test';
import { createHash } from 'node:crypto';
import { readFileSync,mkdtempSync,writeFileSync,existsSync,rmSync,renameSync,mkdirSync } from 'node:fs';
import { join } from 'node:path';
import {tmpdir} from 'node:os';
import { withToolDirectory,assertDigest,downloadVerifiedTool, summarizeRust, summarizeSecrets, tools } from '../scripts/security';

const cleanRust = { database: { 'last-commit': '1'.repeat(40), 'last-updated': '2026-10-19T00:00:00Z', 'advisory-count': 0 }, vulnerabilities: { found: false, count: 0, list: [] }, warnings: {} };
test('tool bytes must match a pinned digest before execution', () => {
  const bytes = Buffer.from('isolated checksum fixture');
  expect(() => assertDigest(bytes, createHash('sha256').update(bytes).digest('hex'))).not.toThrow();
  expect(() => assertDigest(bytes, '0'.repeat(64))).toThrow();
  for (const tool of Object.values(tools)) {
    expect(tool.release.startsWith('https://github.com/')).toBe(true);
    for (const platform of ['win32', 'linux'] as const) expect(tool[platform].sha256).toMatch(/^[a-f0-9]{64}$/);
  }
});
test('tool downloads retry transient errors and always verify the successful response', async () => {
  const fixture = 'verified archive';
  const digest = createHash('sha256').update(fixture).digest('hex');
  const url = 'https://github.com/example/tool/releases/download/v1/archive.zip';
  let attempts = 0;
  const result = await downloadVerifiedTool(url, digest, async () => {
    attempts++;
    if (attempts === 1) return new Response(null, {status: 504});
    if (attempts === 2) throw Error('transport interrupted');
    return new Response(fixture);
  }, async () => {});
  expect(new TextDecoder().decode(result)).toBe(fixture);
  expect(attempts).toBe(3);

  for (const [status, body, failure, expectedAttempts] of [
    [403, '', 'HTTP 403', 1],
    [504, '', 'HTTP 504', 3],
    [200, 'unverified archive', 'checksum mismatch', 1],
  ] as const) {
    attempts = 0;
    await expect(downloadVerifiedTool(url, digest, async () => {
      attempts++;
      return new Response(body, {status});
    }, async () => {})).rejects.toThrow(failure);
    expect(attempts).toBe(expectedAttempts);
  }
});

test('Rust reports cannot hide a failed process, registry error or inconsistent counts', () => {
  expect(summarizeRust(JSON.stringify(cleanRust), 0).passed).toBe(true);
  for (const status of [null, 1, 2]) expect(summarizeRust(JSON.stringify(cleanRust), status).passed).toBe(false);
  const incomplete = summarizeRust(JSON.stringify(cleanRust), 0, "error: couldn't check if the package is yanked");
  expect(incomplete.passed).toBe(false);
  expect(incomplete.registryCheckIncomplete).toBe(true);
  expect(incomplete.diagnosticError).toBe(true);
  expect(summarizeRust(JSON.stringify(cleanRust), 0, "couldn't check if the package is yanked").passed).toBe(false);
  expect(summarizeRust(JSON.stringify(cleanRust), 0).registryCheckIncomplete).toBe(false);
  expect(() => summarizeRust('{}', 0)).toThrow();
  expect(() => summarizeRust(JSON.stringify({ vulnerabilities: { found: false, count: 1, list: [] } }), 0)).toThrow();
  expect(() => summarizeRust(JSON.stringify({ ...cleanRust, database: null }), 0)).toThrow();
});
test('Rust vulnerabilities and yanked packages block; maintenance/unsound warnings remain visible', () => {
  const report = { ...cleanRust, warnings: { unsound: [{ advisory: { id: 'RUSTSEC-test' }, package: { name: 'fixture', version: '1' } }] } };
  expect(summarizeRust(JSON.stringify(report), 0).warnings.unsound.length).toBe(1);
  expect(summarizeRust(JSON.stringify({ ...report, warnings: { yanked: [{}] } }), 0).passed).toBe(false);
  expect(summarizeRust(JSON.stringify({ ...report, vulnerabilities: { found: true, count: 1, list: [{}] } }), 0).passed).toBe(false);
});
test('empty secret reports mean zero findings, not one empty record; failures still block', () => {
  expect(summarizeSecrets('[]', 0)).toEqual({ passed: true, findings: [] });
  expect(summarizeSecrets('[]', 1).passed).toBe(false);
  expect(summarizeSecrets('[]', null).passed).toBe(false);
  for (const raw of ['null', '{}', 'not JSON']) expect(() => summarizeSecrets(raw, 0)).toThrow();
});
test('secret summaries never copy raw matches, secrets, author/email or arbitrary metadata', () => {
  const summary = summarizeSecrets(JSON.stringify([{ RuleID: 'fixture', File: 'example.txt', StartLine: 2, Commit: 'abc', Match: 'private context', Secret: 'not-a-real-secret', Author: 'private name', Email: 'private email' }]), 1);
  expect(summary.passed).toBe(false);
  expect(summary.findings).toEqual([{ rule: 'fixture', file: 'example.txt', line: 2, commit: 'abc' }]);
  expect(JSON.stringify(summary)).not.toContain('private');
});
test('releases gate on reusable read-only security jobs and secrets require full history', () => {
  const root = join(import.meta.dir, '..');
  const workflow = (file: string): any => Bun.YAML.parse(readFileSync(join(root, '.github/workflows', file), 'utf8'));
  const security = workflow('security.yml');
  expect(security.permissions.contents).toBe('read');
  expect(security.jobs.secrets.steps[0].with['fetch-depth']).toBe(0);
  for (const job of Object.values(security.jobs) as any[]) {
    expect(job.steps[0].with['persist-credentials']).toBe(false);
    expect(job.steps[0].with.ref).toContain('inputs.ref');
    expect(job['continue-on-error']).not.toBe(true);
    expect(job.steps.some((step: any) => step.uses?.includes('upload-artifact'))).toBe(false);
  }
  const release = workflow('release.yml');
  expect(release.jobs.security.uses).toBe('./.github/workflows/security.yml');
  expect(release.jobs.security.with.ref).toContain('preflight.outputs.commit');
  expect(release.jobs.windows.needs).toContain('security');
});


test('security extraction is removed on success/failure without deleting siblings or replacements',async()=>{
  const cache=mkdtempSync(join(tmpdir(),'taskpulse-tool-cache-'));
  try{
    const sibling=join(cache,'verified.zip');writeFileSync(sibling,'retained');
    let owned='';
    expect(await withToolDirectory(cache,folder=>{owned=folder;writeFileSync(join(folder,'tool.exe'),'fixture');return 7;})).toBe(7);
    expect(existsSync(owned)).toBe(false);expect(existsSync(sibling)).toBe(true);
    await expect(withToolDirectory(cache,folder=>{owned=folder;throw Error('probe failed');})).rejects.toThrow('probe failed');
    expect(existsSync(owned)).toBe(false);
    await expect(withToolDirectory(cache,folder=>{
      renameSync(folder,folder+'-original');mkdirSync(folder);writeFileSync(join(folder,'foreign.txt'),'keep');
      return 0;
    })).rejects.toThrow('refusing cleanup');
    expect(existsSync(sibling)).toBe(true);
  }finally{rmSync(cache,{recursive:true,force:true});}
});
