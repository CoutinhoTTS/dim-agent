import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { chmodSync, copyFileSync, existsSync, lstatSync, mkdirSync, readFileSync, readdirSync,
  rmSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { buildPlatform, checkArchitecture } from './native-binary.mjs';

const [binaryDirectory, destination, version, sourceCommit, ...rest] = process.argv.slice(2);
const usage = 'Usage: node scripts/stage-release.mjs BIN_DIR DESTINATION VERSION SOURCE_COMMIT [--runtime RUNTIME_DIR]';
assert(binaryDirectory && destination && version && sourceCommit, usage);
assert(rest.length === 0 || (rest.length === 2 && rest[0] === '--runtime'), usage);
const runtimeDirectory = rest[1];
assert(/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$/.test(version), 'Invalid version');
assert(/^[a-f0-9]{40}$/.test(sourceCommit), 'Invalid source commit');
const { platform, host, osName, cpu } = buildPlatform();
const repo = path.resolve(import.meta.dirname, '..');
const productVersion = readFileSync(path.join(repo, 'Cargo.toml'), 'utf8').match(/\[workspace.package\]\s+version = "([^"]+)"/)[1];
assert.equal(version, productVersion, 'Version must match Cargo workspace');
assert.equal(version, JSON.parse(readFileSync(path.join(repo, 'npm/package.json'))).version, 'Version must match npm wrapper');
const output = path.resolve(destination);
assert(!existsSync(output), 'Destination already exists');
const digest = (data) => createHash('sha256').update(data).digest('hex');
const suffix = osName === 'windows' ? '.exe' : '';
const cli = path.resolve(binaryDirectory, `dimcode${suffix}`);
// One executable: the CLI runs its App Server daemon as `dimcode app-server`,
// and an account build carries its runtime inside (stage-runtime.mjs).
assert(lstatSync(cli).isFile(), 'The native executable must be a regular file');
if (platform === host) {
  const result = spawnSync(cli, ['--version'], { encoding: 'utf8' });
  assert.ifError(result.error);
  assert.equal(result.status, 0, `dimcode --version failed: ${result.stderr}`);
  assert.equal(result.stdout.trim(), `dimcode ${version}`, 'Binary version mismatch');
} else {
  // shortcut: Cross-built binaries are inspected; add target-machine smoke tests when runners exist.
  checkArchitecture(cli, osName, cpu);
}
let runtime = null;
if (runtimeDirectory) {
  runtime = JSON.parse(readFileSync(path.join(runtimeDirectory, 'runtime.json'), 'utf8'));
  assert.equal(runtime.schemaVersion, 1, 'Unknown runtime schema');
  assert.equal(runtime.platform, platform, 'Runtime platform mismatch');
  assert.equal(digest(readFileSync(path.join(runtimeDirectory, 'runtime.bin'))), runtime.sha256, 'Runtime archive digest mismatch');
  // build.rs embeds the archive's digest beside it.
  assert(readFileSync(cli).includes(Buffer.from(runtime.sha256)), 'The executable does not embed this runtime');
}
try {
  mkdirSync(path.join(output, 'bin'), { recursive: true });
  copyFileSync(cli, path.join(output, 'bin', `dimcode${suffix}`));
  chmodSync(path.join(output, 'bin', `dimcode${suffix}`), 0o755);
  copyFileSync(path.join(repo, 'LICENSE'), path.join(output, 'LICENSE'));
  if (runtime) {
    writeFileSync(path.join(output, 'NOTICE'), 'Dim public source is licensed under MIT; see LICENSE.\nThe private dim-oauth plugin embedded in bin/dimcode is excluded from that MIT grant.\nThird-party runtimes retain their original license notices.\n');
  }
  const toolchain = { rust: readFileSync(path.join(repo, 'rust-toolchain.toml'), 'utf8').match(/channel = "([^"]+)"/)[1], ...runtime?.toolchain };
  const files = {};
  for (const entry of readdirSync(output, { withFileTypes: true, recursive: true })) {
    if (entry.isFile()) files[path.relative(output, path.join(entry.parentPath, entry.name)).split(path.sep).join('/')] = digest(readFileSync(path.join(entry.parentPath, entry.name)));
    else assert(entry.isDirectory(), 'Unexpected release file type');
  }
  const embedded = runtime && { sha256: runtime.sha256, files: runtime.files };
  writeFileSync(path.join(output, 'release-manifest.json'), `${JSON.stringify({ schemaVersion: 1, version, sourceCommit, platform, toolchain, plugin: runtime?.plugin ?? null, runtime: embedded, files }, null, 2)}\n`);
  console.log(`Staged ${platform} ${version} (${runtime ? 'with private plugin' : 'API key only'}).`);
} catch (error) {
  rmSync(output, { recursive: true, force: true });
  throw error;
}
