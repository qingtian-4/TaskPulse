// Release operations never bump versions, create tags, push, or publish.
// Build exactly the tagged commit; publication is a separate, draft-only CI job.
import { createHash } from 'node:crypto';
import { appendFileSync, copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

export const ALPHA_VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)-(alpha|beta|rc)\.(0|[1-9]\d*)$/;

export function validateRelease(tag: string, versions: string[]) {
  const version = tag.startsWith('v') ? tag.slice(1) : '';
  if (!ALPHA_VERSION.test(version)) throw new Error('Use an explicit pre-release tag, e.g. v0.2.0-alpha.1. Stable publication is not enabled.');
  if (versions.length !== 4 || versions.some(v => v !== version)) {
    throw new Error(`Tag ${tag} must match package.json, Cargo.toml, Cargo.lock and tauri.conf.json: ${versions.join(', ')}`);
  }
  return version;
}

export function projectVersions(root: string): string[] {
  const cargo = Bun.TOML.parse(readFileSync(join(root, 'src-tauri/Cargo.toml'), 'utf8')) as { package: {version:string} };
  const lock = Bun.TOML.parse(readFileSync(join(root, 'src-tauri/Cargo.lock'), 'utf8')) as {package:Array<{name:string;version:string}>};
  return [JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')).version, cargo.package.version,
    lock.package.find(p => p.name === 'agent-tray')?.version ?? '',
    JSON.parse(readFileSync(join(root, 'src-tauri/tauri.conf.json'), 'utf8')).version];
}

// Windows Installer cannot use SemVer's alpha.N identifier directly. Keep the
// public/application version intact and validate a separate numeric Wix version.
export function msiVersionFor(version:string):string {
  const match=ALPHA_VERSION.exec(version);
  if(!match)throw new Error('Invalid pre-release version');
  const [,major,minor,patch,channel,sequence]=match;
  if(Number(major)>255 || Number(minor)>255 || Number(patch)>65535 || Number(sequence)>=1000) {
    throw new Error('Version exceeds Windows Installer numeric limits');
  }
  const build={alpha:1000,beta:2000,rc:3000}[channel as 'alpha'|'beta'|'rc']+Number(sequence);
  return `${major}.${minor}.${patch}.${build}`;
}

export function validateMsiVersion(root:string,version:string):void {
  const config=JSON.parse(readFileSync(join(root,'src-tauri/tauri.conf.json'),'utf8'));
  if(config.bundle?.windows?.wix?.version!==msiVersionFor(version))throw new Error('Wix numeric product version must match the application pre-release version mapping');
}

export function installerNames(version: string) {
  if (!ALPHA_VERSION.test(version)) throw new Error('Invalid pre-release version');
  return [`TaskPulse_${version}_windows_x64_setup.exe`, `TaskPulse_${version}_windows_x64.msi`];
}

export function sha256(data: Uint8Array) { return createHash('sha256').update(data).digest('hex'); }

export function verifyChecksums(directory: string, version: string): void {
  const expected = installerNames(version).sort();
  const actual = readdirSync(directory).filter(n => /\.(exe|msi)$/i.test(n)).sort();
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error('Missing, unexpected or mismatched installer assets');
  const lines = readFileSync(join(directory, 'SHA256SUMS.txt'), 'utf8').trim().split('\n');
  if (lines.length !== expected.length) throw new Error('Incomplete checksum manifest');
  const seen = new Set<string>();
  for (const line of lines) {
    const match = /^([a-f0-9]{64})  ([A-Za-z0-9_.-]+)$/.exec(line.trim());
    if (!match || !expected.includes(match[2]) || seen.has(match[2])) throw new Error('Invalid checksum entry');
    seen.add(match[2]);
    if (sha256(readFileSync(join(directory, match[2]))) !== match[1]) throw new Error(`Checksum mismatch: ${match[2]}`);
  }
}

export function stageInstallers(bundleRoot: string, out: string, version: string, commit: string): void {
  if (!/^[a-f0-9]{40}$/.test(commit)) throw new Error('A full source commit SHA is required');
  if (existsSync(out) && readdirSync(out).length) throw new Error('Output directory must be empty; refusing stale assets');
  const names = installerNames(version);
  const sources = [['nsis', '.exe'], ['msi', '.msi']].map(([subdir, extension]) => {
    const directory = join(bundleRoot, subdir);
    const files = readdirSync(directory).filter(n => n.endsWith(extension));
    if (files.length !== 1) throw new Error(`Expected exactly one ${extension} in ${directory}`);
    const file = join(directory, files[0]);
    const bytes = readFileSync(file);
    const magic = extension === '.exe' ? '4d5a' : 'd0cf11e0a1b11ae1';
    if (!bytes.subarray(0, magic.length / 2).equals(Buffer.from(magic, 'hex'))) throw new Error(`Not an installer: ${file}`);
    return file;
  });
  mkdirSync(out, {recursive:true});
  sources.forEach((file, index) => copyFileSync(file, join(out, names[index])));
  writeFileSync(join(out, 'SHA256SUMS.txt'), names.map(name => `${sha256(readFileSync(join(out, name)))}  ${name}\n`).join(''));
  writeFileSync(join(out, 'release-manifest.json'), JSON.stringify({version, tag:`v${version}`, msi_product_version:msiVersionFor(version), commit, platform:'windows', architecture:'x64', assets:names}, null, 2)+'\n');
  verifyChecksums(out, version);
}

function git(root: string, args: string[]) {
  const result = Bun.spawnSync(['git', '-C', root, ...args], {stdout:'pipe',stderr:'pipe'});
  if (result.exitCode !== 0) throw new Error(result.stderr.toString().trim() || 'Git command failed');
  return result.stdout.toString().trim();
}

if (import.meta.main) {
  try {
    const root = resolve(import.meta.dir, '..');
    const [command, ...args] = process.argv.slice(2);
    const argument = (name: string) => {
      const i = args.indexOf(name);
      if (i < 0 || !args[i+1] || args[i+1].startsWith('--')) throw new Error(`Missing ${name}`);
      return args[i+1];
    };
    if (command === 'check') {
      const versions = projectVersions(root);
      const version = validateRelease(`v${versions[0]}`, versions);
      validateMsiVersion(root,version);
      console.log(`Release versions consistent: ${version}`);
    } else if (command === 'preflight') {
      const tag = argument('--tag');
      const version = validateRelease(tag, projectVersions(root));
      validateMsiVersion(root,version);
      if (!existsSync(join(root, `docs/releases/${version}.md`))) throw new Error('Missing version-specific release notes');
      const commit = git(root, ['rev-parse', 'HEAD']);
      const tagged = git(root, ['rev-parse', '--verify', `${tag}^{commit}`]);
      if (commit !== tagged) throw new Error('Checkout must be the existing release tag, not a branch or another commit');
      // No shell interpolation of user-supplied refs; validated outputs are safe.
      if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `version=${version}\ntag=${tag}\ncommit=${commit}\n`);
      console.log(`Validated ${tag} at ${commit}`);
    } else if (command === 'stage') {
      stageInstallers(resolve(argument('--bundle-root')), resolve(argument('--out')), argument('--version'), argument('--commit'));
    } else if (command === 'verify') {
      verifyChecksums(resolve(argument('--out')), argument('--version'));
    } else {
      throw new Error('Usage: bun scripts/release.ts check | preflight --tag vVERSION | stage --bundle-root DIR --out DIR --version VERSION --commit SHA | verify --out DIR --version VERSION');
    }
  } catch (error) { console.error(String(error)); process.exit(1); }
}
