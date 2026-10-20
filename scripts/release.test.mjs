import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';
import {existsSync,mkdirSync,mkdtempSync,readFileSync,rmSync,symlinkSync,writeFileSync} from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {test} from 'node:test';
import {gunzipSync} from 'node:zlib';

const root=path.resolve(import.meta.dirname,'..');
const commit='a'.repeat(40);
// The product version the checkout carries; an alpha release commit bumps it.
const version=JSON.parse(readFileSync(path.join(root,'npm/package.json'))).version;
const digest=(file)=>createHash('sha256').update(readFileSync(file)).digest('hex');
const run=(script,args,env={})=>spawnSync(process.execPath,[path.join(root,'scripts',script),...args],{encoding:'utf8',env:{...process.env,...env}});
function fixture(t) {
  const temporary=mkdtempSync(path.join(os.tmpdir(),'dim release test '));
  t.after(()=>rmSync(temporary,{recursive:true,force:true}));
  return temporary;
}
// Entry names read straight from the gzip'd ustar stream: macOS bsdtar would
// fold AppleDouble `._*` entries back into extended attributes and hide them.
function entries(archive) {
  const data=gunzipSync(readFileSync(archive));
  const names=[];
  for(let offset=0;offset+512<=data.length&&data[offset]!==0;) {
    const field=(start,length)=>data.subarray(offset+start,offset+start+length).toString('utf8').replace(/\0.*$/s,'');
    const size=Number.parseInt(field(124,12).trim()||'0',8);
    const prefix=field(345,155);
    const type=field(156,1);
    if(!['x','g','L'].includes(type)) names.push(prefix?`${prefix}/${field(0,100)}`:field(0,100));
    offset+=512+Math.ceil(size/512)*512;
  }
  return names;
}
function tar(source,archive) {
  const result=spawnSync('tar',['-czf',archive,'-C',source,'.'],{encoding:'utf8'});
  assert.equal(result.status,0,result.stderr);
}

test('stages public native payload, packs it and rejects modified content', {skip:process.platform==='win32'}, (t)=>{
  const temporary=fixture(t);
  const bin=path.join(temporary,'bin');
  mkdirSync(bin);
  writeFileSync(path.join(bin,'dimcode'),`#!/bin/sh\nprintf "dimcode ${version}\\n"\n`,{mode:0o755});
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

test('private archive gate rejects hash, schema and link violations before creating the runtime', {skip:process.platform==='win32'}, (t)=>{
  const temporary=fixture(t);
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
  const args=[destination];
  assert.notEqual(run('stage-runtime.mjs',args,{...env,DIM_OAUTH_PACKAGE_SHA256:'b'.repeat(64)}).status,0);
  assert.notEqual(run('stage-runtime.mjs',args,env).status,0);
  assert.equal(existsSync(destination),false);
  manifest.hostApiVersion=2;
  writeFileSync(path.join(source,'dim-plugin.json'),JSON.stringify(manifest));
  rmSync(path.join(source,'dist/index.js'));
  symlinkSync('/etc/passwd',path.join(source,'dist/index.js'));
  tar(source,archive);
  assert.notEqual(run('stage-runtime.mjs',args,{...env,DIM_OAUTH_PACKAGE_SHA256:digest(archive)}).status,0);
  assert.equal(existsSync(destination),false);
});

test('an account build stages only its executable, which must embed the runtime', {skip:process.platform==='win32'}, (t)=>{
  const temporary=fixture(t);
  const runtime=path.join(temporary,'runtime');
  mkdirSync(runtime);
  writeFileSync(path.join(runtime,'runtime.bin'),'runtime archive fixture');
  const sha256=digest(path.join(runtime,'runtime.bin'));
  const platform=`${process.platform}-${process.arch}`;
  const plugin={version:'0.2.0',sha256:'c'.repeat(64),hostApiVersion:2,rutisVersion:'0.10.0'};
  writeFileSync(path.join(runtime,'runtime.json'),JSON.stringify({schemaVersion:1,platform,toolchain:{node:'24.21.0',rutis:'0.12.1'},plugin,sha256,files:{'plugins/dim-oauth/dist/index.js':'d'.repeat(64)}}));
  const bin=path.join(temporary,'bin');
  mkdirSync(bin);
  writeFileSync(path.join(bin,'dimcode'),`#!/bin/sh\nprintf "dimcode ${version}\\n"\n`,{mode:0o755});
  assert.notEqual(run('stage-release.mjs',[bin,path.join(temporary,'unembedded'),version,commit,'--runtime',runtime]).status,0);
  assert.equal(existsSync(path.join(temporary,'unembedded')),false);
  writeFileSync(path.join(bin,'dimcode'),`#!/bin/sh\n# ${sha256}\nprintf "dimcode ${version}\\n"\n`,{mode:0o755});
  const stage=path.join(temporary,'stage');
  const staged=run('stage-release.mjs',[bin,stage,version,commit,'--runtime',runtime]);
  assert.equal(staged.status,0,staged.stderr);
  // An extended attribute is what makes macOS tar add an AppleDouble entry.
  if(process.platform==='darwin') assert.equal(spawnSync('xattr',['-w','com.apple.provenance','x',path.join(stage,'bin/dimcode')]).status,0);
  const manifest=JSON.parse(readFileSync(path.join(stage,'release-manifest.json')));
  assert.deepEqual(manifest.plugin,plugin);
  assert.equal(manifest.runtime.sha256,sha256);
  assert.equal(manifest.toolchain.node,'24.21.0');
  assert.deepEqual(Object.keys(manifest.files).sort(),['LICENSE','NOTICE','bin/dimcode']);
  const packed=run('pack-release.mjs',[stage,path.join(temporary,'packed')]);
  assert.equal(packed.status,0,packed.stderr);
  const npmFile=JSON.parse(readFileSync(path.join(temporary,'packed/release-manifest.json'))).npmPackages[0].file;
  assert.deepEqual(entries(path.join(temporary,'packed',npmFile)).filter((name)=>!name.endsWith('/')).sort(),
    ['package/LICENSE','package/NOTICE','package/bin/dimcode','package/package.json','package/release-manifest.json']);
});

test('stages cross-built executables without running them and rejects the wrong architecture', (t)=>{
  const temporary=fixture(t);
  const osName=process.platform==='win32'?'windows':process.platform;
  const cpu=process.arch==='arm64'?'x64':'arm64';
  const platform=`${osName}-${cpu}`;
  const bin=path.join(temporary,'bin');
  mkdirSync(bin);
  const header=Buffer.alloc(128);
  if(osName==='windows') {
    header.writeUInt16LE(0x5a4d,0);
    header.writeUInt32LE(64,0x3c);
    header.writeUInt32LE(0x00004550,64);
    header.writeUInt16LE(cpu==='arm64'?0xaa64:0x8664,68);
  } else if(osName==='darwin') {
    header.writeUInt32LE(0xfeedfacf,0);
    header.writeUInt32LE(cpu==='arm64'?0x0100000c:0x01000007,4);
  } else {
    Buffer.from('7f454c460201','hex').copy(header);
    header.writeUInt16LE(cpu==='arm64'?183:62,18);
  }
  const suffix=osName==='windows'?'.exe':'';
  const cli=path.join(bin,`dimcode${suffix}`);
  writeFileSync(cli,header,{mode:0o755});
  const stage=path.join(temporary,'stage');
  const result=run('stage-release.mjs',[bin,stage,version,commit],{DIM_BUILD_PLATFORM:platform});
  assert.equal(result.status,0,result.stderr);
  assert.equal(JSON.parse(readFileSync(path.join(stage,'release-manifest.json'))).platform,platform);
  const packed=run('pack-release.mjs',[stage,path.join(temporary,'packed')]);
  assert.equal(packed.status,0,packed.stderr);
  assert.equal(JSON.parse(readFileSync(path.join(temporary,'packed/release-manifest.json'))).nativeArchives[0].platform,platform);
  const rejected=path.join(temporary,'rejected');
  if(osName==='windows') header.writeUInt16LE(cpu==='arm64'?0x8664:0xaa64,68);
  else if(osName==='darwin') header.writeUInt32LE(cpu==='arm64'?0x01000007:0x0100000c,4);
  else header.writeUInt16LE(cpu==='arm64'?62:183,18);
  writeFileSync(cli,header);
  const mismatch=run('stage-release.mjs',[bin,rejected,version,commit],{DIM_BUILD_PLATFORM:platform});
  assert.notEqual(mismatch.status,0);
  assert.match(mismatch.stderr,/architecture mismatch/);
  assert.equal(existsSync(rejected),false);
});

test('collects exact platform packages, installs wrapper offline and rejects inconsistent plugin provenance', (t)=>{
  const temporary=fixture(t);
  const platforms=['darwin-arm64','darwin-x64','linux-arm64','linux-x64','windows-arm64','windows-x64'];
  for(const platform of platforms) {
    const directory=path.join(temporary,`release-${platform}`);
    const packageRoot=path.join(directory,'scratch/package');
    mkdirSync(path.join(packageRoot,'bin'),{recursive:true});
    const [osName,cpu]=platform.split('-');
    writeFileSync(path.join(packageRoot,'package.json'),JSON.stringify({name:`dimcode-${platform}`,version,repository:{url:'git+https://github.com/arcships/dim.git'},os:[osName==='windows'?'win32':osName],cpu:[cpu]}));
    writeFileSync(path.join(packageRoot,'bin',osName==='windows'?'dimcode.exe':'dimcode'),`#!/bin/sh\nprintf "dimcode ${version}\\n"\n`,{mode:0o755});
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
