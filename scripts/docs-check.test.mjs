import test from 'node:test';
import assert from 'node:assert/strict';
import {checkChanges, checkNewAdr, isFrozen, parseDiff} from './docs-check.mjs';

const adr = 'docs/adr/001-harness-foundation.md';
const goal = 'docs/design/plugins/Goal.md';
const sessionTitle = 'docs/design/plugins/SessionTitle.md';

test('parseDiff keeps rename paths', () => {
  assert.deepEqual(parseDiff('M\tdocs/INDEX.md\nR100\told.md\tnew.md'), [
    {status: 'M', paths: ['docs/INDEX.md']},
    {status: 'R100', paths: ['old.md', 'new.md']},
  ]);
});

test('an existing ADR is frozen unless it is proposed', () => {
  assert.equal(isFrozen('Decision status: Proposed.'), false);
  assert.equal(isFrozen('Decision status: Accepted by the repository owner.'), true);
  assert.equal(isFrozen('Status: implementation direction approved'), true);
  assert.equal(isFrozen('# 001: Extensible harness foundation'), true);
});

test('accepted ADRs are frozen', () => {
  for (const status of ['M', 'D', 'R100']) {
    const paths = status === 'R100' ? [adr, 'docs/adr/009-new.md'] : [adr];
    const {errors} = checkChanges([{status, paths}]);
    assert.equal(errors.length, status === 'R100' ? 2 : 1, status);
    assert.match(errors[0], /accepted ADRs are frozen/);
  }
});

test('a proposed ADR may still change', () => {
  const {errors} = checkChanges([{status: 'M', paths: [adr]}], {frozen: () => false});
  assert.deepEqual(errors, []);
});

test('the adr-erratum label allows a correction', () => {
  const {errors} = checkChanges([{status: 'M', paths: [adr]}], {erratum: true});
  assert.deepEqual(errors, []);
});

test('a new ADR is allowed', () => {
  const {errors} = checkChanges([{status: 'A', paths: ['docs/adr/008-documentation.md']}]);
  assert.deepEqual(errors, []);
});

test('two plugin designs need the cross-module label', () => {
  const changes = [
    {status: 'M', paths: [goal]},
    {status: 'M', paths: [sessionTitle]},
  ];
  assert.equal(checkChanges(changes).errors.length, 1);
  assert.deepEqual(checkChanges(changes, {crossModule: true}).errors, []);
});

test('a renamed design is one change', () => {
  const changes = [{status: 'R100', paths: ['docs/design/plugins/Old.md', goal]}];
  const {errors, designs} = checkChanges(changes);
  assert.deepEqual(errors, []);
  assert.deepEqual([...designs], [goal]);
});

test('README and Chinese mirrors do not count as designs', () => {
  const changes = [
    {status: 'M', paths: ['docs/design/plugins/README.md']},
    {status: 'M', paths: ['docs/zh/design/plugins/Goal.md']},
  ];
  const {errors, designs} = checkChanges(changes);
  assert.deepEqual(errors, []);
  assert.equal(designs.size, 0);
});

test('a new ADR follows the decision template', () => {
  const valid = [
    '# 008: Documentation',
    '',
    'Decision status: Proposed.',
    '',
    '## Context',
    '',
    '## Decision',
    '',
    '## Consequences',
    '',
  ].join('\n');
  assert.deepEqual(checkNewAdr('docs/adr/008-documentation.md', valid), []);

  const invalid = valid.replace('## Consequences', '## Implementation');
  const errors = checkNewAdr('docs/adr/008-documentation.md', invalid);
  assert.equal(errors.length, 2);
});
