// Local multi-platform build for hands-on testing, the counterpart of the
// TypeScript CLI's `build:all`: cross-compiles `dimcode` and `dim-app-server`
// (with the Web workspace embedded) from this machine into
// dist/<platform>/. Releases are built per target by .github/workflows/release.yml.
//
//   node scripts/build-all.mjs [--target PLATFORM]... [--debug]
//
// macOS targets build with cargo; Linux and Windows targets with
// cargo-zigbuild (`cargo install --locked cargo-zigbuild`, Zig on PATH).
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { chmodSync, copyFileSync, mkdirSync, rmSync } from 'node:fs';
import path from 'node:path';

const targets = {
  'darwin-arm64': { triple: 'aarch64-apple-darwin', zig: false },
  'darwin-x64': { triple: 'x86_64-apple-darwin', zig: false },
  // Linked against glibc 2.28 so the binaries run on older distributions.
  'linux-x64': { triple: 'x86_64-unknown-linux-gnu', zig: true, glibc: '2.28' },
  'linux-arm64': { triple: 'aarch64-unknown-linux-gnu', zig: true, glibc: '2.28' },
  'windows-x64': { triple: 'x86_64-pc-windows-gnu', zig: true },
};

const args = process.argv.slice(2);
const chosen = [];
let profile = 'release';
for (let index = 0; index < args.length; index += 1) {
  if (args[index] === '--target') chosen.push(args[++index]);
  else if (args[index] === '--debug') profile = 'debug';
  else assert.fail(`Usage: node scripts/build-all.mjs [--target ${Object.keys(targets).join('|')}]... [--debug]`);
}
for (const name of chosen) assert(name in targets, `Unknown target ${name}; use one of ${Object.keys(targets).join(', ')}`);
const platforms = chosen.length > 0 ? chosen : Object.keys(targets);
const repo = path.resolve(import.meta.dirname, '..');

const failed = [];
for (const platform of platforms) {
  const { triple, zig, glibc } = targets[platform];
  console.log(`\n▸ ${platform} (${triple})`);
  const command = zig ? 'zigbuild' : 'build';
  const result = spawnSync('cargo', [
    command, '--locked', ...(profile === 'release' ? ['--release'] : []),
    '--target', glibc ? `${triple}.${glibc}` : triple,
    '-p', 'dim-cli', '-p', 'dim-app-server',
  ], { cwd: repo, stdio: 'inherit' });
  if (result.status !== 0) {
    failed.push(platform);
    continue;
  }
  const suffix = platform.startsWith('windows-') ? '.exe' : '';
  const output = path.join(repo, 'dist', platform);
  rmSync(output, { recursive: true, force: true });
  mkdirSync(output, { recursive: true });
  // The CLI starts the App Server found next to itself.
  for (const binary of ['dimcode', 'dim-app-server']) {
    const file = path.join(output, `${binary}${suffix}`);
    copyFileSync(path.join(repo, 'target', triple, profile, `${binary}${suffix}`), file);
    chmodSync(file, 0o755);
  }
  console.log(`✓ dist/${platform}/dimcode${suffix}, dist/${platform}/dim-app-server${suffix}`);
}
if (failed.length > 0) {
  console.error(`\nFailed: ${failed.join(', ')}`);
  process.exit(1);
}
console.log(`\nBuilt ${platforms.join(', ')} (${profile}) into dist/.`);
