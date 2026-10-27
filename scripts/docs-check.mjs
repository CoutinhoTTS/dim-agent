// Checks the document boundaries that need a PR diff:
// - an existing ADR is a snapshot; only a new ADR may be added
// - a PR changes at most one plugin design unless it carries the cross-module label
// - a new ADR uses the decision template and carries no implementation status
//
// Run from the workspace root in a PR checkout:
//   node scripts/docs-check.mjs --base HEAD^1
// Locally, pass origin/main or another base.
import {execFileSync} from 'node:child_process';
import {readFileSync} from 'node:fs';

const adr = /^docs\/adr\/(?!README\.md)[^/]+\.md$/;
const pluginDesign = /^docs\/design\/plugins\/(?!README\.md)[^/]+\.md$/;

export function parseDiff(text) {
  return text
    .split('\n')
    .filter(Boolean)
    .map(line => {
      const [status, ...paths] = line.split('\t');
      return {status, paths};
    });
}

export function checkChanges(changes, {crossModule = false} = {}) {
  const errors = [];
  const designs = new Set();
  for (const {status, paths} of changes) {
    for (const path of paths) {
      if (adr.test(path) && status !== 'A') {
        errors.push(`${path}: existing ADRs are frozen; add a new ADR instead of changing this one`);
      }
    }
    // A rename is one design change; count the new path only.
    const moved = status.startsWith('R') || status.startsWith('C');
    for (const path of moved ? [paths.at(-1)] : paths) {
      if (pluginDesign.test(path)) designs.add(path);
    }
  }
  if (designs.size > 1 && !crossModule) {
    errors.push(
      `this PR changes ${designs.size} plugin designs (${[...designs].join(', ')}); ` +
        'split it or add the cross-module label and review from the affected owners',
    );
  }
  return {errors, designs};
}

export function checkNewAdr(path, text) {
  const errors = [];
  for (const [name, pattern] of [
    ['Decision status', /^Decision status:/m],
    ['Context', /^## Context\b/m],
    ['Decision', /^## Decisions?\b/m],
    ['Consequences', /^## Consequences\b/m],
  ]) {
    if (!pattern.test(text)) errors.push(`${path}: new ADR is missing ${name}`);
  }
  if (/^## (?:Implementation|Delivery)\b/m.test(text) || /^Implementation:/m.test(text)) {
    errors.push(`${path}: an ADR records a decision, not implementation or delivery status`);
  }
  return errors;
}

function git(args) {
  return execFileSync('git', args, {encoding: 'utf8'});
}

function option(args, name) {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : undefined;
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  const base = option(args, '--base') || process.env.DIM_DOCS_BASE || 'HEAD^1';
  const crossModule = args.includes('--cross-module') || process.env.DIM_DOCS_CROSS_MODULE === '1';
  const changes = parseDiff(git(['diff', '--name-status', '--find-renames', base]));
  const {errors, designs} = checkChanges(changes, {crossModule});
  for (const {status, paths} of changes) {
    if (status !== 'A') continue;
    for (const path of paths) {
      if (adr.test(path)) errors.push(...checkNewAdr(path, readFileSync(path, 'utf8')));
    }
  }
  if (errors.length > 0) {
    console.error(errors.join('\n'));
    process.exitCode = 1;
  } else {
    console.log(
      `documentation boundaries passed: ${changes.length} changed files, ${designs.size} plugin design(s)`,
    );
  }
}
