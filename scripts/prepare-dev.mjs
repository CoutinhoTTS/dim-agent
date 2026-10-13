import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { extractPluginArchive } from './plugin-package.mjs';

const [source, ...extra] = process.argv.slice(2);
assert(source && extra.length === 0, 'Usage: node scripts/prepare-dev.mjs OAUTH_SOURCE_DIR');
const repo = path.resolve(import.meta.dirname, '..');
const sourceRoot = path.resolve(source);
assert.equal(process.versions.node, readFileSync(path.join(repo, '.node-version'), 'utf8').trim(), 'Use the pinned Node version for plugin development');
const state = path.join(repo, '.dev');
mkdirSync(state, { recursive: true, mode: 0o700 });
const run = (command, args, cwd) => {
  const result = spawnSync(command, args, { cwd, stdio: ['inherit', 2, 2] });
  assert.ifError(result.error);
  assert.equal(result.status, 0, `${command} failed`);
};
const fingerprint = (directory, lockfile) => createHash('sha256')
  .update(directory).update(process.versions.node)
  .update(readFileSync(path.join(directory, 'package.json')))
  .update(readFileSync(path.join(directory, lockfile))).digest('hex');
const metadata = JSON.parse(readFileSync(path.join(sourceRoot, 'package.json')));
assert.equal(metadata.name, '@arcships/dim-oauth-plugin', 'Unexpected local provider package');
assert(/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$/.test(metadata.version), 'Invalid plugin version');
assert.match(metadata.packageManager, /^pnpm@\d+\.\d+\.\d+$/);
const pnpm = spawnSync('pnpm', ['--version'], { encoding: 'utf8' });
assert.ifError(pnpm.error);
assert.equal(pnpm.status, 0, 'pnpm version check failed');
assert.equal(`pnpm@${pnpm.stdout.trim()}`, metadata.packageManager, 'Use the plugin packageManager version');
const pluginKey = fingerprint(sourceRoot, 'pnpm-lock.yaml');
const pluginMarker = path.join(state, 'oauth-dependencies.sha256');
const pluginInstalled = ['@arcships/rutis/package.json', '.bin/tsc', '.bin/esbuild']
  .every((file) => existsSync(path.join(sourceRoot, 'node_modules', file)));
if (!pluginInstalled || !existsSync(pluginMarker) || readFileSync(pluginMarker, 'utf8') !== pluginKey) {
  run('pnpm', ['install', '--frozen-lockfile', '--ignore-scripts'], sourceRoot);
  writeFileSync(pluginMarker, pluginKey);
}
run('pnpm', ['run', 'pack:runtime'], sourceRoot);
const runtime = path.join(repo, 'runtime');
const runtimeKey = fingerprint(runtime, 'package-lock.json');
const runtimeMarker = path.join(state, 'runtime-dependencies.sha256');
const runtimeInstalled = ['@arcships/rutis-runtime/package.json', '@arcships/rutis/package.json']
  .every((file) => existsSync(path.join(runtime, 'node_modules', file)));
if (!runtimeInstalled || !existsSync(runtimeMarker) || readFileSync(runtimeMarker, 'utf8') !== runtimeKey) {
  run('npm', ['ci', '--ignore-scripts', '--no-audit', '--no-fund'], runtime);
  writeFileSync(runtimeMarker, runtimeKey);
}
const filename = `dim-oauth-plugin-${metadata.version}.tar.gz`;
const archive = path.join(sourceRoot, 'artifacts', filename);
const checksum = readFileSync(`${archive}.sha256`, 'utf8').trim().split(/\s+/);
assert.equal(checksum.length, 2, 'Invalid plugin checksum file');
assert.equal(checksum[1], filename, 'Plugin checksum filename mismatch');
const scratch = mkdtempSync(path.join(state, 'prepare-'));
try {
  const verified = path.join(scratch, 'verified');
  const plugin = extractPluginArchive(archive, checksum[0], verified);
  const destination = path.join(state, 'plugin');
  const previous = path.join(scratch, 'previous');
  if (existsSync(destination)) renameSync(destination, previous);
  try { renameSync(verified, destination); }
  catch (error) {
    if (existsSync(previous)) renameSync(previous, destination);
    throw error;
  }
  console.error(`Prepared local provider ${plugin.version} (${plugin.sha256}).`);
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
