import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const script = fileURLToPath(new URL('./publish-npm.mjs', import.meta.url));
const sourceCommit = 'a'.repeat(40);
const platforms = [
  ['darwin-arm64', 'darwin', 'arm64'], ['darwin-x64', 'darwin', 'x64'],
  ['linux-arm64', 'linux', 'arm64'], ['linux-x64', 'linux', 'x64'],
  ['windows-x64', 'win32', 'x64'],
];

function fixture(t, version = '1.2.3') {
  const root = mkdtempSync(path.join(os.tmpdir(), 'dim-npm-release-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const names = [...platforms.map(([platform]) => `dimcode-${platform}`), 'dimcode'];
  const manifest = { sourceCommit, version, npmPackages: [] };
  for (const name of names) {
    const directory = path.join(root, name);
    mkdirSync(path.join(directory, 'package'), { recursive: true });
    const metadata = {
      name, version, repository: { type: 'git', url: 'git+https://github.com/arcships/dim.git' },
    };
    if (name === 'dimcode') {
      metadata.optionalDependencies = Object.fromEntries(names.slice(0, -1).map((name) => [name, version]));
      metadata.bin = { dim: 'bin/dim.mjs', dimcode: 'bin/dim.mjs' };
    } else {
      const [, os, cpu] = platforms.find(([platform]) => name === `dimcode-${platform}`);
      metadata.os = [os];
      metadata.cpu = [cpu];
    }
    writeFileSync(path.join(directory, 'package/package.json'), JSON.stringify(metadata));
    const file = `${name}-${version}.tgz`;
    const packed = spawnSync('tar', ['-czf', path.join(root, file), '-C', directory, 'package']);
    assert.equal(packed.status, 0);
    manifest.npmPackages.push({ name, file,
      sha256: createHash('sha256').update(readFileSync(path.join(root, file))).digest('hex') });
  }
  // Registry writes are the only substituted boundary; package archives are real.
  const bin = path.join(root, 'bin');
  mkdirSync(bin);
  writeFileSync(path.join(bin, 'npm'), '#!/bin/sh\nprintf "%s\\n" "$*" >> "$PUBLISH_LOG"\n', { mode: 0o755 });
  const log = path.join(root, 'published.log');
  writeFileSync(log, '');
  function run({ publish = false, commit = sourceCommit } = {}) {
    writeFileSync(path.join(root, 'release-manifest.json'), JSON.stringify(manifest));
    return spawnSync(process.execPath, [script, root, version, commit, ...(publish ? ['--publish'] : [])], {
      encoding: 'utf8', env: { ...process.env, PATH: `${bin}${path.delimiter}${process.env.PATH}`, PUBLISH_LOG: log },
    });
  }
  return { root, names, manifest, log, run };
}

test('validates without publishing and publishes the wrapper after every platform', (t) => {
  const release = fixture(t);
  assert.equal(release.run().status, 0);
  assert.equal(readFileSync(release.log, 'utf8'), '');
  const published = release.run({ publish: true });
  assert.equal(published.status, 0, published.stderr);
  const calls = readFileSync(release.log, 'utf8').trim().split('\n');
  assert.equal(calls.length, 6);
  for (const [index, name] of release.names.entries()) {
    assert.equal(calls[index], `publish ${path.join(release.root, `${name}-1.2.3.tgz`)} --access public --tag latest --ignore-scripts --registry https://registry.npmjs.org`);
  }
});

test('rejects a corrupted wrapper before any platform is published', (t) => {
  const release = fixture(t);
  writeFileSync(path.join(release.root, 'dimcode-1.2.3.tgz'), 'corrupt');
  assert.notEqual(release.run({ publish: true }).status, 0);
  assert.equal(readFileSync(release.log, 'utf8'), '');
});

test('rejects wrong source commits, missing platforms and mismatched packed versions', (t) => {
  const release = fixture(t);
  assert.notEqual(release.run({ publish: true, commit: 'b'.repeat(40) }).status, 0);
  const platform = release.manifest.npmPackages.shift();
  assert.notEqual(release.run({ publish: true }).status, 0);
  release.manifest.npmPackages.unshift(platform);
  const packageDirectory = path.join(release.root, 'dimcode');
  const metadataPath = path.join(packageDirectory, 'package/package.json');
  const metadata = JSON.parse(readFileSync(metadataPath, 'utf8'));
  metadata.version = '1.2.4';
  writeFileSync(metadataPath, JSON.stringify(metadata));
  const wrapper = release.manifest.npmPackages.at(-1);
  const archive = path.join(release.root, wrapper.file);
  assert.equal(spawnSync('tar', ['-czf', archive, '-C', packageDirectory, 'package']).status, 0);
  wrapper.sha256 = createHash('sha256').update(readFileSync(archive)).digest('hex');
  assert.notEqual(release.run({ publish: true }).status, 0);
  assert.equal(readFileSync(release.log, 'utf8'), '');
});

test('prerelease versions publish only to next', (t) => {
  const release = fixture(t, '1.2.3-beta.1');
  const published = release.run({ publish: true });
  assert.equal(published.status, 0, published.stderr);
  for (const call of readFileSync(release.log, 'utf8').trim().split('\n')) {
    assert(call.includes('--tag next'));
  }
});
