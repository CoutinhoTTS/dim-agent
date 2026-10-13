import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { lstatSync, readFileSync } from 'node:fs';
import path from 'node:path';

const [directory, version, sourceCommit, mode, ...extra] = process.argv.slice(2);
assert(directory && version && sourceCommit && extra.length === 0,
  'Usage: node scripts/publish-npm.mjs DIRECTORY VERSION SOURCE_COMMIT [--publish]');
assert(mode === undefined || mode === '--publish', 'Unknown publish mode');
assert(/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$/.test(version),
  'Invalid release version');
assert(/^[a-f0-9]{40}$/.test(sourceCommit), 'Invalid source commit');

const root = path.resolve(directory);
const manifestPath = path.join(root, 'release-manifest.json');
assert(lstatSync(manifestPath).isFile(), 'Release manifest must be a regular file');
const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
assert.equal(manifest.version, version, 'Release version mismatch');
assert.equal(manifest.sourceCommit, sourceCommit, 'Release source mismatch');

const platforms = [
  ['darwin-arm64', 'darwin', 'arm64'],
  ['darwin-x64', 'darwin', 'x64'],
  ['linux-arm64', 'linux', 'arm64'],
  ['linux-x64', 'linux', 'x64'],
  ['windows-x64', 'win32', 'x64'],
];
const names = [...platforms.map(([platform]) => `dimcode-${platform}`), 'dimcode'];
assert(Array.isArray(manifest.npmPackages), 'Missing npm package manifest');
assert.deepEqual(manifest.npmPackages.map(({ name }) => name).sort(), [...names].sort(),
  'Release must contain all five platform packages and one wrapper');

const archives = [];
for (const name of names) {
  const entry = manifest.npmPackages.find((entry) => entry.name === name);
  assert.equal(entry.file, `${name}-${version}.tgz`, 'Unexpected package filename');
  assert(typeof entry.sha256 === 'string' && /^[a-f0-9]{64}$/.test(entry.sha256),
    'Invalid package digest');
  const archive = path.join(root, entry.file);
  assert(lstatSync(archive).isFile(), 'Package archive must be a regular file');
  assert.equal(createHash('sha256').update(readFileSync(archive)).digest('hex'), entry.sha256,
    `Package digest mismatch: ${name}`);
  const packed = spawnSync('tar', ['-xOzf', archive, 'package/package.json'], { encoding: 'utf8' });
  assert.ifError(packed.error);
  assert.equal(packed.status, 0, `Cannot read package metadata: ${name}`);
  const metadata = JSON.parse(packed.stdout);
  assert.equal(metadata.name, name, 'Packed package name mismatch');
  assert.equal(metadata.version, version, 'Packed package version mismatch');
  assert.equal(metadata.repository?.url, 'git+https://github.com/arcships/dim.git',
    'Packed package repository mismatch');
  if (name === 'dimcode') {
    assert.deepEqual(metadata.optionalDependencies,
      Object.fromEntries(names.slice(0, -1).map((name) => [name, version])),
      'Wrapper must reference the exact platform package versions');
    assert.equal(typeof metadata.bin?.dim, 'string', 'Missing dim command');
    assert(metadata.bin.dim.length > 0 && metadata.bin.dimcode === metadata.bin.dim,
      'dim and dimcode must use the same wrapper');
  } else {
    const [, os, cpu] = platforms.find(([platform]) => name === `dimcode-${platform}`);
    assert.deepEqual(metadata.os, [os], 'Packed package OS mismatch');
    assert.deepEqual(metadata.cpu, [cpu], 'Packed package CPU mismatch');
  }
  archives.push(archive);
}

console.log(`Validated ${archives.length} npm packages for ${version} (${sourceCommit}).`);
if (mode === '--publish') {
  const tag = version.includes('-') ? 'next' : 'latest';
  for (const archive of archives) {
    const published = spawnSync('npm', ['publish', archive, '--access', 'public', '--tag', tag,
      '--ignore-scripts', '--registry', 'https://registry.npmjs.org'], { stdio: 'inherit' });
    assert.ifError(published.error);
    assert.equal(published.status, 0, `npm publish failed: ${path.basename(archive)}`);
  }
}
