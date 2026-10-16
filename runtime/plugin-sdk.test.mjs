import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { test } from 'node:test';

const root = path.resolve(import.meta.dirname, '..');
test('packed plugin service types compile the author guide and enforce the public call shapes', (t) => {
  const directory = mkdtempSync(path.join(os.tmpdir(), 'dim plugin sdk '));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const npmCli = path.join(path.dirname(process.execPath), process.platform === 'win32'
    ? 'node_modules/npm/bin/npm-cli.js' : '../lib/node_modules/npm/bin/npm-cli.js');
  const packed = spawnSync(process.execPath, [npmCli, 'pack', path.join(root, 'npm'),
    '--pack-destination', directory, '--offline', '--ignore-scripts', '--json'], { encoding: 'utf8' });
  assert.equal(packed.status, 0, packed.stderr);
  const archive = path.join(directory, JSON.parse(packed.stdout)[0].filename);
  const packageRoot = path.join(directory, 'node_modules/dimcode');
  mkdirSync(packageRoot, { recursive: true });
  const extracted = spawnSync('tar', ['-xzf', archive, '--strip-components=1', '-C', packageRoot], { encoding: 'utf8' });
  assert.equal(extracted.status, 0, extracted.stderr);
  cpSync(path.join(root, 'runtime/node_modules/@arcships/rutis'),
    path.join(directory, 'node_modules/@arcships/rutis'), { recursive: true });
  const guide = readFileSync(path.join(root, 'docs/typescript-plugin-authoring.md'), 'utf8');
  const examples = [...guide.matchAll(/```ts\n([\s\S]*?)```/g)];
  assert(examples.length > 0);
  const files = examples.map(([, source], index) => {
    const file = path.join(directory, `guide-${index}.mts`);
    writeFileSync(file, source);
    return file;
  });
  const calls = path.join(directory, 'calls.mts');
  writeFileSync(calls, `
import type { Sessions, FileScope, FileResult } from 'dimcode/plugin';
declare const sessions: Sessions;
declare const files: FileScope;
const at = { sessionId: 'session', branchId: 'main' };
void sessions.launch({ cwd: '/workspace', policy: { preset: 'read-only' } });
void sessions.input({ ...at, submissionId: 'notebook:unique', mode: 'queue', rawBlocks: [{ text: 'notebook' }], content: [] });
void sessions.lookup({ ...at, submissionId: 'notebook:unique' });
void sessions.history({ ...at, atLeast: 1, limit: 10 });
void sessions.withdraw({ ...at, inputId: 'input' });
void sessions.stop(at);
void sessions.idle(at);
void sessions.delete({ sessionId: at.sessionId });
const quiesced: null = files.quiesce();
const closed: Promise<null> = files.close();
const transaction: Promise<FileResult<FileResult<string | null>>> = files.withLock('notebook/state', file => file.read());
void quiesced; void closed; void transaction;
// @ts-expect-error branch identity is required.
void sessions.idle({ sessionId: 'session' });
// @ts-expect-error input mode is part of the wire contract.
void sessions.input({ ...at, submissionId: 'id', mode: 'replace', rawBlocks: [], content: [] });
// @ts-expect-error policy field spelling follows PermissionPolicy.
void sessions.launch({ cwd: '/workspace', policy: { allowed_tools: [] } });
// @ts-expect-error lease replacement accepts text, not a host storage envelope.
void files.withLock('state', file => file.replace({ version: 1, text: 'state' }));
`);
  const compiler = path.join(root, 'runtime/node_modules/typescript/bin/tsc');
  const declarations = spawnSync(process.execPath, [compiler,
    '--strict', '--noEmit', '--target', 'ES2022', '--module', 'NodeNext', '--moduleResolution', 'NodeNext',
    path.join(packageRoot, 'plugin.d.ts')], { encoding: 'utf8' });
  assert.equal(declarations.status, 0, declarations.stdout + declarations.stderr);
  // rutis 0.10 has an optional/required Plugin inheritance conflict; check our declarations separately.
  const checked = spawnSync(process.execPath, [compiler, '--skipLibCheck',
    '--strict', '--noEmit', '--target', 'ES2022', '--module', 'NodeNext', '--moduleResolution', 'NodeNext',
    ...files, calls], { encoding: 'utf8' });
  assert.equal(checked.status, 0, checked.stdout + checked.stderr);
});
