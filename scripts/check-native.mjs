import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {lstatSync,readFileSync} from 'node:fs';
import path from 'node:path';
const [directory,version,sourceCommit,receiptPath,buildRunId,...extra]=process.argv.slice(2);
assert(directory&&version&&sourceCommit&&extra.length===0&&Boolean(receiptPath)===Boolean(buildRunId),'Usage: node scripts/check-native.mjs DIRECTORY VERSION SOURCE_COMMIT [NPM_RECEIPT BUILD_RUN_ID]');
assert(/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$/.test(version),'Invalid version');
assert(/^[a-f0-9]{40}$/.test(sourceCommit),'Invalid source commit');
const manifest=JSON.parse(readFileSync(path.join(directory,'release-manifest.json')));
assert.equal(manifest.version,version,'Release version mismatch');
assert.equal(manifest.sourceCommit,sourceCommit,'Release source mismatch');
if(receiptPath) {
  const receipt=JSON.parse(readFileSync(receiptPath));
  assert.equal(receipt.buildRunId,buildRunId,'npm consumed a different release build');
  assert.equal(receipt.sourceCommit,sourceCommit,'npm source mismatch');
  assert.equal(receipt.version,version,'npm version mismatch');
  assert.deepEqual(receipt.npmPackages,manifest.npmPackages,'npm package provenance mismatch');
}
const platforms=['darwin-arm64','darwin-x64','linux-arm64','linux-x64','windows-x64'];
assert.deepEqual(manifest.nativeArchives.map(({platform})=>platform).sort(),platforms,'Missing native platforms');
for (const {platform,file,sha256} of manifest.nativeArchives) {
  assert.equal(file,`dimcode-${platform}-${version}.${platform==='windows-x64'?'zip':'tar.gz'}`,'Invalid archive name');
  const archive=path.join(directory,file);
  assert(lstatSync(archive).isFile(),'Native archive must be a regular file');
  assert.equal(createHash('sha256').update(readFileSync(archive)).digest('hex'),sha256,'Native archive digest mismatch');
}
console.log(`Validated five native archives for ${version}.`);
