import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { cpSync, existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import path from 'node:path';

const [destination, ...extra] = process.argv.slice(2);
const update = extra.includes('--update');
const arguments_ = extra.filter((argument) => argument !== '--update');
assert(destination && arguments_.length === 0,
  'Usage: node scripts/stage-plugins.mjs DESTINATION [--update]');
const repo = path.resolve(import.meta.dirname, '..');
const output = path.resolve(destination);
const defaults = path.join(repo, 'runtime/default-plugins.json');
const { plugins } = JSON.parse(readFileSync(defaults, 'utf8'));
const npm = process.platform === 'win32' ? 'npm.cmd' : 'npm';
const registry = process.env.DIMCODE_NPM_REGISTRY ?? 'https://registry.npmjs.org';
const record = path.join(output, '.stage-plugins.json');

const copy = (source, target) => cpSync(source, target, {
  recursive: true,
  filter: (file) => {
    const base = path.basename(file);
    return base !== 'node_modules' && base !== 'tsconfig.json' && !base.endsWith('.test.mjs');
  },
});

const install = (target, name) => {
  const result = spawnSync(npm, ['ci', '--omit=dev', '--ignore-scripts', '--no-bin-links',
    '--no-audit', '--no-fund', '--registry', registry],
  { cwd: target, stdio: 'inherit', shell: process.platform === 'win32' });
  assert.ifError(result.error);
  assert.equal(result.status, 0, `npm ci failed for ${name}`);
};

/** The dependency hash of one plugin: its package files, or null without them. */
const dependencies = (source) => {
  const files = ['package.json', 'package-lock.json'];
  if (!files.every((file) => existsSync(path.join(source, file)))) return null;
  const hash = createHash('sha256');
  for (const file of files) {
    hash.update(file).update('\0').update(readFileSync(path.join(source, file))).update('\0');
  }
  return hash.digest('hex');
};

/** Puts `partial` in place of `target`, restoring `target` if the swap fails. */
const swap = (partial, target) => {
  const backup = `${target}.previous`;
  rmSync(backup, { recursive: true, force: true });
  const existed = existsSync(target);
  if (existed) renameSync(target, backup);
  try {
    renameSync(partial, target);
  } catch (error) {
    if (existed) renameSync(backup, target);
    throw error;
  }
  rmSync(backup, { recursive: true, force: true });
};

const writeAtomically = (file, content) => {
  const temporary = `${file}.tmp`;
  writeFileSync(temporary, content);
  renameSync(temporary, file);
};

/**
 * Updates an existing stage in place. The rule: every run copies every listed
 * plugin's sources again, so an edit takes effect on the next run; `npm ci`
 * runs only for a plugin whose package files hash differs from the one
 * recorded in `.stage-plugins.json`; a plugin that is no longer listed is
 * removed; and each plugin is built beside its previous copy and swapped in
 * only when complete, so every staged plugin is complete at every moment.
 * The record is rewritten after each swap, so it always describes what is on
 * disk. A failure names the plugin, exits non-zero, and leaves the plugins
 * before it updated and the rest as they were.
 */
function updateStage() {
  mkdirSync(output, { recursive: true });
  const previous = (() => {
    try {
      const data = JSON.parse(readFileSync(record, 'utf8'));
      return data.plugins && typeof data.plugins === 'object' ? data.plugins : {};
    } catch {
      return {};
    }
  })();
  const entries = { ...previous };
  const writeRecord = () => writeAtomically(record,
    `${JSON.stringify({ schemaVersion: 1, plugins: entries }, null, 2)}\n`);
  const installed = [];
  for (const [name, relative] of Object.entries(plugins)) {
    const source = path.join(path.dirname(defaults), relative);
    const target = path.join(output, relative);
    const partial = `${target}.partial`;
    const code = existsSync(path.join(source, 'package.json'));
    assert(!code || existsSync(path.join(source, 'package-lock.json')), `${name} has no package-lock.json`);
    const hash = dependencies(source);
    const before = previous[name];
    const reusable = before?.hash === hash && existsSync(target)
      && (!code || existsSync(path.join(target, 'node_modules')));
    try {
      rmSync(partial, { recursive: true, force: true });
      mkdirSync(path.dirname(target), { recursive: true });
      copy(source, partial);
      if (reusable) {
        const modules = path.join(target, 'node_modules');
        if (existsSync(modules)) renameSync(modules, path.join(partial, 'node_modules'));
      } else if (code) {
        install(partial, name);
        installed.push(name);
      }
      swap(partial, target);
    } catch (error) {
      rmSync(partial, { recursive: true, force: true });
      throw new Error(`Cannot stage ${name}: ${error.message}`);
    }
    if (before?.dir && before.dir !== relative) {
      rmSync(path.join(output, before.dir), { recursive: true, force: true });
    }
    entries[name] = { hash, dir: relative };
    writeRecord();
  }
  for (const [name, entry] of Object.entries(previous)) {
    if (name in plugins || !entry?.dir) continue;
    rmSync(path.join(output, entry.dir), { recursive: true, force: true });
    delete entries[name];
  }
  writeRecord();
  const defaultsCopy = path.join(output, 'default-plugins.json');
  copy(defaults, `${defaultsCopy}.tmp`);
  renameSync(`${defaultsCopy}.tmp`, defaultsCopy);
  console.log(`Updated ${Object.keys(plugins).length} default plugins (npm ci: ${installed.length ? installed.join(', ') : 'none'}).`);
}

if (update) {
  updateStage();
} else {
  assert(!existsSync(output), 'Destination already exists');
  try {
    mkdirSync(output, { recursive: true });
    cpSync(defaults, path.join(output, 'default-plugins.json'));
    for (const [name, relative] of Object.entries(plugins)) {
      const source = path.join(path.dirname(defaults), relative);
      const target = path.join(output, relative);
      const code = existsSync(path.join(source, 'package.json'));
      assert(!code || existsSync(path.join(source, 'package-lock.json')), `${name} has no package-lock.json`);
      copy(source, target);
      if (!code) continue;
      install(target, name);
    }
    console.log(`Staged ${Object.keys(plugins).length} default plugins.`);
  } catch (error) {
    rmSync(output, { recursive: true, force: true });
    throw error;
  }
}
