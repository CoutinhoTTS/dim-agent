import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, statSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { test } from 'node:test';

test('developer entry runs without Node or plugin packages and preserves terminal invocation', { skip: process.platform === 'win32' }, (t) => {
  const temporary = mkdtempSync(path.join(os.tmpdir(), 'dim dev flow '));
  t.after(() => rmSync(temporary, { recursive: true, force: true }));
  const repo = path.join(temporary, 'public source');
  const workspace = path.join(temporary, 'caller workspace');
  const bin = path.join(temporary, 'tools');
  const home = path.join(temporary, 'isolated home');
  const calls = path.join(temporary, 'native-calls.jsonl');
  for (const directory of [repo, workspace, bin, path.join(repo, 'target/debug')]) mkdirSync(directory, { recursive: true });
  cpSync(path.resolve(import.meta.dirname, '../dev'), path.join(repo, 'dev'));
  writeFileSync(path.join(repo, '.env.dev'), "DIMCODE_HOME=/unused/configured-home\nDIMCODE_PROVIDER=configured-provider\nDIMCODE_MODEL=configured-model\n");
  writeFileSync(path.join(bin, 'cargo'), '#!/bin/bash\necho "build output"\nexit 0\n', { mode: 0o755 });
  writeFileSync(path.join(bin, 'node'), '#!/bin/bash\nexit 91\n', { mode: 0o755 });
  writeFileSync(path.join(repo, 'target/debug/dimcode'), `#!${process.execPath}
const fs = require('node:fs');
const args = process.argv.slice(2);
if (args.slice(-2).join(' ') === 'daemon stop') {
  fs.appendFileSync(process.env.TEST_CALLS, JSON.stringify({stop: args})+'\\n');
} else {
  console.log(JSON.stringify({args, cwd: process.cwd(), input: fs.readFileSync(0, 'utf8')}));
  process.exit(Number(process.env.TEST_NATIVE_EXIT || 0));
}
`, { mode: 0o755 });
  const model = path.join(workspace, 'model config.json');
  const env = { ...process.env, PATH: `${bin}:/usr/bin:/bin`, DIMCODE_HOME: home, DIMCODE_MODEL_CONFIG: '', DIMCODE_PROVIDER: 'openai', DIMCODE_MODEL: 'test-model', TEST_CALLS: calls };
  const run = (args, overrides = {}) => spawnSync(path.join(repo, 'dev'), args, { cwd: workspace, env: { ...env, ...overrides }, encoding: 'utf8', input: 'piped text\n' });
  const coldRepo = path.join(temporary, 'cold source');
  mkdirSync(coldRepo);
  cpSync(path.join(repo, 'dev'), path.join(coldRepo, 'dev'));
  const cold = spawnSync(path.join(coldRepo, 'dev'), ['stop'], { env, encoding: 'utf8' });
  assert.equal(cold.status, 0, cold.stderr);
  assert.equal(cold.stdout, '');
  assert.doesNotMatch(cold.stderr, /build output/);
  assert.equal(existsSync(home), false);
  const first = run(['--model-config', model, '--prompt', 'models']);
  assert.equal(first.status, 0, first.stderr);
  const forwarded = JSON.parse(first.stdout);
  assert.deepEqual(forwarded.args, ['--home', home, '--provider', 'openai', '--model', 'test-model', '--model-config', model, '--prompt', 'models']);
  assert.equal(forwarded.cwd, realpathSync(workspace));
  assert.equal(forwarded.input, 'piped text\n');
  assert.match(first.stderr, /build output/);
  assert.equal(statSync(home).mode & 0o077, 0);
  assert.equal(existsSync(path.join(repo, '.dev')), false);
  const account = run(['login', 'dimcode-api-oauth']);
  assert.equal(account.status, 0, account.stderr);
  assert.deepEqual(JSON.parse(account.stdout).args, ['--home', home, 'login', 'dimcode-api-oauth']);
  const resumed = run(['--model-config', model, '--resume', 'session with spaces'], { TEST_NATIVE_EXIT: '23' });
  assert.equal(resumed.status, 23, resumed.stderr);
  assert.deepEqual(JSON.parse(resumed.stdout).args, ['--home', home, '--model-config', model, '--resume', 'session with spaces']);
  const stopped = run(['stop']);
  assert.equal(stopped.status, 0, stopped.stderr);
  assert.equal(stopped.stdout, '');
  const stops = readFileSync(calls, 'utf8').trim().split('\n').map((line) => JSON.parse(line));
  assert.equal(stops.length, 4);
  assert(stops.every((call) => JSON.stringify(call.stop) === JSON.stringify(['--home', home, 'daemon', 'stop'])));
});

test('development home starts without OS isolation and keeps an existing policy', { skip: process.platform === 'win32' }, (t) => {
  const temporary = mkdtempSync(path.join(os.tmpdir(), 'dim dev isolation '));
  t.after(() => rmSync(temporary, { recursive: true, force: true }));
  const repo = path.join(temporary, 'public source');
  const bin = path.join(temporary, 'tools');
  const home = path.join(temporary, 'isolated home');
  for (const directory of [repo, bin, path.join(repo, 'target/debug')]) mkdirSync(directory, { recursive: true });
  cpSync(path.resolve(import.meta.dirname, '../dev'), path.join(repo, 'dev'));
  writeFileSync(path.join(bin, 'cargo'), '#!/bin/bash\nexit 0\n', { mode: 0o755 });
  writeFileSync(path.join(repo, 'target/debug/dimcode'), '#!/bin/bash\nexit 0\n', { mode: 0o755 });
  const env = { ...process.env, PATH: `${bin}:/usr/bin:/bin`, DIMCODE_HOME: home, DIMCODE_MODEL_CONFIG: '', DIMCODE_PROVIDER: '', DIMCODE_MODEL: '' };
  const run = (...args) => spawnSync(path.join(repo, 'dev'), ['--prompt', 'hello', ...args], { env, encoding: 'utf8', input: '' });
  const product = path.join(home, 'host-isolation.json');
  const created = run();
  assert.equal(created.status, 0, created.stderr);
  assert.deepEqual(JSON.parse(readFileSync(product, 'utf8')), { version: 2, sandbox: 'host' });
  const edited = { version: 2, sandbox: 'workspace', escalation: 'ask' };
  writeFileSync(product, JSON.stringify(edited));
  const kept = run();
  assert.equal(kept.status, 0, kept.stderr);
  assert.deepEqual(JSON.parse(readFileSync(product, 'utf8')), edited);
  const configured = path.join(temporary, 'model.json');
  writeFileSync(configured, '{}');
  const configured_home = path.join(temporary, 'configured home');
  const configured_run = spawnSync(path.join(repo, 'dev'), ['--model-config', configured, '--prompt', 'hello'], {
    env: { ...env, DIMCODE_HOME: configured_home },
    encoding: 'utf8',
    input: '',
  });
  assert.equal(configured_run.status, 0, configured_run.stderr);
  assert.deepEqual(JSON.parse(readFileSync(path.join(configured_home, 'rust/host-isolation.json'), 'utf8')), { version: 2, sandbox: 'host' });
});
