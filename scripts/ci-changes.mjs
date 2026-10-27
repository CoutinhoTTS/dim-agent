import {execFileSync} from 'node:child_process';
import {appendFileSync} from 'node:fs';

export const groups=['rust','windows','xtask','web','scripts','dependencies'];
const everything=Object.fromEntries(groups.map(group=>[group,true]));

const rules=[
  [/^docs\/architecture\/rust-agent\/(plugins|generated)\//,['xtask']],
  [/^docs\/design\/appserver\/native-storage-format\.md$/,['rust']],
  [/^docs\//,[]],
  [/^deny\.toml$|^LICENSE$/,['dependencies']],
  [/^[^/]+\.md$/,[]],
  [/^web\/src\/features\/chat\/quote-cases\.json$/,['web','rust','windows']],
  [/^web\//,['web']],
  [/^scripts\//,['scripts','web']],
  [/^crates\/(dim-cli|dim-agent-types)\//,['rust','windows']],
  [/^(crates|tests|xtask|vendor|runtime|npm)\//,['rust']],
];

export function classify(files) {
  const needed=new Set();
  for (const file of files) {
    const rule=rules.find(([pattern])=>pattern.test(file));
    for (const group of rule?rule[1]:groups) needed.add(group);
  }
  return Object.fromEntries(groups.map(group=>[group,needed.has(group)]));
}

if (import.meta.main) {
  let result=everything;
  if (process.env.GITHUB_EVENT_NAME==='pull_request') {
    const files=execFileSync('git',['diff','--name-only','--no-renames','HEAD^1','HEAD'],{encoding:'utf8'}).split('\n').filter(Boolean);
    console.log(files.join('\n'));
    result=classify(files);
  }
  const lines=Object.entries(result).map(([group,value])=>`${group}=${value}`);
  console.log(lines.join('\n'));
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT,lines.join('\n')+'\n');
}
