import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';
import {existsSync,mkdirSync,mkdtempSync,readFileSync,rmSync,symlinkSync,writeFileSync} from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {test} from 'node:test';

const root=path.resolve(import.meta.dirname,'..');
const commit='a'.repeat(40);
const version='0.1.0';
const digest=(file)=>createHash('sha256').update(readFileSync(file)).digest('hex');
const run=(script,args,env={})=>spawnSync(process.execPath,[path.join(root,'scripts',script),...args],{encoding:'utf8',env:{...process.env,...env}});
function fixture(t) {
  const temporary=mkdtempSync(path.join(os.tmpdir(),'dim release test '));
  t.after(()=>rmSync(temporary,{recursive:true,force:true}));
  return temporary;
}
function tar(source,archive) {
  const result=spawnSync('tar',['-czf',archive,'-C',source,'.'],{encoding:'utf8'});
  assert.equal(result.status,0,result.stderr);
}

test('stages public native payload, packs it and rejects modified content', {skip:process.platform==='win32'}, (t)=>{
  const temporary=fixture(t);
  const bin=path.join(temporary,'bin');
  mkdirSync(bin);
  writeFileSync(path.join(bin,'dimcode'),'#!/bin/sh\nprintf "dimcode 0.1.0\\n"\n',{mode:0o755});
  writeFileSync(path.join(bin,'dim-app-server'),'#!/bin/sh\nexit 0\n',{mode:0o755});
  const stage=path.join(temporary,'stage');
  const staged=run('stage-release.mjs',[bin,stage,version,commit]);
  assert.equal(staged.status,0,staged.stderr);
  const manifest=JSON.parse(readFileSync(path.join(stage,'release-manifest.json')));
  assert.equal(manifest.plugin,null);
  assert.equal(existsSync(path.join(stage,'runtime')),false);
  const packed=run('pack-release.mjs',[stage,path.join(temporary,'packed')]);
  assert.equal(packed.status,0,packed.stderr);
  const packedManifest=JSON.parse(readFileSync(path.join(temporary,'packed/release-manifest.json')));
  const metadata=spawnSync('tar',['-xOzf',path.join(temporary,'packed',packedManifest.npmPackages[0].file),'package/package.json'],{encoding:'utf8'});
  assert.equal(JSON.parse(metadata.stdout).license,'MIT');
  writeFileSync(path.join(stage,'bin/dimcode'),'modified');
  assert.notEqual(run('pack-release.mjs',[stage,path.join(temporary,'tampered')]).status,0);
  assert.equal(existsSync(path.join(temporary,'tampered')),false);
});

test('private archive gate rejects hash, schema and link violations before creating release content', {skip:process.platform==='win32'}, (t)=>{
  const temporary=fixture(t);
  const bin=path.join(temporary,'bin');
  mkdirSync(bin);
  writeFileSync(path.join(bin,'dimcode'),'#!/bin/sh\nprintf "dimcode 0.1.0\\n"\n',{mode:0o755});
  writeFileSync(path.join(bin,'dim-app-server'),'#!/bin/sh\nexit 0\n',{mode:0o755});
  const source=path.join(temporary,'plugin');
  mkdirSync(path.join(source,'dist'),{recursive:true});
  writeFileSync(path.join(source,'package.json'),JSON.stringify({type:'module',version:'0.1.0'}));
  writeFileSync(path.join(source,'dist/index.js'),'export default {};');
  const manifest={schemaVersion:1,id:'dim-oauth',version:'0.1.0',hostApiVersion:99,rutisVersion:'0.10.0',entry:'dist/index.js',files:{'dist/index.js':digest(path.join(source,'dist/index.js'))}};
  writeFileSync(path.join(source,'dim-plugin.json'),JSON.stringify(manifest));
  const archive=path.join(temporary,'plugin.tar.gz');
  tar(source,archive);
  const destination=path.join(temporary,'output');
  const env={DIM_OAUTH_PACKAGE_ARCHIVE:archive,DIM_OAUTH_PACKAGE_URL:'',DIM_OAUTH_PACKAGE_SHA256:digest(archive)};
  const args=[bin,destination,version,commit,'--with-plugin'];
  assert.notEqual(run('stage-release.mjs',args,{...env,DIM_OAUTH_PACKAGE_SHA256:'b'.repeat(64)}).status,0);
  assert.notEqual(run('stage-release.mjs',args,env).status,0);
  assert.equal(existsSync(destination),false);
  manifest.hostApiVersion=1;
  writeFileSync(path.join(source,'dim-plugin.json'),JSON.stringify(manifest));
  rmSync(path.join(source,'dist/index.js'));
  symlinkSync('/etc/passwd',path.join(source,'dist/index.js'));
  tar(source,archive);
  assert.notEqual(run('stage-release.mjs',args,{...env,DIM_OAUTH_PACKAGE_SHA256:digest(archive)}).status,0);
  assert.equal(existsSync(destination),false);
});

test('collects exact platform packages, installs wrapper offline and rejects inconsistent plugin provenance', (t)=>{
  const temporary=fixture(t);
  const platforms=['darwin-arm64','darwin-x64','linux-arm64','linux-x64','windows-x64'];
  for(const platform of platforms) {
    const directory=path.join(temporary,`release-${platform}`);
    const packageRoot=path.join(directory,'scratch/package');
    mkdirSync(path.join(packageRoot,'bin'),{recursive:true});
    const [osName,cpu]=platform.split('-');
    writeFileSync(path.join(packageRoot,'package.json'),JSON.stringify({name:`dimcode-${platform}`,version,repository:{url:'git+https://github.com/arcships/dim.git'},os:[osName==='windows'?'win32':osName],cpu:[cpu]}));
    writeFileSync(path.join(packageRoot,'bin',osName==='windows'?'dimcode.exe':'dimcode'),'#!/bin/sh\nprintf "dimcode 0.1.0\\n"\n',{mode:0o755});
    const npmFile=`dimcode-${platform}-${version}.tgz`;
    assert.equal(spawnSync('tar',['-czf',path.join(directory,npmFile),'-C',path.join(directory,'scratch'),'package']).status,0);
    const nativeFile=`dimcode-${platform}-${version}.${osName==='windows'?'zip':'tar.gz'}`;
    writeFileSync(path.join(directory,nativeFile),'native archive fixture');
    writeFileSync(path.join(directory,'release-manifest.json'),JSON.stringify({schemaVersion:1,platform,version,sourceCommit:commit,toolchain:{rust:'1.98.1'},plugin:null,npmPackages:[{name:`dimcode-${platform}`,file:npmFile,sha256:digest(path.join(directory,npmFile))}],nativeArchives:[{platform,file:nativeFile,sha256:digest(path.join(directory,nativeFile))}]}));
  }
  const complete=path.join(temporary,'complete');
  const collected=run('collect-release.mjs',[temporary,complete]);
  assert.equal(collected.status,0,collected.stderr);
  assert.equal(run('check-native.mjs',[path.join(complete,'native'),version,commit]).status,0);
  const receiptPath=path.join(temporary,'npm-publication.json');
  const receipt={...JSON.parse(readFileSync(path.join(complete,'npm/release-manifest.json'))),buildRunId:'123'};
  writeFileSync(receiptPath,JSON.stringify(receipt));
  assert.equal(run('check-native.mjs',[path.join(complete,'native'),version,commit,receiptPath,'123']).status,0);
  assert.notEqual(run('check-native.mjs',[path.join(complete,'native'),version,commit,receiptPath,'124']).status,0);
  receipt.npmPackages[0].sha256='b'.repeat(64);
  writeFileSync(receiptPath,JSON.stringify(receipt));
  assert.notEqual(run('check-native.mjs',[path.join(complete,'native'),version,commit,receiptPath,'123']).status,0);
  if(process.platform!=='win32') {
    const current=`dimcode-${process.platform}-${process.arch}`;
    const installed=path.join(temporary,'installation');
    mkdirSync(installed);
    const npmCli=path.join(path.dirname(process.execPath),'../lib/node_modules/npm/bin/npm-cli.js');
    const install=spawnSync(process.execPath,[npmCli,'install','--prefix',installed,'--offline','--ignore-scripts','--omit=optional','--no-audit','--no-fund',path.join(complete,'npm',`${current}-${version}.tgz`),path.join(complete,'npm',`dimcode-${version}.tgz`)],{encoding:'utf8'});
    assert.equal(install.status,0,install.stderr);
    const smoke=spawnSync(process.execPath,[path.join(installed,'node_modules/dimcode/bin/dimcode.mjs'),'--version'],{encoding:'utf8'});
    assert.equal(smoke.status,0,smoke.stderr);
    assert.equal(smoke.stdout.trim(),`dimcode ${version}`);
  }
  const incompatible=path.join(temporary,'release-linux-x64/release-manifest.json');
  const manifest=JSON.parse(readFileSync(incompatible));
  manifest.plugin={version:'0.1.0',sha256:'b'.repeat(64)};
  writeFileSync(incompatible,JSON.stringify(manifest));
  assert.notEqual(run('collect-release.mjs',[temporary,path.join(temporary,'inconsistent')]).status,0);
  assert.equal(existsSync(path.join(temporary,'inconsistent')),false);
});
