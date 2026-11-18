import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const ANSWER = 'The fixture answer is deterministic.';

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function lastMessage(request) {
  return (request.messages ?? []).at(-1) ?? {};
}

function userText(request) {
  const last = lastMessage(request);
  return typeof last.content === 'string' ? last.content : JSON.stringify(last.content ?? '');
}

/// Requests the fixture answered for a real turn: a prompt-suggestion or title
/// request carries no tool declarations, a turn request does.
function turnRequests(model) {
  return model.requests.filter((entry) => (entry.body.tools ?? []).length > 0);
}

function lastTurnText(model) {
  const turn = turnRequests(model).at(-1);
  assert(turn, 'the fixture model answered a turn request');
  return userText(turn.body);
}

async function waitForRequest(model, predicate, {timeout = 20_000, describe = 'a fixture request'} = {}) {
  const deadline = Date.now() + timeout;
  for (;;) {
    const found = model.requests.find((entry) => predicate(entry.body));
    if (found) return found;
    if (Date.now() >= deadline) {
      throw new Error(
        `waited ${timeout}ms for ${describe}; ${model.requests.length} request(s): ${model.requests
          .map((entry) => JSON.stringify(userText(entry.body).slice(0, 40)))
          .join(', ')}`,
      );
    }
    await sleep(100);
  }
}

async function exitWith(terminal, expected = 0) {
  terminal.send('/exit');
  terminal.key('Enter');
  await terminal.waitFor('Bye!');
  const status = await terminal.waitForExit();
  assert.equal(status.status, expected, `the CLI exits with ${expected}`);
  return status;
}

const scenarios = [
  {
    name: 'scrollback_streams_and_exits_cleanly',
    area: 'terminal-modes',
    title: 'The default scrollback mode streams an answer and exits into the same transcript',
    async run(tui) {
      await tui.setup({respond: () => ({text: ANSWER})});
      const terminal = await tui.launch([], {waitForComposer: true});

      await terminal.waitFor('Welcome to DIM');
      await terminal.waitFor('Workspace write (shift+tab to cycle)');
      const startup = terminal.raw();
      assert(!startup.includes('\u001b[?1049h'), 'scrollback must not take the alternate screen');

      await terminal.sendPrompt('hello fixture');
      await terminal.waitFor(ANSWER);
      await terminal.waitFor('┃ hello fixture');
      assert.equal(lastTurnText(tui.model), 'hello fixture');

      await exitWith(terminal);

      const transcript = terminal.screen({history: true});
      assert(transcript.includes('hello fixture'), 'the scrollback keeps the message');
      assert(transcript.includes(ANSWER), 'the scrollback keeps the answer');

      const raw = terminal.raw();
      assert(!raw.includes('\u001b[?1049h'), 'a scrollback run never enters the alternate screen');
      assert(raw.includes('\u001b[?25h'), 'the cursor is shown again on exit');
      assert(raw.includes('\u001b]7501;state=working'), 'a run reports working through OSC 7501');
      assert(raw.includes('\u001b]7501;state=clear'), 'leaving clears the status record');
    },
  },
  {
    name: 'fullscreen_switches_screens_and_restores_the_terminal',
    area: 'terminal-modes',
    title: 'Fullscreen enters and leaves the alternate screen with a balanced keyboard protocol',
    async run(tui) {
      await tui.setup({respond: () => ({text: ANSWER})});
      const terminal = await tui.launch(['--fullscreen'], {waitForComposer: true});

      await terminal.waitForRaw((raw) => raw.includes('\u001b[?1049h'), {describe: 'the alternate screen'});
      await terminal.waitFor('Welcome to DIM');

      await terminal.sendPrompt('hello fullscreen');
      await terminal.waitFor(ANSWER);

      await terminal.send('/tui');
      terminal.key('Enter');
      await terminal.waitForRaw((raw) => raw.includes('\u001b[?1049l'), {describe: 'the caller screen'});
      await terminal.untilComposer();
      assert(
        terminal.raw().indexOf('\u001b[?1049l') > terminal.raw().indexOf('\u001b[?1049h'),
        'leaving fullscreen returns to the caller screen',
      );

      await terminal.send('/tui');
      terminal.key('Enter');
      await terminal.waitForRaw((raw) => (raw.match(/\u001b\[\?1049h/g) ?? []).length >= 2, {
        describe: 'fullscreen again',
      });
      await terminal.untilComposer();

      await exitWith(terminal);

      const raw = terminal.raw();
      const pushes = (raw.match(/\u001b\[>\d+u/g) ?? []).length;
      const pops = (raw.match(/\u001b\[<u/g) ?? []).length;
      assert.equal(pops, pushes, `every keyboard-protocol push is popped (${pushes} pushes, ${pops} pops)`);
      assert(raw.includes('\u001b[?25h'), 'the cursor is restored on exit');
      assert(raw.trimEnd().includes('\u001b[?1049l'), 'the last screen is the caller screen');
    },
  },
  {
    name: 'resize_rewraps_history_without_losing_rows',
    area: 'terminal-modes',
    title: 'Resizing rewraps a long message and its answer in place',
    async run(tui) {
      await tui.setup({respond: () => ({text: ANSWER})});
      const marker = 'resize-marker-0123456789';
      const prompt = `${marker} ${'the quick brown fox wraps '.repeat(4)}end-of-prompt`;
      const terminal = await tui.launch([], {waitForComposer: true});
      await terminal.sendPrompt(prompt);
      await terminal.waitFor(ANSWER);

      for (const [cols, rows] of [[60, 18], [140, 40]]) {
        await terminal.resizeTo(cols, rows);
        const transcript = terminal.screen({history: true});
        assert(transcript.includes(marker), `${cols}x${rows} keeps the message`);
        assert(transcript.includes('end-of-prompt'), `${cols}x${rows} keeps the message tail`);
        assert(transcript.includes(ANSWER), `${cols}x${rows} keeps the answer`);
        const visible = terminal.capture({join: false});
        const width = Math.max(0, ...visible.split('\n').map((line) => line.length));
        assert(width <= cols, `no visible row is wider than ${cols} columns (widest ${width})`);
      }

      await exitWith(terminal);
    },
  },
  {
    name: 'composer_edits_multiline_and_stores_a_large_paste',
    area: 'composer',
    title: 'The composer edits a newline draft and stores a multi-line paste as an attachment',
    async run(tui) {
      await tui.setup({respond: () => ({text: ANSWER})});
      const terminal = await tui.launch([], {waitForComposer: true});

      terminal.send('first line');
      terminal.key('C-j');
      terminal.send('second line');
      await terminal.waitFor('second line');
      for (let index = 0; index < 5; index += 1) terminal.key('BSpace');
      terminal.send('END');
      await terminal.waitFor('secondEND');
      terminal.key('Enter');
      await terminal.untilComposer();
      await terminal.waitFor(ANSWER);
      assert.equal(lastTurnText(tui.model), 'first line\nsecondEND');

      const pasted = ['alpha', 'beta', 'gamma', 'delta', 'epsilon'].join('\n');
      terminal.paste(pasted);
      await terminal.waitFor('[... 5 lines]');
      terminal.key('Enter');
      await terminal.untilComposer();
      await terminal.waitFor('[Pasted text:', {history: true});
      const sent = await waitForRequest(
        tui.model,
        (request) => (request.tools ?? []).length > 0 && userText(request).includes('[Pasted text:'),
        {describe: 'the stored paste in a turn request'},
      );
      assert(!userText(sent.body).includes('epsilon'), 'a stored paste is not inlined into the prompt');

      await exitWith(terminal);
    },
  },
  {
    name: 'file_mention_inserts_a_workspace_link',
    area: 'composer',
    title: 'The @ picker lists the workspace and inserts the chosen file into the draft',
    async run(tui) {
      await tui.setup({
        files: {
          'lantern-notes.md': '# Notes\n\nThe fixture note mentions a lantern.\n',
          'src/app.ts': 'export const answer = 42;\n',
        },
        respond: () => ({text: ANSWER}),
      });
      const workspace = path.join(tui.root, 'workspace');
      const terminal = await tui.launch(['--cwd', workspace], {waitForComposer: true});

      terminal.send('@lantern');
      await terminal.waitFor('Files');
      await terminal.waitFor('@lantern-notes.md');
      terminal.key('Enter');
      await terminal.waitFor('@lantern-notes.md', {describe: 'the inserted mention'});
      terminal.key('Enter');
      await terminal.untilComposer();
      await terminal.waitFor(ANSWER);

      const sent = lastTurnText(tui.model);
      assert(sent.includes('@lantern-notes.md'), `the prompt carries the mention: ${sent}`);

      await exitWith(terminal);
    },
  },
  {
    name: 'slash_palette_lists_curated_commands_and_settings',
    area: 'commands',
    title: 'The command palette keeps its curated order and opens a settings panel',
    async run(tui) {
      await tui.setup({respond: () => ({text: ANSWER})});
      const terminal = await tui.launch([], {waitForComposer: true});

      terminal.send('/');
      await terminal.waitFor('Commands');
      await terminal.waitFor('/new');
      const palette = terminal.capture();
      assert(palette.includes('type filter'), 'the palette names its keyboard actions');
      assert(palette.indexOf('/new') < palette.indexOf('/resume'), 'the curated order puts /new first');
      assert(palette.indexOf('/resume') < palette.indexOf('/models'), 'the curated order follows /new');

      terminal.send('settings');
      await terminal.waitFor('/settings');
      terminal.key('Enter');
      await terminal.waitFor('Settings');
      await terminal.waitFor('Theme');
      await terminal.waitFor('Language');
      terminal.key('Escape');
      await terminal.waitForAbsent('Settings');

      terminal.send('/keys');
      terminal.key('Enter');
      await terminal.waitFor('Enter submit when idle', {history: true});
      await terminal.waitForAbsent('Commands');

      await exitWith(terminal);
    },
  },
  {
    name: 'streaming_status_reports_and_interrupts',
    area: 'runs',
    title: 'A streaming run reports its state and Escape interrupts it',
    async run(tui) {
      await tui.setup({
        respond: (request) =>
          (request.tools ?? []).length > 0 && userText(request).includes('slow')
            ? {text: 'slow answer. '.repeat(80), chunkSize: 8, chunkDelayMs: 120}
            : {text: ANSWER},
      });
      const terminal = await tui.launch([], {waitForComposer: true});

      terminal.send('slow run');
      terminal.key('Enter');
      await terminal.waitFor('working…');
      await terminal.waitFor('esc interrupt');
      await terminal.waitForRaw((raw) => raw.includes('state=working'), {describe: 'the working status'});

      terminal.key('Escape');
      await terminal.waitFor('User interrupted the run.');
      await terminal.waitFor('stopped');
      await terminal.untilComposer();
      await terminal.waitForRaw((raw) => raw.includes('state=idle') || raw.includes('state=error'), {
        describe: 'the settled status',
      });

      await exitWith(terminal);
      assert(terminal.raw().includes('\u001b]7501;state=clear'), 'leaving clears the status record');
    },
  },
  {
    name: 'queue_holds_steers_and_deletes',
    area: 'input-queue',
    title: 'Queued input is held during a run, deleted, and steered through the panel',
    async run(tui) {
      await tui.setup({
        config: {tuiDefaultSendMode: 'queue'},
        respond: (request) =>
          (request.tools ?? []).length > 0 && userText(request).includes('slow')
            ? {text: 'slow answer. '.repeat(120), chunkSize: 8, chunkDelayMs: 150}
            : {text: ANSWER},
      });
      const terminal = await tui.launch([], {waitForComposer: true});

      terminal.send('slow run');
      terminal.key('Enter');
      await terminal.waitFor('esc interrupt');

      terminal.send('first follow-up');
      terminal.key('Enter');
      await terminal.waitFor('Queue (1)');
      await terminal.waitFor('Held · first follow-up');
      terminal.send('second follow-up');
      terminal.key('Enter');
      await terminal.waitFor('Queue (2)');
      await terminal.waitFor('Held · second follow-up');

      terminal.send('/queue');
      terminal.key('Enter');
      await terminal.waitFor('Held · awaiting application', {describe: 'the queue panel'});
      terminal.key('x');
      await terminal.waitForAbsent('Held · first follow-up');
      await sleep(500);
      terminal.key('Enter');
      await terminal.waitFor('No queued messages.', {describe: 'the emptied queue'});
      await terminal.waitFor('┃ second follow-up', {history: true});
      terminal.key('Escape');
      await terminal.waitForAbsent('Queue');

      await terminal.waitForAbsent('esc interrupt', {timeout: 40_000});
      await exitWith(terminal);
    },
  },
  {
    name: 'approval_panel_allows_one_command',
    area: 'approvals',
    title: 'The permission panel allows one command and its output returns to the model',
    async run(tui) {
      await tui.setup({
        respond: (request) => {
          if ((request.tools ?? []).length === 0) return {text: ANSWER};
          if (lastMessage(request).role === 'tool') return {text: 'The command printed fixture-ok.'};
          return {toolCalls: [{name: 'exec', arguments: {command: 'echo fixture-ok'}}]};
        },
      });
      const terminal = await tui.launch([], {waitForComposer: true});

      terminal.send('run it');
      terminal.key('Enter');
      await terminal.waitFor('Permission required');
      await terminal.waitFor('echo fixture-ok');
      await terminal.waitFor('Allow once');
      await terminal.waitFor('permission required');
      await terminal.waitForRaw((raw) => raw.includes('state=blocked:kind=permission'), {
        describe: 'the blocked status',
      });

      terminal.key('y');
      await terminal.waitFor('Ran 1 command');
      await terminal.waitFor('The command printed fixture-ok.');
      await terminal.untilComposer();

      const turn = await waitForRequest(tui.model, (request) => lastMessage(request).role === 'tool', {
        describe: 'the tool result request',
      });
      assert(
        String(lastMessage(turn.body).content).includes('fixture-ok'),
        'the tool output reaches the model',
      );

      await exitWith(terminal);
    },
  },
  {
    name: 'question_panel_answer_reaches_the_next_request',
    area: 'approvals',
    title: 'A question is answered in the panel and the answer reaches the model',
    async run(tui) {
      await tui.setup({
        respond: (request) => {
          if ((request.tools ?? []).length === 0) return {text: ANSWER};
          if (lastMessage(request).role === 'tool') return {text: 'The answer was recorded.'};
          return {
            toolCalls: [
              {
                name: 'ask_user',
                arguments: {
                  questions: [
                    {
                      header: 'Fixture choice',
                      question: 'Which fixture should I use?',
                      options: [{label: 'Option A'}, {label: 'Option B'}],
                    },
                  ],
                },
              },
            ],
          };
        },
      });
      const terminal = await tui.launch([], {waitForComposer: true});

      terminal.send('ask me');
      terminal.key('Enter');
      await terminal.waitFor('Which fixture should I use?');
      await terminal.waitFor('Option A');
      await terminal.waitFor('answer required');

      terminal.key('Down');
      terminal.key('Enter');
      await terminal.waitFor('Review your answers');
      terminal.key('Enter');
      await terminal.waitFor('The answer was recorded.');
      await terminal.untilComposer();

      const turn = await waitForRequest(tui.model, (request) => lastMessage(request).role === 'tool', {
        describe: 'the answered question request',
      });
      assert(
        String(lastMessage(turn.body).content).includes('Option B'),
        `the chosen answer reaches the model: ${lastMessage(turn.body).content}`,
      );

      await exitWith(terminal);
    },
  },
  {
    name: 'background_task_row_and_tasks_panel',
    area: 'tasks',
    title: 'A background command shows a task row and the tasks panel can stop it',
    async run(tui) {
      await tui.setup({
        respond: (request) => {
          if ((request.tools ?? []).length === 0) return {text: ANSWER};
          if (lastMessage(request).role === 'tool') return {text: 'Started the fixture task.'};
          return {
            toolCalls: [
              {
                name: 'exec',
                arguments: {command: 'sleep 30', runInBackground: true, title: 'Fixture background work'},
              },
            ],
          };
        },
      });
      const terminal = await tui.launch([], {waitForComposer: true});

      terminal.send('task please');
      terminal.key('Enter');
      await terminal.waitFor('Permission required');
      terminal.key('a');
      await terminal.waitFor('Ran 1 command');
      await terminal.waitFor('Fixture background work');
      await terminal.waitFor('1 in background');

      terminal.send('/tasks');
      terminal.key('Enter');
      await terminal.waitFor('Background tasks');
      await terminal.waitFor('command · Running');
      terminal.key('x');
      await terminal.waitForAbsent('1 in background');
      terminal.key('Escape');
      await terminal.waitForAbsent('Background tasks');
      await terminal.untilComposer();

      await exitWith(terminal);
    },
  },
  {
    name: 'daemon_stop_reports_disconnect_and_resume_replays_history',
    area: 'daemon',
    title: 'A stopped daemon keeps the draft, names the resume command and the resumed session replays history',
    async run(tui) {
      await tui.setup({respond: () => ({text: ANSWER})});
      const terminal = await tui.launch([], {waitForComposer: true});

      await terminal.sendPrompt('remember this');
      await terminal.waitFor(ANSWER);
      terminal.send('unfinished follow-up');
      await terminal.waitFor('unfinished follow-up');

      tui.stopDaemon();
      await terminal.waitFor('Disconnected; draft kept');
      const screen = await terminal.waitFor('dimcode --resume');
      const resume = /dimcode --resume ([0-9a-f-]{36})/.exec(screen);
      assert(resume, `the notice names the resume command: ${screen}`);
      assert.equal((await terminal.waitForExit()).status, 0);
      assert(terminal.raw().includes('\u001b[?25h'), 'the terminal is restored when the daemon leaves');

      const resumed = await tui.launch(['--resume', resume[1]], {waitForComposer: true});
      await resumed.waitFor('remember this', {history: true});
      await resumed.waitFor(ANSWER, {history: true});
      await resumed.sendPrompt('after restart');
      await resumed.waitFor(ANSWER);

      await exitWith(resumed);
    },
  },
  {
    name: 'npm_launcher_streams_and_passes_interrupts',
    area: 'launcher',
    title: 'The npm launcher runs the native TUI and forwards terminal input and exit status',
    async run(tui, {repository} = {}) {
      await tui.setup({respond: () => ({text: ANSWER})});
      const wrapper = path.join(tui.root, 'wrapper');
      const platform = `${process.platform}-${process.arch}`;
      const packageDir = path.join(wrapper, 'node_modules', `dimcode-${platform}`, 'bin');
      fs.mkdirSync(packageDir, {recursive: true});
      fs.writeFileSync(
        path.join(wrapper, 'node_modules', `dimcode-${platform}`, 'package.json'),
        JSON.stringify({name: `dimcode-${platform}`}),
      );
      fs.linkSync(tui.binary, path.join(packageDir, 'dimcode'));
      const launcherDir = path.join(wrapper, 'bin');
      fs.mkdirSync(launcherDir, {recursive: true});
      fs.copyFileSync(path.join(repository, 'npm', 'bin', 'dimcode.mjs'), path.join(launcherDir, 'dimcode.mjs'));
      fs.copyFileSync(path.join(repository, 'npm', 'package.json'), path.join(wrapper, 'package.json'));

      const terminal = await tui.launch(['--home', tui.home, '--model-config', tui.modelFile], {
        command: ['node', path.join(launcherDir, 'dimcode.mjs')],
        waitForComposer: true,
      });

      await terminal.sendPrompt('launcher prompt');
      await terminal.waitFor(ANSWER);
      assert.equal(lastTurnText(tui.model), 'launcher prompt');

      terminal.key('C-c');
      await sleep(300);
      assert(terminal.alive(), 'the first interrupt keeps the interface');
      terminal.key('C-c');
      const status = await terminal.waitForExit();
      assert.equal(status.status, 130, 'a second interrupt ends the CLI with the interrupt status');
      assert(terminal.raw().includes('\u001b[?25h'), 'the launcher leaves the terminal restored');
    },
  },
];

export {scenarios};
