// Stages the account runtime the dimcode binary embeds: the target's Node,
// the locked rutis runtime and the verified private plugin, as one archive.
//
//   DIM_BUILD_PLATFORM=PLATFORM DIM_OAUTH_PACKAGE_SHA256=… DIM_OAUTH_PACKAGE_ARCHIVE=… \
//     node scripts/stage-runtime.mjs DESTINATION
//
// DESTINATION/runtime.bin is built into the binary through
// DIMCODE_EMBEDDED_RUNTIME; DESTINATION/runtime.json records its provenance
// for stage-release.mjs. The archive is gzip over "DIMRT1\n" followed, per
// file in path order, by: u32 BE path length, UTF-8 path, u8 flags (1 =
// executable), u64 BE size, the bytes. The CLI extracts it under its product
// home (crates/dim-cli/src/runtime.rs reads the same format).
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { chmodSync, copyFileSync, cpSync, existsSync, lstatSync, mkdirSync, mkdtempSync,
  readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { gzipSync } from 'node:zlib';
import { buildPlatform, checkArchitecture } from './native-binary.mjs';
import { extractPluginArchive, pluginArchiveLimit } from './plugin-package.mjs';

const [destination, ...extra] = process.argv.slice(2);
assert(destination && extra.length === 0, 'Usage: node scripts/stage-runtime.mjs DESTINATION');
const { platform, host, osName, cpu } = buildPlatform();
const repo = path.resolve(import.meta.dirname, '..');
const output = path.resolve(destination);
assert(!existsSync(output), 'Destination already exists');
const digest = (data) => createHash('sha256').update(data).digest('hex');
const run = (command, args, options = {}) => {
  const result = spawnSync(command, args, { encoding: 'utf8', ...options });
  assert.ifError(result.error);
  assert.equal(result.status, 0, `${command} failed: ${result.stderr}`);
  return result.stdout;
};

async function pluginBytes() {
  const local = process.env.DIM_OAUTH_PACKAGE_ARCHIVE;
  const url = process.env.DIM_OAUTH_PACKAGE_URL;
  assert(Boolean(local) !== Boolean(url), 'Set exactly one plugin archive source');
  if (local) {
    assert(lstatSync(local).isFile(), 'Plugin archive must be a regular file');
    return readFileSync(local);
  }
  let address;
  try { address = new URL(url); }
  catch { throw new Error('Invalid private plugin URL'); }
  assert(['https:', 'http:'].includes(address.protocol) && !address.username && !address.password,
    'Invalid plugin URL');
  const headers = {};
  if (process.env.DIM_OAUTH_DOWNLOAD_TOKEN) headers.Authorization = `Bearer ${process.env.DIM_OAUTH_DOWNLOAD_TOKEN}`;
  let response;
  try { response = await fetch(address, { headers, redirect: 'error', signal: AbortSignal.timeout(60_000) }); }
  catch { throw new Error('Private plugin download failed'); }
  assert(response.ok, 'Private plugin download failed');
  const chunks = [];
  let size = 0;
  try {
    for await (const chunk of response.body) {
      size += chunk.length;
      assert(size <= pluginArchiveLimit, 'Plugin archive is too large');
      chunks.push(chunk);
    }
  } catch { throw new Error('Private plugin download failed or exceeded its size limit'); }
  return Buffer.concat(chunks);
}

/** The target's Node executable and its license: the host's own, or the official distribution. */
async function targetNode(scratch, nodeVersion) {
  if (platform === host) {
    return {
      binary: process.execPath,
      license: path.join(path.dirname(process.execPath), process.platform === 'win32' ? 'LICENSE' : '../LICENSE'),
    };
  }
  const distribution = `node-v${nodeVersion}-${osName === 'windows' ? 'win' : osName}-${cpu}`;
  const filename = `${distribution}.${osName === 'windows' ? 'zip' : 'tar.gz'}`;
  const base = `https://nodejs.org/dist/v${nodeVersion}/`;
  const checksums = await fetch(`${base}SHASUMS256.txt`, { redirect: 'error', signal: AbortSignal.timeout(60_000) });
  assert(checksums.ok, 'Node checksum download failed');
  const expected = (await checksums.text()).split('\n').map(line => line.trim().split(/\s+/)).find(([, name]) => name === filename)?.[0];
  assert(/^[a-f0-9]{64}$/.test(expected ?? ''), 'Missing target Node checksum');
  const response = await fetch(`${base}${filename}`, { redirect: 'error', signal: AbortSignal.timeout(120_000) });
  assert(response.ok, 'Target Node download failed');
  const archive = Buffer.from(await response.arrayBuffer());
  assert.equal(digest(archive), expected, 'Target Node archive digest mismatch');
  const nodeArchive = path.join(scratch, filename);
  writeFileSync(nodeArchive, archive);
  run('tar', ['-xf', nodeArchive, '-C', scratch]);
  return {
    binary: path.join(scratch, distribution, osName === 'windows' ? 'node.exe' : 'bin/node'),
    license: path.join(scratch, distribution, 'LICENSE'),
  };
}

const scratch = mkdtempSync(path.join(os.tmpdir(), 'dimcode-runtime-'));
try {
  const expectedDigest = process.env.DIM_OAUTH_PACKAGE_SHA256;
  assert(/^[a-f0-9]{64}$/.test(expectedDigest ?? ''), 'DIM_OAUTH_PACKAGE_SHA256 is required');
  const pluginArchive = path.join(scratch, 'plugin.tar.gz');
  writeFileSync(pluginArchive, await pluginBytes());
  const plugin = extractPluginArchive(pluginArchive, expectedDigest, path.join(scratch, 'plugin'));

  const nodeVersion = readFileSync(path.join(repo, '.node-version'), 'utf8').trim();
  assert.equal(process.versions.node, nodeVersion, 'Node version mismatch');
  const node = await targetNode(scratch, nodeVersion);
  checkArchitecture(node.binary, osName, cpu);
  assert(lstatSync(node.license).isFile(), 'Node distribution LICENSE is required');

  // The tree the CLI extracts: the same layout an installed payload had.
  const tree = path.join(scratch, 'tree');
  const nodeDirectory = path.join(tree, 'runtime/node');
  const nodeExecutable = path.join(nodeDirectory, osName === 'windows' ? 'node.exe' : 'bin/node');
  mkdirSync(path.dirname(nodeExecutable), { recursive: true });
  copyFileSync(node.binary, nodeExecutable);
  chmodSync(nodeExecutable, 0o755);
  copyFileSync(node.license, path.join(nodeDirectory, 'LICENSE'));
  const rutis = path.join(tree, 'runtime/rutis');
  mkdirSync(rutis);
  copyFileSync(path.join(repo, 'runtime/package.json'), path.join(rutis, 'package.json'));
  copyFileSync(path.join(repo, 'runtime/package-lock.json'), path.join(rutis, 'package-lock.json'));
  const npmCli = path.join(path.dirname(process.execPath), process.platform === 'win32' ? 'node_modules/npm/bin/npm-cli.js' : '../lib/node_modules/npm/bin/npm-cli.js');
  assert(lstatSync(npmCli).isFile(), 'Node distribution npm CLI is required');
  run(process.execPath, [npmCli, 'ci', '--omit=dev', '--ignore-scripts',
    `--os=${osName === 'windows' ? 'win32' : osName}`, `--cpu=${cpu}`, '--registry=https://registry.npmjs.org'], { cwd: rutis });
  assert(lstatSync(path.join(rutis, 'node_modules/@esbuild', `${osName === 'windows' ? 'win32' : osName}-${cpu}`,
    osName === 'windows' ? 'esbuild.exe' : 'bin/esbuild')).isFile(), 'Missing target runtime executable');
  // npm's bin links are not used at runtime; the archive holds regular files only.
  rmSync(path.join(rutis, 'node_modules/.bin'), { recursive: true, force: true });
  cpSync(path.join(scratch, 'plugin'), path.join(tree, 'plugins/dim-oauth'), { recursive: true });

  const files = {};
  const entries = [];
  const collect = (directory) => {
    for (const entry of readdirSync(directory, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : 1))) {
      const absolute = path.join(directory, entry.name);
      const relative = path.relative(tree, absolute).split(path.sep).join('/');
      if (entry.isDirectory()) collect(absolute);
      else {
        assert(entry.isFile(), `Unexpected runtime file type: ${relative}`);
        const data = readFileSync(absolute);
        files[relative] = digest(data);
        const executable = osName !== 'windows' && (lstatSync(absolute).mode & 0o111) !== 0;
        entries.push({ relative, data, executable });
      }
    }
  };
  collect(tree);
  const parts = [Buffer.from('DIMRT1\n')];
  for (const { relative, data, executable } of entries) {
    const name = Buffer.from(relative, 'utf8');
    const header = Buffer.alloc(4 + name.length + 1 + 8);
    header.writeUInt32BE(name.length, 0);
    name.copy(header, 4);
    header.writeUInt8(executable ? 1 : 0, 4 + name.length);
    header.writeBigUInt64BE(BigInt(data.length), 5 + name.length);
    parts.push(header, data);
  }
  const archive = gzipSync(Buffer.concat(parts), { level: 9 });
  mkdirSync(output, { recursive: true });
  writeFileSync(path.join(output, 'runtime.bin'), archive);
  const toolchain = {
    node: process.versions.node,
    rutis: JSON.parse(readFileSync(path.join(repo, 'runtime/package.json'), 'utf8')).dependencies['@arcships/rutis-runtime'],
  };
  writeFileSync(path.join(output, 'runtime.json'), `${JSON.stringify({ schemaVersion: 1, platform, toolchain, plugin, sha256: digest(archive), files }, null, 2)}\n`);
  console.log(`Staged the ${platform} account runtime (${entries.length} files, ${(archive.length / 2 ** 20).toFixed(1)} MiB).`);
} catch (error) {
  rmSync(output, { recursive: true, force: true });
  throw error;
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
