import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';

const [destination, ...extra] = process.argv.slice(2);
assert(destination && extra.length === 0, 'Usage: node scripts/rust-notices.mjs NEW_OUTPUT.txt');
assert(!existsSync(destination), 'Destination already exists');
const repo = path.resolve(import.meta.dirname, '..');
const tool = spawnSync('cargo-about', ['--version'], { encoding: 'utf8' });
assert.ifError(tool.error);
assert.equal(tool.status, 0, tool.stderr);
assert.equal(tool.stdout.trim(), 'cargo-about 0.9.3', 'Install cargo-about 0.9.3 for release notices');
const result = spawnSync('cargo-about', ['generate', '--workspace', '--all-features', '--locked', '--fail',
  '--config', 'scripts/release-licenses.toml', '--format', 'json'],
{ cwd: repo, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });
assert.ifError(result.error);
assert.equal(result.status, 0, result.stderr);
const { licenses, crates } = JSON.parse(result.stdout);
assert(licenses.length > 0 && crates.length > 0, 'No Rust dependency licenses generated');
const sections = ['Dim Rust dependency notices\nGenerated from Cargo.lock by cargo-about 0.9.3.\n' +
  'Includes all workspace features and target dependencies, excluding dev-only dependencies.\n'];
for (const { id, text, used_by } of licenses) {
  sections.push(`${id}\n${used_by.map(({ crate: entry }) => `${entry.name} ${entry.version}`).sort().join('\n')}\n\n${text}`);
}
for (const { package: entry } of crates) {
  const directory = path.dirname(entry.manifest_path);
  const files = readdirSync(directory, { recursive: true, withFileTypes: true })
    .filter(file => file.isFile() && /^(licen[cs]e|copying|copyright|notice)(?:[._-].*)?$/i.test(file.name))
    .map(file => path.relative(directory, path.join(file.parentPath, file.name))).sort();
  for (const file of files) {
    sections.push(`${entry.name} ${entry.version} — ${file.split(path.sep).join('/')}\n\n` +
      readFileSync(path.join(directory, file), 'utf8'));
  }
}
writeFileSync(destination, `${sections.join('\n\n' + '='.repeat(72) + '\n\n')}\n`, { flag: 'wx' });
console.log(`Wrote Rust notices for ${crates.length} crates to ${destination}.`);
