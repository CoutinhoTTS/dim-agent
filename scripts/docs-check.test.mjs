import test from 'node:test';
import assert from 'node:assert/strict';
import {checkChanges, isFrozen, parseDiff} from './docs-check.mjs';

const adr = 'docs/adr/001-harness-foundation.md';

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
    const errors = checkChanges([{status, paths}]);
    assert.equal(errors.length, status === 'R100' ? 2 : 1, status);
    assert.match(errors[0], /accepted ADRs are frozen/);
  }
});

test('a proposed ADR may still change', () => {
  const errors = checkChanges([{status: 'M', paths: [adr]}], {frozen: () => false});
  assert.deepEqual(errors, []);
});

test('the adr-erratum label allows a correction', () => {
  const errors = checkChanges([{status: 'M', paths: [adr]}], {erratum: true});
  assert.deepEqual(errors, []);
});

test('a new ADR is allowed', () => {
  const errors = checkChanges([{status: 'A', paths: ['docs/adr/009-documentation.md']}]);
  assert.deepEqual(errors, []);
});
