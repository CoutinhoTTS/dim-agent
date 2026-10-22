import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { copyFileSync, cpSync, existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync,
  rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';

// The Web workspace is one platform-independent artifact per release
// (docs/design/web-workspace.md §2.4): this builds `web/` once, records the
// protocol it speaks beside the build, and packs the same tree as the native
// archive dimcode-web-VERSION.tar.gz and the npm package dimcode-web, which the
// dimcode wrapper lists as an optional dependency. `--dist DIR` takes an
// already built tree instead of running npm (tests, a prebuilt frontend).
const [destination, version, sourceCommit, ...rest] = process.argv.slice(2);
const usage = 'Usage: node scripts/stage-web.mjs DESTINATION VERSION SOURCE_COMMIT [--dist DIR]';
assert(destination && version && sourceCommit, usage);
assert(rest.length === 0 || (rest.length === 2 && rest[0] === '--dist'), usage);
assert(/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$/.test(version), 'Invalid version');
assert(/^[a-f0-9]{40}$/.test(sourceCommit), 'Invalid source commit');
const repo = path.resolve(import.meta.dirname, '..');
const productVersion = readFileSync(path.join(repo, 'Cargo.toml'), 'utf8').match(/\[workspace.package\]\s+version = "([^"]+)"/)[1];
assert.equal(version, productVersion, 'Version must match Cargo workspace');
assert.equal(version, JSON.parse(readFileSync(path.join(repo, 'npm/package.json'))).version, 'Version must match npm wrapper');
// The protocol the frontend's transport speaks; the App Server refuses a
// frontend recorded for another one.
const protocolVersion = Number(readFileSync(path.join(repo, 'crates/dim-app-server/src/protocol.rs'), 'utf8')
  .match(/pub const PROTOCOL_VERSION: u32 = (\d+);/)[1]);
const output = path.resolve(destination);
assert(!existsSync(output), 'Destination already exists');
const digest = (data) => createHash('sha256').update(data).digest('hex');
const scratch = mkdtempSync(path.join(os.tmpdir(), 'dimcode-web-'));
const runTar = (args) => {
  // macOS tar would otherwise add AppleDouble `._*` entries for extended attributes.
  const result = spawnSync('tar', args, { encoding: 'utf8', env: { ...process.env, COPYFILE_DISABLE: '1' } });
  assert.ifError(result.error);
  assert.equal(result.status, 0, 'Archive creation failed');
};
try {
  let dist;
  if (rest.length) {
    dist = path.resolve(rest[1]);
  } else {
    const web = path.join(repo, 'web');
    const npm = (args) => {
      const result = spawnSync(process.platform === 'win32' ? 'npm.cmd' : 'npm', args, { cwd: web, stdio: 'inherit', shell: process.platform === 'win32' });
      assert.ifError(result.error);
      assert.equal(result.status, 0, `npm ${args[0]} failed in web/`);
    };
    npm(['ci', '--ignore-scripts', '--no-audit', '--no-fund']);
    dist = path.join(scratch, 'dist');
    npm(['run', 'build', '--', '--outDir', dist, '--emptyOutDir']);
  }
  assert(lstatSync(path.join(dist, 'index.html')).isFile(), 'The frontend build has no index.html');
  // The staged tree: web/ as the App Server serves it, the license, the record.
  const name = `dimcode-web`;
  const stage = path.join(scratch, name);
  cpSync(dist, path.join(stage, 'web'), { recursive: true });
  writeFileSync(path.join(stage, 'web', 'dimcode-web.json'), `${JSON.stringify({ version, protocolVersion, sourceCommit }, null, 2)}\n`);
  copyFileSync(path.join(repo, 'LICENSE'), path.join(stage, 'LICENSE'));
  const files = {};
  for (const entry of readdirSync(stage, { withFileTypes: true, recursive: true })) {
    assert(entry.isFile() || entry.isDirectory(), 'Unexpected frontend file type');
    if (entry.isFile()) files[path.relative(stage, path.join(entry.parentPath, entry.name)).split(path.sep).join('/')] = digest(readFileSync(path.join(entry.parentPath, entry.name)));
  }
  const toolchain = { node: process.versions.node };
  const staged = { schemaVersion: 1, version, sourceCommit, platform: 'web', toolchain, protocolVersion, files };
  writeFileSync(path.join(stage, 'release-manifest.json'), `${JSON.stringify(staged, null, 2)}\n`);
  mkdirSync(output, { recursive: true });
  const native = `${name}-${version}.tar.gz`;
  runTar(['-czf', path.join(output, native), '-C', scratch, name]);
  const packageRoot = path.join(scratch, 'package');
  cpSync(stage, packageRoot, { recursive: true });
  writeFileSync(path.join(packageRoot, 'package.json'), `${JSON.stringify({
    name, version, description: 'Dim Web workspace frontend, served by the dimcode App Server', license: 'MIT',
    repository: { type: 'git', url: 'git+https://github.com/arcships/dim.git' },
    files: ['web', 'LICENSE', 'release-manifest.json'],
  }, null, 2)}\n`);
  const archive = `${name}-${version}.tgz`;
  runTar(['-czf', path.join(output, archive), '-C', scratch, 'package']);
  const result = { ...staged,
    npmPackages: [{ name, platform: 'web', file: archive, sha256: digest(readFileSync(path.join(output, archive))) }],
    nativeArchives: [{ platform: 'web', file: native, sha256: digest(readFileSync(path.join(output, native))) }],
  };
  writeFileSync(path.join(output, 'release-manifest.json'), `${JSON.stringify(result, null, 2)}\n`);
  console.log(`Staged the Web workspace ${version} (protocol ${protocolVersion}).`);
} catch (error) {
  rmSync(output, { recursive: true, force: true });
  throw error;
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
