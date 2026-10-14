import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { lstatSync, mkdirSync, readFileSync } from 'node:fs';
import path from 'node:path';

export const pluginArchiveLimit = 16 * 1024 * 1024;

export function extractPluginArchive(archive, expectedDigest, destination) {
  assert(/^[a-f0-9]{64}$/.test(expectedDigest ?? ''), 'Plugin SHA-256 is required');
  const archiveInfo = lstatSync(archive);
  assert(archiveInfo.isFile(), 'Plugin archive must be a regular file');
  assert(archiveInfo.size <= pluginArchiveLimit, 'Plugin archive is too large');
  const data = readFileSync(archive);
  assert(data.length <= pluginArchiveLimit, 'Plugin archive is too large');
  const digest = (bytes) => createHash('sha256').update(bytes).digest('hex');
  assert.equal(digest(data), expectedDigest, 'Plugin archive digest mismatch');
  const tar = (args) => {
    const result = spawnSync('tar', args, { encoding: 'utf8', input: data });
    assert.ifError(result.error);
    assert.equal(result.status, 0, 'Plugin archive could not be read');
    return result.stdout;
  };
  const names = tar(['-tzf', '-']).trim().split('\n').map((name) => name.replace(/^\.\//, '')).filter(Boolean);
  const required = ['package.json', 'dim-plugin.json', 'dist/index.js'];
  assert.equal(new Set(names).size, names.length, 'Duplicate plugin archive entries');
  assert.deepEqual(names.filter((name) => !name.endsWith('/')).sort(), required.toSorted(), 'Unexpected plugin archive files');
  assert(names.every((name) => required.includes(name) || ['', 'dist/'].includes(name)), 'Unsafe plugin archive path');
  assert(tar(['-tvzf', '-']).trim().split('\n').every((line) => ['-', 'd'].includes(line[0])), 'Plugin archive links are forbidden');
  mkdirSync(destination);
  tar(['-xzf', '-', '-C', destination]);
  const manifest = JSON.parse(readFileSync(path.join(destination, 'dim-plugin.json')));
  const metadata = JSON.parse(readFileSync(path.join(destination, 'package.json')));
  assert.equal(manifest.schemaVersion, 1, 'Unsupported plugin schema');
  assert.equal(manifest.id, 'dim-oauth', 'Unexpected plugin ID');
  assert.equal(manifest.hostApiVersion, 2, 'Unsupported host API');
  assert.equal(manifest.rutisVersion, '0.10.0', 'Unsupported rutis version');
  assert.equal(manifest.entry, 'dist/index.js', 'Unexpected plugin entry');
  assert.equal(metadata.type, 'module', 'Plugin must be ESM');
  assert.equal(metadata.version, manifest.version, 'Plugin version mismatch');
  assert.deepEqual(Object.keys(manifest.files), ['dist/index.js'], 'Unexpected plugin file manifest');
  assert.equal(manifest.files['dist/index.js'], digest(readFileSync(path.join(destination, 'dist/index.js'))), 'Plugin entry digest mismatch');
  return { version: manifest.version, sha256: expectedDigest, hostApiVersion: 2, rutisVersion: '0.10.0' };
}
