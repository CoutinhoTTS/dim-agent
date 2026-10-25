import assert from 'node:assert/strict';
import {test} from 'node:test';
import {classify,groups,selectGroups} from './ci-changes.mjs';

const only=(...names)=>Object.fromEntries(groups.map(group=>[group,names.includes(group)]));

test('documentation-only changes skip every check',()=>{
  assert.deepEqual(classify(['docs/adr/006-plugin-contract.md','docs/zh/INDEX.md','README.md','AGENTS.md']),only());
});

test('documents that code reads keep their checks',()=>{
  assert.deepEqual(classify(['docs/architecture/rust-agent/plugins/goal.md']),only('xtask'));
  assert.deepEqual(classify(['docs/design/appserver/native-storage-format.md']),only('rust'));
});

test('source areas select their groups',()=>{
  assert.deepEqual(classify(['web/src/main.ts']),only('web'));
  assert.deepEqual(classify(['scripts/stage-web.mjs']),only('scripts','web'));
  assert.deepEqual(classify(['crates/dim-app-server/src/lib.rs','runtime/fixtures/session-part.json']),only('rust'));
  assert.deepEqual(classify(['crates/dim-cli/src/main.rs']),only('rust','windows'));
});

test('workspace, toolchain, workflow and unknown paths run everything',()=>{
  for (const file of ['Cargo.lock','rust-toolchain.toml','.github/workflows/ci.yml','.node-version','dev','skills/x/SKILL.md'])
    assert.deepEqual(classify([file]),only(...groups),file);
});

test('an incomplete or capped file listing runs everything',()=>{
  const docs=count=>Array.from({length:count},(_,index)=>({filename:`docs/page-${index}.md`}));
  assert.deepEqual(selectGroups(docs(2),2),only());
  assert.deepEqual(selectGroups(docs(2),3),only(...groups));
  assert.deepEqual(selectGroups(docs(3000),3000),only(...groups));
});

test('a rename keeps the checks of its old path',()=>{
  assert.deepEqual(selectGroups([{filename:'docs/notes.md',previous_filename:'crates/dim-cli/src/notes.rs'}],1),only('rust','windows'));
});
