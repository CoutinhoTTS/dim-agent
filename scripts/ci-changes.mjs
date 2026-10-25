// Decides which PR check groups a change needs. Paths no rule recognizes run
// every group, so a missed rule costs CI time rather than coverage.
import {appendFileSync,readFileSync} from 'node:fs';

export const groups=['rust','windows','xtask','web','scripts'];
const everything=Object.fromEntries(groups.map(group=>[group,true]));

const rules=[
  // xtask check validates the plugin pages against the generated snapshots.
  [/^docs\/architecture\/rust-agent\/(plugins|generated)\//,['xtask']],
  // storage_flow reads this document.
  [/^docs\/design\/appserver\/native-storage-format\.md$/,['rust']],
  [/^docs\//,[]],
  [/^[^/]+\.md$|^LICENSE$/,[]],
  [/^web\//,['web']],
  // stage-web.mjs builds the Web workspace.
  [/^scripts\//,['scripts','web']],
  // The Windows job tests dim-cli, which depends only on dim-agent-types in this workspace.
  [/^crates\/(dim-cli|dim-agent-types)\//,['rust','windows']],
  // Rust tests read runtime/ fixtures, and the Node flows exercise npm/ and runtime/.
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

// GitHub lists at most 3000 files for a pull request and silently drops the rest.
const fileLimit=3000;

// `entries` is the pull request files listing; `changedFiles` is the pull request's own count.
// A listing that may be incomplete selects every group.
export function selectGroups(entries,changedFiles) {
  if (entries.length>=fileLimit||entries.length!==changedFiles) return everything;
  // A rename also counts its old path, so moving source into docs/ keeps the source checks.
  return classify(entries.flatMap(({filename,previous_filename})=>previous_filename?[filename,previous_filename]:[filename]));
}

async function pullRequestFiles(repository,number,token) {
  const entries=[];
  for (let page=1;;page++) {
    const response=await fetch(`https://api.github.com/repos/${repository}/pulls/${number}/files?per_page=100&page=${page}`,{
      headers:{accept:'application/vnd.github+json',authorization:`Bearer ${token}`,'x-github-api-version':'2022-11-28'},
    });
    if (!response.ok) throw new Error(`Listing pull request files failed: ${response.status} ${await response.text()}`);
    const batch=await response.json();
    entries.push(...batch);
    if (batch.length<100) return entries;
  }
}

if (import.meta.main) {
  const {GITHUB_EVENT_NAME,GITHUB_EVENT_PATH,GITHUB_REPOSITORY,GITHUB_TOKEN,GITHUB_OUTPUT}=process.env;
  let result=everything;
  // Pushes to main and manual runs verify everything.
  if (GITHUB_EVENT_NAME==='pull_request') {
    const {number,changed_files}=JSON.parse(readFileSync(GITHUB_EVENT_PATH)).pull_request;
    const entries=await pullRequestFiles(GITHUB_REPOSITORY,number,GITHUB_TOKEN);
    console.log(`${entries.length} of ${changed_files} files listed`);
    console.log(entries.map(({filename})=>filename).join('\n'));
    result=selectGroups(entries,changed_files);
  }
  const lines=Object.entries(result).map(([group,value])=>`${group}=${value}`);
  console.log(lines.join('\n'));
  if (GITHUB_OUTPUT) appendFileSync(GITHUB_OUTPUT,lines.join('\n')+'\n');
}
