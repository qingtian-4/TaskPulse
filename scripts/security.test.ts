import { expect, test } from 'bun:test';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { assertDigest, summarizeRust, summarizeSecrets, tools } from './security';

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
