import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {readFileSync} from 'node:fs';
import {dirname} from 'node:path';

const [artifacts,target,...args]=process.argv.slice(2);
assert(artifacts&&target,'Usage: node scripts/run-rust-test.mjs ARTIFACTS TARGET [TEST_ARGUMENTS...]');
const matches=readFileSync(artifacts,'utf8').split('\n').filter(Boolean).map(line=>JSON.parse(line))
  .filter(record=>record.reason==='compiler-artifact'&&record.profile.test&&record.target.name===target&&record.executable);
assert.equal(matches.length,1,`Expected one compiled test target named ${target}`);
const result=spawnSync(matches[0].executable,args,{cwd:dirname(matches[0].manifest_path),stdio:'inherit'});
assert.ifError(result.error);
process.exit(result.status??1);
