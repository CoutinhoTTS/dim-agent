import assert from 'node:assert/strict';
import {readdirSync, readFileSync} from 'node:fs';
import test from 'node:test';
import {replyFrames} from './tui-tmux/model.mjs';
import {reflowComplete} from './tui-tmux/harness.mjs';
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

// A backslash-newline continuation belongs to the line it continues, so the
// multi-line CI selection is one shell command, not one command per line.
function logicalCommands(script) {
  const commands = [];
  let command = '';
  for (const line of script.split('\n')) {
    const continued = /\\\s*$/.test(line);
    command += `${continued ? line.replace(/\\\s*$/, '') : line}\n`;
    if (!continued) {
      if (command.trim()) commands.push(command.trim());
      command = '';
    }
  }
  if (command.trim()) commands.push(command.trim());
  return commands;
}

const IGNORED_CLI_FLOWS = [
  'accounts::login_reaches_the_codex_account_of_the_production_daemon',
  'acp::acp_stdio_temporary_mcp_calls_tools_and_settles_the_child_on_eof',
  'configured_api_key_model_uses_production_tools_and_restores_the_same_session',
  'export_writes_the_timeline_without_starting_a_daemon',
  'fresh_native_startup_uses_workspace_and_ask_without_policy_file',
  'hooks::project_hooks_start_disabled_and_the_user_enables_them',
  'mcp::mcp_entries_round_trip_through_the_real_app_server',
  'permission::real_daemon_always_allow_saves_an_exact_rule_until_approvals_removes_it',
  'permission::real_daemon_exec_presets_decide_process_tools_without_automatic_approval',
  'project_mcp_entries_start_disabled_and_the_user_enables_them',
  'real_daemon_profile_conflicts_preserve_the_running_daemon_and_endpoint',
  'workflow::workflow_commands_write_and_enable_against_the_real_owner',
];

// The `cli_flow --ignored` selections of a script as word lists. A bare
// `--ignored` is still a selection, which `assertNamedIgnoredSelection`
// rejects: without named flows it would run every ignored case.
function ignoredCliFlowSelections(script) {
  return logicalCommands(script)
    .filter((command) => /\bcli_flow\b/.test(command) && /(?:^|\s)--ignored(?:\s|$)/.test(command))
    .map((command) => command.split(/\s+/));
}

function assertNamedIgnoredSelection(script, expected) {
  const selections = ignoredCliFlowSelections(script);
  assert.equal(selections.length, 1, 'one CI command runs the ignored cli_flow selection');
  const words = selections[0];
  const after = words.slice(words.indexOf('cli_flow') + 1);
  const names = after.filter((word) => !word.startsWith('--'));
  assert.deepEqual(
    [...names].sort(),
    [...expected].sort(),
    'the ignored selection names each flow it runs',
  );
}

test('the CI selection names its ignored flows instead of the whole suite', () => {
  assert(
    workflow.includes('fresh_native_startup_uses_workspace_and_ask_without_policy_file'),
    'the line-mode startup case stays selected',
  );
  assertNamedIgnoredSelection(workflow, IGNORED_CLI_FLOWS);
  assert(!workflow.includes('DIM_TEST_NPM_WRAPPER'), 'the TUI launcher flow left CI');
});

test('the ignored-flow guard reads a continued command as one line', () => {
  const expected = [IGNORED_CLI_FLOWS[0]];
  const bare = [
    '      - run: |',
    '          node scripts/run-rust-test.mjs out.jsonl cli_flow \\',
    '            --ignored --test-threads=2',
    '',
  ].join('\n');
  assert.equal(ignoredCliFlowSelections(bare).length, 1, 'the continuation is one command');
  assert.throws(
    () => assertNamedIgnoredSelection(bare, expected),
    /names each flow it runs/,
    'a bare --ignored after cli_flow is rejected',
  );
  const named = [
    '      - run: |',
    '          node scripts/run-rust-test.mjs out.jsonl cli_flow \\',
    `            ${expected[0]} \\`,
    '            --ignored --test-threads=2',
    '',
  ].join('\n');
  assert.doesNotThrow(() => assertNamedIgnoredSelection(named, expected));
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

test('a resize waits for a reflow frame, not a cursor report from the old one', () => {
  assert(!reflowComplete(''), 'no output is not a repaint');
  assert(!reflowComplete('\u001b[?25h old frame'), 'a cursor report before the clear is the outgoing frame');
  assert(!reflowComplete('\u001b[3J rows without a completed frame'), 'the clear alone is not the new frame');
  assert(reflowComplete('\u001b[?25h old\u001b[3J rows\u001b[?25h'), 'the clear and the completed frame after it are');
  assert(!reflowComplete('\u001b[3J rows\u001b[?25h\u001b[3J next rewrite'), 'a later clear restarts the wait');
});
