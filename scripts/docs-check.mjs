// Checks that an accepted ADR is not edited or deleted in a PR.
// A link or typo fix uses the adr-erratum label.
//
// Run from the workspace root in a PR checkout:
//   node scripts/docs-check.mjs --base HEAD^1
// Locally, pass origin/main or another base.
import {execFileSync} from 'node:child_process';

const adr = /^docs\/adr\/(?!README\.md)[^/]+\.md$/;

export function parseDiff(text) {
  return text
    .split('\n')
    .filter(Boolean)
    .map(line => {
      const [status, ...paths] = line.split('\t');
      return {status, paths};
    });
}

export function isFrozen(text) {
  // An existing ADR is frozen unless it says it is still proposed.
  return !/^Decision status:\s*Proposed\b/m.test(text);
}

export function checkChanges(changes, {frozen = () => true, erratum = false} = {}) {
  const errors = [];
  for (const {status, paths} of changes) {
    for (const path of paths) {
      if (adr.test(path) && status !== 'A' && frozen(path) && !erratum) {
        errors.push(
          `${path}: accepted ADRs are frozen; add a new ADR, or add the adr-erratum label and push again for a link or typo fix`,
        );
      }
    }
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
  const erratum = args.includes('--adr-erratum') || process.env.DIM_DOCS_ADR_ERRATUM === '1';
  const mergeBase = git(['merge-base', base, 'HEAD']).trim();
  const changes = parseDiff(git(['diff', '--name-status', '--find-renames', mergeBase]));
  const frozen = path => {
    try {
      return isFrozen(git(['show', `${mergeBase}:${path}`]));
    } catch {
      return false;
    }
  };
  const errors = checkChanges(changes, {frozen, erratum});
  if (errors.length > 0) {
    console.error(errors.join('\n'));
    process.exitCode = 1;
  } else {
    console.log(`documentation boundaries passed: ${changes.length} changed files`);
  }
}
