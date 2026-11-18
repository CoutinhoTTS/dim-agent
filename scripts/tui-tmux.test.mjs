import assert from 'node:assert/strict';
import {readdirSync, readFileSync} from 'node:fs';
import test from 'node:test';
import {replyFrames} from './tui-tmux/model.mjs';
import {scenarios} from './tui-tmux/scenarios.mjs';

const document = readFileSync(new URL('../docs/architecture/tui-e2e.md', import.meta.url), 'utf8');
const workflowsDirectory = new URL('../.github/workflows/', import.meta.url);
const workflows = readdirSync(workflowsDirectory)
  .filter((name) => /\.ya?ml$/.test(name))
  .map((name) => ({name, text: readFileSync(new URL(name, workflowsDirectory), 'utf8')}));
const workflow = workflows.find((entry) => entry.name === 'ci.yml').text;

test('every tmux scenario is unique and documented', () => {
  const names = scenarios.map((scenario) => scenario.name);
  assert.deepEqual([...new Set(names)].sort(), [...names].sort(), 'scenario names are unique');
  for (const scenario of scenarios) {
    assert.match(scenario.area, /^[a-z-]+$/, scenario.name);
    assert(scenario.title.length > 20, `${scenario.name} has a descriptive title`);
    assert.equal(typeof scenario.run, 'function', `${scenario.name} runs`);
    assert(document.includes(`\`${scenario.name}\``), `docs/architecture/tui-e2e.md lists ${scenario.name}`);
  }
  assert(document.includes('node scripts/tui-tmux/run.mjs'), 'the suite documents its command');
});

test('the CI selection names its ignored flows instead of the whole suite', () => {
  assert(
    workflow.includes('fresh_native_startup_uses_workspace_and_ask_without_policy_file'),
    'the line-mode startup case stays selected',
  );
  assert(!/cli_flow[^\n]*--ignored/.test(workflow), 'the ignored selection names each flow it runs');
  assert(!workflow.includes('DIM_TEST_NPM_WRAPPER'), 'the TUI launcher flow left CI');
});

test('no workflow runs the tmux suite', () => {
  for (const entry of workflows) {
    assert(!entry.text.includes('tui-tmux'), `${entry.name} must not invoke the local TUI E2E suite`);
  }
});

test('a scripted reply frames text and tool calls as an OpenAI stream', () => {
  const text = replyFrames({text: 'answer'});
  assert(text.at(-1).includes('"finish_reason":"stop"'), 'the last choice stops the reply');
  assert(text.join('').includes('"delta":{"content":"'), 'the text is streamed as deltas');
  assert.equal(text.filter((frame) => frame.includes('"content"')).map((frame) => JSON.parse(frame.slice('data: '.length)).choices[0].delta.content).join(''), 'answer');
  const reasoned = replyFrames({thinking: 'plan', text: 'answer'});
  const deltas = reasoned.map((frame) => JSON.parse(frame.slice('data: '.length)).choices[0].delta);
  assert.equal(deltas.filter((delta) => delta.reasoning_content).map((delta) => delta.reasoning_content).join(''), 'plan', 'reasoning streams as reasoning_content');
  assert(
    deltas.findIndex((delta) => delta.reasoning_content) < deltas.findIndex((delta) => delta.content),
    'reasoning arrives before the answer',
  );
  const calls = replyFrames({toolCalls: [{name: 'exec', arguments: {command: 'echo hi'}}]});
  const delta = JSON.parse(calls[0].slice('data: '.length));
  assert.equal(delta.choices[0].delta.tool_calls[0].function.name, 'exec');
  assert.equal(delta.choices[0].delta.tool_calls[0].function.arguments, '{"command":"echo hi"}');
  assert(calls.at(-1).includes('"finish_reason":"tool_calls"'), 'a tool reply ends in tool_calls');
});
