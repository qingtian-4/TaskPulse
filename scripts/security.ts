/** Read-only, pinned security tools. Downloads/reports stay in ignored target/.
 * No global installation, credential upload, baselines or ignored advisories.
 */
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';

type ToolName = 'cargo-audit' | 'gitleaks';
export const tools = {
  'cargo-audit': {
    version: '0.22.2',
    win32: { asset: 'cargo-audit-x86_64-pc-windows-msvc-v0.22.2.zip', sha256: '0a7316540862c13d954f648917ceacca593747baed6eec180fafa590be2710ab' },
    linux: { asset: 'cargo-audit-x86_64-unknown-linux-gnu-v0.22.2.tgz', sha256: 'ab28a1bdb54db4d5d8ad5981cf1f959410370b3d28250dbd35f6a44248620e39' },
    release: 'https://github.com/rustsec/rustsec/releases/download/cargo-audit%2Fv0.22.2/',
  },
  gitleaks: {
    version: '8.30.1',
    win32: { asset: 'gitleaks_8.30.1_windows_x64.zip', sha256: 'd29144deff3a68aa93ced33dddf84b7fdc26070add4aa0f4513094c8332afc4e' },
    linux: { asset: 'gitleaks_8.30.1_linux_x64.tar.gz', sha256: '551f6fc83ea457d62a0d98237cbad105af8d557003051f41f3e7ca7b3f2470eb' },
    release: 'https://github.com/gitleaks/gitleaks/releases/download/v8.30.1/',
  },
} as const;

export function assertDigest(bytes: Uint8Array, expected: string) {
  if (!/^[a-f0-9]{64}$/.test(expected) || createHash('sha256').update(bytes).digest('hex') !== expected) {
    throw Error('Security tool checksum mismatch; refusing execution');
  }
}

function parseJson(raw: string): any {
  return JSON.parse(raw.replace(/^\uFEFF/, ''));
}

export function summarizeRust(raw: string, status: number | null, stderr = '') {
  const data = parseJson(raw);
  const vulnerabilities = data?.vulnerabilities;
  if (!Array.isArray(vulnerabilities?.list) || vulnerabilities.count !== vulnerabilities.list.length ||
      vulnerabilities.found !== (vulnerabilities.count > 0)) throw Error('Incomplete Rust advisory report');
  if (!/^[a-f0-9]{40}$/.test(data.database?.['last-commit'] ?? '') ||
      !Number.isFinite(Date.parse(data.database?.['last-updated'] ?? ''))) throw Error('Missing Rust advisory database provenance');
  const warnings: Record<string, { id: string | null; package: string; version: string }[]> = {};
  for (const [kind, items] of Object.entries(data.warnings ?? {})) {
    if (!Array.isArray(items)) throw Error('Malformed Rust warning report');
    warnings[kind] = items.map((item: any) => ({ id: item.advisory?.id ?? null, package: item.package?.name, version: item.package?.version }));
  }
  const diagnosticError = /\berror:/i.test(stderr);
  const registryCheckIncomplete = /couldn't check if the package is yanked/i.test(stderr);
  return {
    passed: status === 0 && !diagnosticError && !registryCheckIncomplete && vulnerabilities.count === 0 && !warnings.yanked?.length,
    exitCode: status, diagnosticError, registryCheckIncomplete,
    database: { commit: data.database['last-commit'], updatedAt: data.database['last-updated'], advisoryCount: data.database['advisory-count'] },
    vulnerabilities: vulnerabilities.list.map((item: any) => ({ id: item.advisory?.id, package: item.package?.name, version: item.package?.version, patched: item.versions?.patched })),
    warnings,
  };
}

export function summarizeSecrets(raw: string, status: number | null) {
  const findings = parseJson(raw);
  if (!Array.isArray(findings)) throw Error('Incomplete secret-scan report');
  return {
    passed: status === 0 && findings.length === 0,
    findings: findings.map((item: any) => ({ rule: item.RuleID, file: item.File, line: item.StartLine, commit: item.Commit })),
  };
}

const root = resolve(import.meta.dir, '..');
const evidence = join(root, 'src-tauri/target/security-audit');

function run(argv: string[], timeout = 240000) {
  const result = spawnSync(argv[0], argv.slice(1), { cwd: root, encoding: 'utf8', windowsHide: true, timeout, maxBuffer: 32 * 1024 * 1024 });
  if (result.error || result.signal || result.status === null) throw Error(`Security command failed or timed out: ${argv[0]}`);
  return result;
}

async function executable(name: ToolName) {
  if (process.arch !== 'x64' || !['win32', 'linux'].includes(process.platform)) throw Error('Security bootstrap supports Windows/Linux x64; install official tools manually on other hosts');
  const spec = tools[name];
  const asset = spec[process.platform as 'win32' | 'linux'];
  const cache = join(root, 'src-tauri/target/security-tools', `${name}-${spec.version}`);
  mkdirSync(cache, { recursive: true });
  const archive = join(cache, asset.asset);
  if (!existsSync(archive)) {
    console.error(`Downloading pinned ${name} ${spec.version} from its official release`);
    const response = await fetch(spec.release + asset.asset, { signal: AbortSignal.timeout(60000) });
    if (!response.ok) throw Error(`Security tool download failed: HTTP ${response.status}`);
    const bytes = new Uint8Array(await response.arrayBuffer());
    assertDigest(bytes, asset.sha256);
    writeFileSync(archive, bytes);
  }
  assertDigest(readFileSync(archive), asset.sha256);
  // Re-extract the verified archive, rather than trusting a previously cached executable.
  const unpacked = mkdtempSync(join(cache, 'unpacked-'));
  const literal = (value: string) => `'${value.replaceAll("'", "''")}'`;
  const extraction = process.platform === 'win32'
    ? run(['powershell.exe', '-NoProfile', '-NonInteractive', '-Command', `$ErrorActionPreference='Stop';Expand-Archive -LiteralPath ${literal(archive)} -DestinationPath ${literal(unpacked)}`], 60000)
    : run(['tar', '-xzf', archive, '-C', unpacked], 60000);
  if (extraction.status !== 0) throw Error('Could not extract the verified security tool');
  const filename = name + (process.platform === 'win32' ? '.exe' : '');
  const candidates: string[] = [];
  function find(dir: string, depth = 0) {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      if (entry.isFile() && entry.name === filename) candidates.push(join(dir, entry.name));
      else if (entry.isDirectory() && depth < 2) find(join(dir, entry.name), depth + 1);
    }
  }
  find(unpacked);
  if (candidates.length !== 1) throw Error('Expected exactly one extracted security executable');
  return candidates[0];
}

export async function auditRust() {
  mkdirSync(evidence, { recursive: true });
  const binary = await executable('cargo-audit');
  const result = run([binary, 'audit', '--file', join(root, 'src-tauri/Cargo.lock'), '--db', join(evidence, 'advisory-db'), '--deny', 'yanked', '--json']);
  const summary = summarizeRust(result.stdout, result.status, result.stderr);
  writeFileSync(join(evidence, 'rust-summary.json'), JSON.stringify({ tool: tools['cargo-audit'].version, ...summary }, null, 2));
  console.log(JSON.stringify(summary));
  if (!summary.passed) throw Error('Rust audit blocked: vulnerabilities, yanked dependencies, or incomplete registry checks; see local rust-summary.json');
}

export async function auditSecrets() {
  mkdirSync(evidence, { recursive: true });
  const shallow = run(['git', 'rev-parse', '--is-shallow-repository']);
  if (shallow.status !== 0 || shallow.stdout.trim() !== 'false') throw Error('Secret history scan requires a full, non-shallow checkout');
  const binary = await executable('gitleaks');
  const config = join(evidence, 'gitleaks-default.toml');
  const ignore = join(evidence, 'gitleaks-empty.ignore');
  writeFileSync(config, '[extend]\nuseDefault = true\n');
  writeFileSync(ignore, '');
  const report = join(evidence, 'gitleaks-history.json');
  const result = run([binary, 'git', '--log-opts=--all --full-history --diff-merges=first-parent', '--config', config, '--gitleaks-ignore-path', ignore, '--ignore-gitleaks-allow', '--redact=100', '--no-banner', '--no-color', '--timeout=120', '--report-format=json', '--report-path', report, root]);
  // Do not print matches, secrets, author emails or raw report context.
  const summary = summarizeSecrets(readFileSync(report, 'utf8'), result.status);
  const commits = run(['git', 'rev-list', '--all', '--count']);
  if (commits.status !== 0) throw Error('Could not record secret-scan scope');
  writeFileSync(join(evidence, 'secrets-summary.json'), JSON.stringify({ tool: tools.gitleaks.version, reachableCommits: Number(commits.stdout.trim()), ...summary }, null, 2));
  console.log(JSON.stringify(summary));
  if (!summary.passed) throw Error('Secret scan blocked; review only the redacted local report, never upload raw contexts');
}

if (import.meta.main) {
  try {
    const mode = process.argv[2];
    if (mode === 'rust') await auditRust();
    else if (mode === 'secrets') await auditSecrets();
    else throw Error('Usage: bun scripts/security.ts rust|secrets');
  } catch (error) {
    console.error(error instanceof Error ? error.message : 'Security check failed');
    process.exitCode = 1;
  }
}
