import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import test from 'node:test';
import {replyFrames} from './tui-tmux/model.mjs';
import {scenarios} from './tui-tmux/scenarios.mjs';

const document = readFileSync(new URL('../docs/architecture/tui-e2e.md', import.meta.url), 'utf8');
const workflow = readFileSync(new URL('../.github/workflows/ci.yml', import.meta.url), 'utf8');

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

test('the migration mapping keeps the line-mode startup case and the #1805 survivors', () => {
  assert(
    document.includes('`fresh_native_startup_uses_workspace_and_ask_without_policy_file`'),
    'the startup case keeps its routing in the mapping',
  );
  assert(document.includes('is line mode, not a TUI case'), 'the startup case is recorded as line mode');
  for (const survivor of [
    'tui_home_commands_without_a_session_answer_and_send_nothing',
    'tui_draws_a_prompt_hooks_context_as_a_status_row',
    'prompt_suggestion::tab_keeps_the_fullscreen_focus_and_right_still_takes_the_offer',
  ]) {
    assert(document.includes(survivor), `the mapping names the survivor ${survivor}`);
  }
});

test('no CI job runs the tmux suite', () => {
  assert(!workflow.includes('tui-tmux'), 'CI must not invoke the local TUI E2E suite');
});

test('a scripted reply frames text and tool calls as an OpenAI stream', () => {
  const text = replyFrames({text: 'answer'});
  assert(text.at(-1).includes('"finish_reason":"stop"'), 'the last choice stops the reply');
  assert(text.join('').includes('"delta":{"content":"'), 'the text is streamed as deltas');
  assert.equal(text.filter((frame) => frame.includes('"content"')).map((frame) => JSON.parse(frame.slice('data: '.length)).choices[0].delta.content).join(''), 'answer');
  const calls = replyFrames({toolCalls: [{name: 'exec', arguments: {command: 'echo hi'}}]});
  const delta = JSON.parse(calls[0].slice('data: '.length));
  assert.equal(delta.choices[0].delta.tool_calls[0].function.name, 'exec');
  assert.equal(delta.choices[0].delta.tool_calls[0].function.arguments, '{"command":"echo hi"}');
  assert(calls.at(-1).includes('"finish_reason":"tool_calls"'), 'a tool reply ends in tool_calls');
});
