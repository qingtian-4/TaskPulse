// Check the emitted desktop entry pages before Tauri embeds the frontend.
import { accessSync, readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

const root = resolve(process.argv[2] ?? 'build');
const scan = new Bun.Transpiler({ loader: 'js' });
const pages = ['index.html', 'manage/index.html', 'notice.html'];
let sharedEntries: string | undefined;

for (const page of pages) {
  const html = readFileSync(join(root, page), 'utf8');
  const assets = [...html.matchAll(/(?:href=|import\()["'](\/_app\/immutable\/[\w./-]+)["']/g)].map(match => match[1]);
  for (const asset of new Set(assets)) accessSync(join(root, asset));

  // Kit 3 passes the HTML payload into init() before starting the route app.
  // The global name is no longer compiled into a JS chunk.
  const payloads = [...html.matchAll(/\b(__sveltekit_[a-z0-9]+)\s*=\s*\{/g)].map(match => match[1]);
  const initialized = html.match(/\bkit\.init\((__sveltekit_[a-z0-9]+)\)/)?.[1];
  const imported = [...html.matchAll(/\bimport\(["'](\/_app\/immutable\/entry\/[\w.-]+\.js)["']\)/g)].map(match => match[1]);
  const [runtime, app] = imported;
  if (payloads.length !== 1 || initialized !== payloads[0] ||
      imported.length !== 2 || !runtime?.includes('/start.') || !app?.includes('/app.') ||
      !/\bkit\.start\(app,/.test(html)) {
    throw Error(`Invalid frontend startup in ${page}`);
  }

  const entries = [runtime, app].join(',');
  if (sharedEntries && sharedEntries !== entries) throw Error(`Frontend entry modules differ in ${page}`);
  sharedEntries = entries;
  const exports = scan.scan(readFileSync(join(root, runtime), 'utf8')).exports;
  if (!exports.includes('init') || !exports.includes('start')) throw Error(`Missing frontend startup exports in ${runtime}`);
}
console.log('Frontend startup payloads and assets match for dashboard, settings and reminders.');
