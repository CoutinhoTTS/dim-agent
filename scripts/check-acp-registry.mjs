import assert from 'node:assert/strict';
import {execFileSync,spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {cpSync,createReadStream,mkdirSync,mkdtempSync,readFileSync,rmSync,writeFileSync} from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {pathToFileURL} from 'node:url';

const [executable,upstream,destination]=process.argv.slice(2);
assert(executable&&upstream&&destination,'Usage: node scripts/check-acp-registry.mjs EXECUTABLE REGISTRY_CHECKOUT REPORT_DIRECTORY');
const binary=path.resolve(executable);
const registry=path.resolve(upstream);
const output=path.resolve(destination);
const platform={darwin:'darwin',linux:'linux'}[process.platform];
const architecture={arm64:'aarch64',x64:'x86_64'}[process.arch];
assert(platform&&architecture,'Registry acceptance requires macOS or Linux on arm64 or x64');
mkdirSync(output,{recursive:true});
const temporary=mkdtempSync(path.join(os.tmpdir(),'dim-registry-'));
const home=path.join(temporary,'home');
const checkout=path.join(temporary,'registry');
const run=(program,args)=>execFileSync(program,args,{encoding:'utf8',timeout:120_000,maxBuffer:10*1024*1024});
try {
  const version=run(binary,['--version']).trim().match(/^dimcode (\S+)$/)?.[1];
  assert(version,'The executable must report its native Dim version');
  const revision=run('git',['-C',registry,'rev-parse','HEAD']).trim();
  cpSync(path.join(registry,'.github'),path.join(checkout,'.github'),{recursive:true});
  mkdirSync(path.join(checkout,'dimcode'),{recursive:true});
  const archive=path.join(temporary,'dimcode.tar');
  run('tar',['-cf',archive,'-C',path.dirname(binary),'--',path.basename(binary)]);
  const digest=createHash('sha256');
  for await (const chunk of createReadStream(archive)) digest.update(chunk);
  writeFileSync(path.join(checkout,'dimcode/agent.json'),JSON.stringify({
    id:'dimcode',name:'Dim',version,
    distribution:{binary:{[`${platform}-${architecture}`]:{
      archive:pathToFileURL(archive).href,sha256:digest.digest('hex'),cmd:`./${path.basename(binary)}`,
      args:['acp','--home',home],env:{DIMCODE_OAUTH_MANUAL:'1'},
    }}},
  },null,2)+'\n');
  const scripts=path.join(checkout,'.github/workflows');
  const auth=spawnSync('uv',['run','--no-project','--python','3.14.2',path.join(scripts,'verify_agents.py'),
    '--auth-check','--agent','dimcode','--auth-timeout','30'],
  {encoding:'utf8',timeout:120_000,maxBuffer:10*1024*1024});
  const authLog=(auth.stdout??'')+(auth.stderr??'');
  process.stdout.write(authLog);
  writeFileSync(path.join(output,'auth.log'),authLog);
  assert.equal(auth.status,0,auth.error?.message??'The upstream authentication check must exit successfully');
  assert.match(authLog,/Passed:\s+1\b/,'The upstream authentication check must pass once');
  assert.match(authLog,/Failed:\s+0\b/,'The upstream authentication check must not fail');
  assert.match(authLog,/Skipped:\s+0\b/,'The upstream authentication check must not skip Dim');
  process.stdout.write(run('uv',['run','--no-project','--python','3.14.2',path.join(scripts,'protocol_matrix.py'),
    '--agent','dimcode','--output-dir',output,'--table-mode','capabilities','--init-timeout','30','--rpc-timeout','5']));
  const matrix=JSON.parse(readFileSync(path.join(output,'latest.json'),'utf8'));
  assert.equal(matrix.agents.length,1,'The protocol matrix must probe exactly one agent');
  const agent=matrix.agents[0];
  assert.equal(agent.id,'dimcode');
  assert.equal(agent.agentInfoVersion,version,'The matrix must launch the executable under test');
  assert.equal(agent.initialize.status,'success','ACP initialize must succeed');
  assert(agent.authMethods.includes('terminal'),'The account build must advertise terminal authentication');
  assert.equal(agent.methodProbes['session/list'].status,'success','Session discovery must succeed without login');
  for (const [capability,method] of [['sessionFork','session/fork'],['sessionResume','session/resume'],
    ['sessionStop','session/stop'],['setModel','session/set_model']]) {
    if (agent.capabilities[capability]) {
      assert(['success','auth_required','invalid_params','resource_not_found'].includes(agent.methodProbes[method].status),
        `Advertised ${method} failed: ${JSON.stringify(agent.methodProbes[method])}`);
    }
  }
  writeFileSync(path.join(output,'provenance.json'),JSON.stringify({registryRevision:revision,
    sourceCommit:process.env.GITHUB_SHA??null,version,platform:`${platform}-${architecture}`},null,2)+'\n');
  console.log(`ACP Registry authentication and protocol checks passed for dimcode ${version}`);
} finally {
  const stopped=spawnSync(binary,['--home',home,'daemon','stop'],{encoding:'utf8',timeout:15_000});
  if (stopped.status!==0) {
    console.error('Registry acceptance could not stop its isolated daemon:',stopped.stderr??stopped.error);
    process.exitCode=1;
  }
  rmSync(temporary,{recursive:true,force:true});
}
