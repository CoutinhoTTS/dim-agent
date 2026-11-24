import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';

test('rolling nightly installs only complete releases and retains the three highest versions', { skip: process.platform === 'win32' }, t => {
  const root = mkdtempSync(path.join(tmpdir(), 'dim-nightly-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const file = (...parts) => path.join(root, ...parts);
  const env = { ...process.env, HOME: root, XDG_DATA_HOME: file('data'), PATH: `${root}:${process.env.PATH}` };
  const assets = file('assets');
  mkdirSync(assets);
  mkdirSync(file('payload/bin'), { recursive: true });
  writeFileSync(file('gh'), `#!/bin/sh
set -eu
case "$1 $2" in
  'api --paginate') case "$3" in */assets) ls "$HOME/assets";; */releases) if [ -f "$HOME/release" ]; then cat "$HOME/release"; fi;; *) exit 2;; esac;;
  'api repos/{owner}/{repo}/releases/tags/Nightly') echo 1;;
  'release create') test "$3" = Nightly && echo '{"id":1,"prerelease":true,"draft":true,"tag_name":"Nightly"}' >"$HOME/release";;
  'release upload') test "$3" = Nightly; shift 3; for f do cp "$f" "$HOME/assets/"; if [ -f "$HOME/interrupt" ]; then exit 1; fi; done;;
  'release edit') test "$3" = Nightly && jq --arg name "$5" '.name=$name | .draft=false' "$HOME/release" >"$HOME/next" && mv "$HOME/next" "$HOME/release";;
  'release delete-asset') test "$3" = Nightly && rm "$HOME/assets/$4";;
  *) exit 2;;
esac\n`, { mode: 0o755 });
  writeFileSync(file('curl'), `#!/bin/sh
set -eu
case "$2" in
  https://api.github.com/repos/arcships/dim/releases/tags/Nightly) cp "$HOME/release" "$4";;
  https://github.com/arcships/dim/releases/download/Nightly/*) cp "$HOME/assets/\${2##*/}" "$4";;
  *) exit 2;;
esac\n`, { mode: 0o755 });
  const platforms = ['linux-arm64', 'linux-x64', 'darwin-arm64', 'darwin-x64'];
  function publish(n, ok = true) {
    const v = `1.2.3-nightly.${n}`, dir = file(v);
    mkdirSync(dir);
    writeFileSync(file('payload/bin/dimcode'), `#!/bin/sh\necho 'dimcode ${v}'\n`, { mode: 0o755 });
    for (const p of platforms) execFileSync('tar', ['-czf', path.join(dir, `dimcode-${p}-${v}.tar.gz`), '-C', file('payload'), '.']);
    const run = spawnSync(process.execPath, [path.join(import.meta.dirname, 'publish-nightly.mjs'), dir, `v${v}`, 'a'.repeat(40)], { env, encoding: 'utf8' });
    assert.equal(run.status === 0, ok, run.stderr);
  }
  const install = () => spawnSync('sh', [path.join(import.meta.dirname, 'install-nightly.sh')], { env, encoding: 'utf8' });
  const installed = () => execFileSync(file('.local/bin/dim-nightly'), ['--version'], { env, encoding: 'utf8' }).trim();
  const release = () => JSON.parse(readFileSync(file('release'))).name;
  publish(8);
  assert.equal(install().status, 0);
  assert.equal(installed(), 'dimcode 1.2.3-nightly.8');
  writeFileSync(file('.local/bin/dima'), 'npm installation');
  writeFileSync(file('assets/unrelated.txt'), 'keep');
  writeFileSync(file('interrupt'), '');
  publish(12, false);
  assert.equal(release(), 'Nightly v1.2.3-nightly.8');
  rmSync(file('interrupt'));
  publish(9);
  publish(11);
  publish(10);
  assert.equal(release(), 'Nightly v1.2.3-nightly.11');
  const expected = [9, 10, 11].flatMap(n => [...platforms.map(p => `dimcode-${p}-1.2.3-nightly.${n}.tar.gz`), `SHA256SUMS-1.2.3-nightly.${n}`]);
  assert.deepEqual(readdirSync(assets).sort(), [...expected, 'unrelated.txt'].sort());
  const tar = file('assets', `dimcode-${process.platform}-${process.arch}-1.2.3-nightly.11.tar.gz`);
  const bytes = readFileSync(tar);
  writeFileSync(tar, 'corrupted');
  assert.match(install().stderr, /Checksum mismatch/);
  assert.equal(installed(), 'dimcode 1.2.3-nightly.8');
  writeFileSync(tar, bytes);
  for (let i = 0; i < 2; i++) {
    const run = install();
    assert.equal(run.status, 0, run.stderr);
    assert.equal(installed(), 'dimcode 1.2.3-nightly.11');
  }
  assert.equal(readFileSync(file('.local/bin/dima'), 'utf8'), 'npm installation');
});
