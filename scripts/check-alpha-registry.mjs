import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

const [receipt, mode, flag, ...extra] = process.argv.slice(2);
assert(receipt && ['source', 'snapshot', 'promote', 'npm', 'mirror'].includes(mode)
  && [undefined, '--apply', '--dry-run'].includes(flag) && extra.length === 0,
  'Usage: node scripts/check-alpha-registry.mjs RECEIPT source|snapshot|promote|npm|mirror [--apply|--dry-run]');
assert(flag !== '--apply' || ['promote', 'mirror'].includes(mode), 'Only promotion and mirror sync write remote state');
const state = JSON.parse(readFileSync(receipt));
const wrapper = 'dimcode-alpha';
const names = [...Object.keys(JSON.parse(readFileSync(new URL('../npm/package.json', import.meta.url))).optionalDependencies), wrapper];
const upstream = 'https://registry.npmjs.org';
const registry = mode === 'mirror' ? 'https://registry.npmmirror.com' : 'https://registry.npmjs.org';
const alpha = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)-alpha\.(0|[1-9]\d*)$/;

async function json(url) {
  const response = await fetch(url, { signal: AbortSignal.timeout(30_000) });
  assert(response.ok, `${url}: HTTP ${response.status}`);
  return response.json();
}

async function tags(name) {
  return json(`${upstream}/-/package/${name}/dist-tags`);
}

const protectedTags = (name, values) => Object.fromEntries(Object.entries(values)
  .filter(([tag]) => name === 'dimcode' || (name === wrapper ? !['latest', 'alpha'].includes(tag) : tag !== 'alpha')));

async function snapshot() {
  const all = Object.fromEntries(await Promise.all(['dimcode', ...names].map(async name => [name, await tags(name)])));
  state.protectedTags = Object.fromEntries(Object.entries(all).map(([name, values]) => [name, protectedTags(name, values)]));
  state.wrapperTags = all[wrapper];
  console.log(`Recorded ${wrapper} tags: ${JSON.stringify(state.wrapperTags)}`);
}

async function checkProtectedTags() {
  assert(state.protectedTags, 'Missing pre-publication dist-tag snapshot');
  for (const [name, expected] of Object.entries(state.protectedTags)) {
    assert.deepEqual(protectedTags(name, await tags(name)), expected, `Protected dist-tags changed: ${name}`);
  }
}

function checkWrapper(metadata, version) {
  assert.equal(metadata.name, wrapper);
  assert.equal(metadata.version, version);
  assert.deepEqual(Object.keys(metadata.bin), ['dima']);
  assert.deepEqual(metadata.optionalDependencies, Object.fromEntries(names.filter(name => name !== wrapper).map(name => [name, version])));
}

async function provenance(name, version, sourceCommit) {
  const response = await fetch(`${upstream}/${name}/${version}`, { signal: AbortSignal.timeout(30_000) });
  if (response.status === 404) return false;
  assert(response.ok, `${name}@${version}: HTTP ${response.status}`);
  const metadata = await response.json();
  assert.equal(metadata.name, name);
  assert.equal(metadata.version, version);
  if (name === wrapper) {
    checkWrapper(metadata, version);
    return true;
  }
  assert(metadata.dist.tarball.startsWith(`${upstream}/${name}/-/`), `Unexpected tarball URL: ${name}`);
  const archive = await fetch(metadata.dist.tarball, { signal: AbortSignal.timeout(120_000) });
  assert(archive.ok, `${metadata.dist.tarball}: HTTP ${archive.status}`);
  const bytes = Buffer.from(await archive.arrayBuffer());
  assert.equal(`sha512-${createHash('sha512').update(bytes).digest('base64')}`, metadata.dist.integrity, `Tarball integrity mismatch: ${name}`);
  const unpacked = spawnSync('tar', ['-xzO', 'package/release-manifest.json'], { input: bytes, maxBuffer: 16 * 1024 * 1024 });
  assert.ifError(unpacked.error);
  assert.equal(unpacked.status, 0, `Cannot read release manifest: ${name}`);
  const manifest = JSON.parse(unpacked.stdout);
  assert.equal(manifest.version, version);
  assert.equal(manifest.platform, name.slice('dimcode-'.length));
  assert(/^[a-f0-9]{40}$/.test(manifest.sourceCommit), `Invalid source commit: ${name}`);
  if (name !== 'dimcode-web') assert.equal(manifest.account, true, `Missing Dim account build: ${name}`);
  return manifest.sourceCommit === sourceCommit;
}

async function complete(version, sourceCommit) {
  assert(alpha.test(version), `Expected X.Y.Z-alpha.N: ${version}`);
  assert(/^[a-f0-9]{40}$/.test(sourceCommit), 'Invalid source commit');
  for (const name of names) {
    if (!await provenance(name, version, sourceCommit)) return false;
    console.log(`Verified provenance: ${name}@${version} from ${sourceCommit}`);
  }
  return true;
}

function compare(left, right) {
  assert(alpha.test(left) && alpha.test(right), `Unexpected nightly version: ${left}, ${right}`);
  const a = left.match(alpha).slice(1).map(BigInt);
  const b = right.match(alpha).slice(1).map(BigInt);
  const index = a.findIndex((part, index) => part !== b[index]);
  return index === -1 ? 0 : a[index] < b[index] ? -1 : 1;
}

function command(args) {
  console.log(`$ npm ${args.join(' ')}`);
  const result = spawnSync('npm', args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], timeout: 120_000 });
  assert.ifError(result.error);
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
}

if (mode === 'snapshot' || mode === 'source') {
  await snapshot();
  if (mode === 'source') {
    assert(/^[a-f0-9]{40}$/.test(state.sourceCommit), 'Invalid source commit');
    const candidates = [...new Set(['latest', 'alpha'].map(tag => state.wrapperTags[tag]).filter(Boolean))].sort(compare).reverse();
    state.version = null;
    for (const version of candidates) {
      if (await complete(version, state.sourceCommit)) {
        state.version = version;
        break;
      }
    }
    console.log(state.version ? `Reuse ${state.version}; repair and acceptance are required before skipping the build.` : 'No complete nightly for this source; a new build is required.');
  }
  writeFileSync(receipt, `${JSON.stringify(state, null, 2)}\n`);
} else if (mode === 'promote') {
  const deadline = Date.now() + (flag === '--dry-run' ? 0 : 600_000);
  while (!await complete(state.version, state.sourceCommit)) {
    assert(Date.now() < deadline, 'Nightly is incomplete or its source does not match');
    await delay(Math.min(15_000, deadline - Date.now()));
  }
  await checkProtectedTags();
  const expected = await tags(wrapper);
  for (const tag of ['latest', 'alpha']) {
    if (expected[tag] !== undefined) assert(compare(expected[tag], state.version) <= 0, `${tag} already points to newer ${expected[tag]}`);
    assert(expected[tag] === state.wrapperTags[tag] || expected[tag] === state.version, `Concurrent publication changed ${tag}`);
  }
  for (const tag of ['alpha', 'latest']) {
    if (expected[tag] === state.version) continue;
    assert.deepEqual(await tags(wrapper), expected, 'Concurrent publication changed wrapper tags');
    if (flag === '--apply') {
      command(['dist-tag', 'add', `${wrapper}@${state.version}`, tag, '--registry', upstream]);
      expected[tag] = state.version;
      assert.deepEqual(await tags(wrapper), expected, 'Concurrent publication changed wrapper tags');
    } else {
      console.log(`Would set ${wrapper}@${state.version} ${tag}; no registry write performed.`);
    }
  }
  await checkProtectedTags();
} else {
  const { version } = state;
  assert(alpha.test(version), 'Expected X.Y.Z-alpha.N');
  if (mode === 'mirror' && flag === '--apply') {
    for (const name of names) {
      const response = await fetch(`${registry}/-/package/${name}/syncs`, {
        method: 'PUT',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ skipDependencies: true }),
        signal: AbortSignal.timeout(30_000),
      });
      assert(response.ok && (await response.json()).ok, `Mirror sync failed for ${name} (HTTP ${response.status})`);
    }
  }
  const refs = names.flatMap(name => name === wrapper ? ['latest', 'alpha'].map(tag => [name, tag])
    : (mode === 'npm' ? [[name, version], [name, 'alpha']] : [[name, version]]));
  const pending = new Set(refs);
  const deadline = Date.now() + (flag === '--dry-run' ? 0 : 600_000);
  while (pending.size) {
    await Promise.all([...pending].map(async ref => {
      const [name, tag] = ref;
      const url = `${registry}/${name}/${tag}`;
      let response;
      try {
        response = await fetch(url, { signal: AbortSignal.timeout(15_000) });
      } catch (error) {
        console.error(`${url}: ${error.message}`);
        return;
      }
      if (response.status === 404 || response.status === 429 || response.status >= 500) return;
      assert(response.ok, `${url}: HTTP ${response.status}`);
      const metadata = await response.json();
      if (metadata.version !== version) return;
      assert.equal(metadata.name, name);
      if (name === wrapper) checkWrapper(metadata, version);
      pending.delete(ref);
      console.log(`Verified ${name}@${tag} = ${version} on ${registry}`);
    }));
    if (pending.size) {
      assert(Date.now() < deadline, `Visibility incomplete for ${version} on ${registry}: ${[...pending].map(ref => ref.join('@')).join(', ')}`);
      await delay(Math.min(15_000, deadline - Date.now()));
    }
  }
  await checkProtectedTags();
  for (const spec of [wrapper, `${wrapper}@alpha`]) {
    const temporary = mkdtempSync(path.join(os.tmpdir(), 'dim-nightly-install-'));
    try {
      const prefix = path.join(temporary, 'prefix');
      command(['install', '--global', '--prefix', prefix, '--cache', path.join(temporary, 'cache'), '--include=optional',
        '--ignore-scripts', '--no-audit', '--no-fund', '--registry', registry, spec]);
      const metadata = JSON.parse(readFileSync(path.join(prefix, 'lib/node_modules', wrapper, 'package.json')));
      checkWrapper(metadata, version);
      const binary = spawnSync(path.join(prefix, 'bin/dima'), ['--version'], {
        encoding: 'utf8', timeout: 30_000, env: { ...process.env, DIMCODE_HOME: path.join(temporary, 'home') },
      });
      assert.ifError(binary.error);
      assert.equal(binary.status, 0, binary.stderr);
      assert.equal(binary.stdout.trim(), `dimcode ${version}`);
      console.log(`Installed ${spec} from ${registry}: ${binary.stdout.trim()}`);
    } finally {
      rmSync(temporary, { recursive: true, force: true });
    }
  }
  const finalTags = await json(`${registry}/-/package/${wrapper}/dist-tags`);
  for (const tag of ['latest', 'alpha']) assert.equal(finalTags[tag], version, `${tag} changed during installation`);
  await checkProtectedTags();
}
