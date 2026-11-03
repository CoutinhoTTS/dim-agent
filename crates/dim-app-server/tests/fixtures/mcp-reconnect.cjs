// A stdio MCP server for reconnect tests. Node built-ins only.
const fs = require('node:fs');
const readline = require('node:readline');

const mode = process.argv[2] || 'normal';
const stateFile = process.env.MCP_STATE_FILE || '';
if (stateFile) fs.appendFileSync(stateFile, 'start\n');

function send(message) {
  process.stdout.write(JSON.stringify(message) + '\n');
}
function reply(id, result) {
  send({ jsonrpc: '2.0', id, result });
}
function fail(id) {
  send({ jsonrpc: '2.0', id, error: { code: -32000, message: 'failed' } });
}
function stderr(text) {
  process.stderr.write(text + '\n');
}

if (mode === 'always-exit') {
  stderr('startup-failure');
  process.exit(5);
}

if (mode === 'big-stderr') {
  for (let index = 1; index <= 4; index += 1) {
    stderr('big-' + index + '-' + 'x'.repeat(3000));
  }
  process.stderr.write(Buffer.from([0xff, 0xfe, 0x0a]));
  setTimeout(() => process.exit(6), 100);
  setInterval(() => {}, 1000);
  return;
}

if (mode === 'chatty') {
  for (let index = 1; index <= 200; index += 1) {
    stderr('chatty-' + index + '-' + 'y'.repeat(1000));
  }
}

const starts = stateFile
  ? fs.readFileSync(stateFile, 'utf8').split('\n').filter(Boolean).length
  : 1;

const tools = [
  { name: 'echo', description: 'Echo', inputSchema: { type: 'object', properties: {} } },
];
if (mode === 'slow-exit') {
  tools.push({ name: 'slow', description: 'Slow', inputSchema: { type: 'object', properties: {} } });
}
if (mode === 'grow-tools' && starts >= 2) {
  tools.push({ name: 'grown', description: 'Grown', inputSchema: { type: 'object', properties: {} } });
}

let calls = 0;
const rl = readline.createInterface({ input: process.stdin, terminal: false });
rl.on('line', (line) => {
  line = line.trim();
  if (!line) return;
  let message;
  try {
    message = JSON.parse(line);
  } catch {
    return;
  }
  if (message.method === 'initialize') {
    const answer = () =>
      reply(message.id, {
        protocolVersion: message.params.protocolVersion,
        capabilities: { tools: { listChanged: false } },
        serverInfo: { name: 'reconnect-fixture', version: '1' },
      });
    if (mode === 'slow-restart' && starts >= 2) {
      setTimeout(answer, 800);
      return;
    }
    answer();
    return;
  }
  if (message.method === 'notifications/initialized') return;
  if (message.method === 'tools/list') {
    reply(message.id, { tools });
    return;
  }
  if (message.method === 'ping') {
    reply(message.id, {});
    return;
  }
  if (message.method === 'tools/call') {
    calls += 1;
    const name = message.params.name;
    if (mode === 'slow-exit' && name === 'slow') {
      stderr('slow-failure');
      setTimeout(() => process.exit(9), 30);
      return;
    }
    reply(message.id, { content: [{ type: 'text', text: 'ok' }] });
    if (mode === 'exit-after-call') {
      for (let index = 1; index <= 30; index += 1) {
        stderr('stderr-line-' + index);
      }
      setTimeout(() => process.exit(7), 20);
    }
    if (mode === 'chatty') {
      stderr('chatty-final');
      setTimeout(() => process.exit(8), 20);
    }
    if (mode === 'slow-restart') {
      setTimeout(() => process.exit(0), 20);
    }
    return;
  }
  if (message.id !== undefined) {
    fail(message.id);
  }
});
process.stdin.on('end', () => process.exit(0));
setInterval(() => {}, 1000);
