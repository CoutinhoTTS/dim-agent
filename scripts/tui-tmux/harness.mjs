import {execFileSync} from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {modelConfig, startModelFixture} from './model.mjs';

const DEFAULT_PATH = '/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin';
const TMUX_TIMEOUT_MS = 20_000;
const POLL_MS = 100;

export const DEFAULT_STEP_TIMEOUT_MS = 25_000;

/// The tmux server's socket lives in a short path: a unix socket name is
/// bounded, and the temporary home below it may be long.
export function socketRoot() {
  return fs.existsSync('/tmp') ? '/tmp' : fs.realpathSync(os.tmpdir());
}

function quote(value) {
  return `'${String(value).replace(/'/g, "'\\''")}'`;
}

function shellCommand(environment, argv) {
  const assignments = Object.entries(environment).map(([key, value]) => `${key}=${value}`);
  return ['env', '-i', ...assignments, ...argv].map(quote).join(' ');
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitUntil(predicate, {timeout = DEFAULT_STEP_TIMEOUT_MS, interval = POLL_MS, describe = 'condition'} = {}) {
  const deadline = Date.now() + timeout;
  for (;;) {
    const value = await predicate();
    if (value) return value;
    if (Date.now() >= deadline) throw new Error(`waited ${timeout}ms for ${describe}`);
    await sleep(interval);
  }
}

export function tmuxVersion(tmux = 'tmux') {
  const printed = execFileSync(tmux, ['-V'], {encoding: 'utf8'}).trim();
  const match = /^tmux\s+(\d+)\.(\d+)/.exec(printed);
  if (!match) throw new Error(`cannot read the tmux version from ${JSON.stringify(printed)}`);
  return {printed, major: Number(match[1]), minor: Number(match[2])};
}

/// One scenario's disposable state: a product home, a user home, a model
/// fixture, a dedicated tmux server and socket, and its raw output tap.
export class TmuxCase {
  constructor({name, root, binary, tmux = 'tmux', keep = false, verbose = false}) {
    this.name = name;
    this.root = root;
    this.binary = binary;
    this.tmux = tmux;
    this.keep = keep;
    this.verbose = verbose;
    this.home = path.join(root, 'home');
    this.userHome = path.join(root, 'user-home');
    this.socket = path.join(root, 'tmux.sock');
    this.appServer = path.join(path.dirname(binary), 'dim-app-server');
    this.requestsFile = path.join(root, 'requests.jsonl');
    this.interactionsFile = path.join(root, 'interactions.log');
    this.sessions = [];
  }

  /// `documents` writes product-home settings documents (for example
  /// `hooks.json` or `prompt-suggestion.json`), which is how a case turns on a
  /// daemon feature the production App Server reads from its home.
  async setup({respond, config = {}, files = {}, documents = {}}) {
    fs.mkdirSync(path.join(this.home, 'rust'), {recursive: true, mode: 0o700});
    fs.chmodSync(path.join(this.home, 'rust'), 0o700);
    fs.writeFileSync(path.join(this.home, 'config.json'), JSON.stringify({locale: 'en', ...config}));
    for (const [name, document] of Object.entries(documents)) {
      fs.writeFileSync(path.join(this.home, name), JSON.stringify(document, null, 2));
    }
    fs.mkdirSync(this.userHome, {recursive: true});
    fs.mkdirSync(path.join(this.root, 'tmp'), {recursive: true});
    fs.mkdirSync(path.join(this.root, 'workspace'), {recursive: true});
    for (const [name, content] of Object.entries(files)) {
      const file = path.join(this.root, 'workspace', name);
      fs.mkdirSync(path.dirname(file), {recursive: true});
      fs.writeFileSync(file, content);
    }
    this.model = await startModelFixture({respond, logFile: this.requestsFile});
    this.modelFile = path.join(this.root, 'model.json');
    fs.writeFileSync(this.modelFile, JSON.stringify(modelConfig(this.model.url), null, 2));
    this.writeTmuxConfig();
    this.log(`# ${this.name}: home=${this.home} model=${this.model.url}`);
  }

  writeTmuxConfig() {
    this.configFile = path.join(this.root, 'tmux.conf');
    fs.writeFileSync(
      this.configFile,
      [
        'set-option -g remain-on-exit on',
        'set-option -g exit-empty off',
        'set-option -g default-terminal "screen-256color"',
        'set-option -g history-limit 20000',
        'set-option -g status off',
        'set-option -g escape-time 10',
        'set-option -g window-size manual',
        'set-option -g detach-on-destroy on',
        'set-option -g set-clipboard off',
        '',
      ].join('\n'),
    );
  }

  environment(extra = {}) {
    return {
      HOME: this.userHome,
      TMPDIR: path.join(this.root, 'tmp'),
      PATH: process.env.PATH ?? DEFAULT_PATH,
      LANG: process.env.LANG ?? 'en_US.UTF-8',
      TERM: 'screen-256color',
      SHELL: '/bin/sh',
      DIMCODE_UPDATE_CHECK: '0',
      ...extra,
    };
  }

  tmuxRun(args, {allowFailure = false} = {}) {
    try {
      return execFileSync(
        this.tmux,
        ['-S', this.socket, ...(this.configFile ? ['-f', this.configFile] : []), ...args],
        {
          encoding: 'utf8',
          timeout: TMUX_TIMEOUT_MS,
          stdio: ['ignore', 'pipe', 'pipe'],
        },
      );
    } catch (error) {
      if (allowFailure) return '';
      const stderr = error.stderr?.toString().trim() ?? '';
      throw new Error(`tmux ${args.join(' ')} failed: ${stderr || error.message}`);
    }
  }

  /// Starts one CLI (or launcher) on its own tmux session and taps the pane's
  /// output stream, which is where escape sequences a pane cannot show live.
  ///
  /// `modelConfig` runs the development model profile, whose daemon serves
  /// sessions but not provider management. `server` runs the managed
  /// production profile instead: the standalone App Server next to the CLI with
  /// the case's product home, so `/connect` manages real providers.
  async launch(
    argv = [],
    {env = {}, cols = 100, rows = 30, name, command, waitForComposer = false, modelConfig = true, server = false} = {},
  ) {
    const session = name ?? `cli-${this.sessions.length + 1}`;
    this.sessions.push(session);
    if (server && !fs.existsSync(this.appServer)) {
      throw new Error(`${this.appServer} is missing; build it with: cargo build --locked -p dim-app-server`);
    }
    const program =
      command ??
      (server
        ? [this.binary, '--home', this.home, '--server', this.appServer]
        : modelConfig
          ? [this.binary, '--home', this.home, '--model-config', this.modelFile]
          : [this.binary, '--home', this.home]);
    const line = shellCommand(this.environment(env), [...program, ...argv]);
    this.log(`# launch ${session}: ${line}`);
    this.tmuxRun(['new-session', '-d', '-s', session, '-x', String(cols), '-y', String(rows), line]);
    const rawFile = path.join(this.root, `${session}.raw`);
    this.tmuxRun(['pipe-pane', '-o', '-t', session, `cat >> ${quote(rawFile)}`]);
    const terminal = new Terminal(this, {session, rawFile});
    if (waitForComposer) await terminal.untilComposer();
    return terminal;
  }

  log(line) {
    const entry = `${new Date().toISOString()} ${line}`;
    fs.appendFileSync(this.interactionsFile, `${entry}\n`);
    if (this.verbose) process.stderr.write(`  ${entry}\n`);
  }

  runCommand(args, {env = {}, cwd, timeout = 60_000} = {}) {
    this.log(`# command: ${this.binary} ${args.join(' ')}${cwd ? ` (cwd ${cwd})` : ''}`);
    return execFileSync(this.binary, args, {
      encoding: 'utf8',
      timeout,
      cwd,
      env: {...process.env, ...this.environment(env)},
      stdio: ['ignore', 'pipe', 'pipe'],
    });
  }

  stopDaemon() {
    try {
      return this.runCommand(['--home', this.home, 'daemon', 'stop'], {timeout: 30_000});
    } catch {
      return '';
    }
  }

  async close() {
    this.tmuxRun(['kill-server'], {allowFailure: true});
    this.stopDaemon();
    if (fs.existsSync(path.join(this.home, 'rust', 'daemon.sock'))) {
      try {
        execFileSync('pkill', ['-f', path.join(this.home, 'rust', 'dimcode-server')], {
          stdio: 'ignore',
          timeout: 5_000,
        });
      } catch {
        // No daemon left to stop.
      }
    }
    await this.model?.close();
    if (!this.keep) fs.rmSync(this.root, {recursive: true, force: true});
  }

  /// Copies everything a failure needs to be diagnosed from the case's own
  /// state: both captures, the raw terminal stream, the fixture's request log,
  /// the interaction log and the daemon's own log.
  writeArtifacts(target, error) {
    fs.mkdirSync(target, {recursive: true});
    const write = (name, content) => fs.writeFileSync(path.join(target, name), content);
    write('failure.txt', `${error?.stack ?? error}\n`);
    for (const session of this.sessions) {
      const terminal = new Terminal(this, {
        session,
        rawFile: path.join(this.root, `${session}.raw`),
      });
      try {
        write(`${session}-screen.txt`, terminal.capture({history: false}));
        write(`${session}-scrollback.txt`, terminal.capture({history: true}));
      } catch (captureError) {
        write(`${session}-screen.txt`, `<capture failed: ${captureError.message}>\n`);
      }
      const raw = path.join(this.root, `${session}.raw`);
      if (fs.existsSync(raw)) fs.copyFileSync(raw, path.join(target, `${session}-pane.raw`));
    }
    for (const [source, name] of [
      [this.interactionsFile, 'interactions.log'],
      [this.requestsFile, 'requests.jsonl'],
      [path.join(this.home, 'rust', 'daemon.log'), 'daemon.log'],
      [path.join(this.home, 'config.json'), 'config.json'],
      [path.join(this.root, 'model.json'), 'model.json'],
    ]) {
      if (fs.existsSync(source)) fs.copyFileSync(source, path.join(target, name));
    }
    const listing = [];
    const walk = (directory, prefix = '') => {
      for (const entry of fs.readdirSync(directory, {withFileTypes: true})) {
        const relative = path.join(prefix, entry.name);
        if (entry.isDirectory()) {
          listing.push(`${relative}/`);
          if (relative.split(path.sep).length < 3) walk(path.join(directory, entry.name), relative);
        } else {
          listing.push(`${relative} ${fs.statSync(path.join(directory, entry.name)).size}`);
        }
      }
    };
    if (fs.existsSync(this.home)) walk(this.home);
    write('home-listing.txt', `${listing.join('\n')}\n`);
    return target;
  }
}

export class Terminal {
  constructor(owner, {session, rawFile}) {
    this.owner = owner;
    this.session = session;
    this.rawFile = rawFile;
  }

  send(text) {
    this.owner.log(`send ${this.session} ${JSON.stringify(text)}`);
    this.owner.tmuxRun(['send-keys', '-t', this.session, '-l', '--', text]);
  }

  key(...keys) {
    this.owner.log(`key ${this.session} ${keys.join(' ')}`);
    this.owner.tmuxRun(['send-keys', '-t', this.session, ...keys]);
  }

  bytes(buffer) {
    const hex = Array.from(buffer, (byte) => byte.toString(16).padStart(2, '0'));
    this.owner.log(`bytes ${this.session} ${hex.join(' ')}`);
    this.owner.tmuxRun(['send-keys', '-t', this.session, '-H', ...hex]);
  }

  paste(text) {
    this.owner.log(`paste ${this.session} ${JSON.stringify(text)}`);
    this.bytes(Buffer.concat([
      Buffer.from('\u001b[200~', 'utf8'),
      Buffer.from(text, 'utf8'),
      Buffer.from('\u001b[201~', 'utf8'),
    ]));
  }

  resize(cols, rows) {
    this.owner.log(`resize ${this.session} ${cols}x${rows}`);
    this.owner.tmuxRun(['resize-window', '-t', this.session, '-x', String(cols), '-y', String(rows)]);
  }

  /// Resizes the pane and waits for the settled layout: the interface pauses
  /// frame painting until the resize debounce settles, so a screen still wider
  /// than the pane is the frame before the rewrite.
  async resizeTo(cols, rows, {timeout = DEFAULT_STEP_TIMEOUT_MS} = {}) {
    this.resize(cols, rows);
    const deadline = Date.now() + timeout;
    for (;;) {
      const text = this.capture({join: false});
      const widest = Math.max(0, ...text.split('\n').map((line) => line.length));
      if (widest <= cols) {
        this.untilComposer({timeout: Math.max(1000, deadline - Date.now())});
        return text;
      }
      if (Date.now() >= deadline) {
        const size = this.owner.tmuxRun(
          ['display-message', '-p', '-t', this.session, '#{window_width}x#{window_height} pane #{pane_width}x#{pane_height}'],
          {allowFailure: true},
        ).trim();
        throw new Error(
          `pane stayed ${widest} columns wide ${timeout}ms after resizing to ${cols}; tmux reports ${size}\n${text}`,
        );
      }
      await sleep(POLL_MS);
    }
  }

  capture({history = false, join = true} = {}) {
    const args = ['capture-pane', '-p', '-t', this.session];
    if (join) args.push('-J');
    if (history) args.push('-S', '-');
    return this.owner.tmuxRun(args);
  }

  raw() {
    return fs.existsSync(this.rawFile) ? fs.readFileSync(this.rawFile, 'utf8') : '';
  }

  /// The visible screen or, with `history`, the transcript the pane keeps above
  /// it, as the user can scroll to it.
  screen({history = false} = {}) {
    return this.capture({history});
  }

  async waitFor(expected, {timeout = DEFAULT_STEP_TIMEOUT_MS, history = false, describe} = {}) {
    const matches = (text) => (typeof expected === 'string' ? text.includes(expected) : expected.test(text));
    const deadline = Date.now() + timeout;
    const what = describe ?? (expected instanceof RegExp ? expected.toString() : JSON.stringify(expected));
    for (;;) {
      const text = this.capture({history});
      if (matches(text)) return text;
      if (Date.now() >= deadline) {
        throw new Error(
          `pane did not show ${what} within ${timeout}ms\n--- screen ---\n${this.capture()}\n--- transcript ---\n${this.capture({history: true})}\n--- output tail ---\n${tail(this.raw(), 4000)}`,
        );
      }
      await sleep(POLL_MS);
    }
  }

  async waitForRaw(predicate, {timeout = DEFAULT_STEP_TIMEOUT_MS, describe = 'terminal output'} = {}) {
    const deadline = Date.now() + timeout;
    for (;;) {
      const raw = this.raw();
      if (predicate(raw)) return raw;
      if (Date.now() >= deadline) {
        throw new Error(
          `terminal output did not satisfy ${describe} within ${timeout}ms\n--- output tail ---\n${tail(raw, 4000)}\n--- screen ---\n${this.capture()}`,
        );
      }
      await sleep(POLL_MS);
    }
  }

  async waitForAbsent(expected, {timeout = DEFAULT_STEP_TIMEOUT_MS} = {}) {
    const deadline = Date.now() + timeout;
    for (;;) {
      if (!this.capture().includes(expected)) return;
      if (Date.now() >= deadline) {
        throw new Error(`pane still showed ${JSON.stringify(expected)} after ${timeout}ms\n${this.capture()}`);
      }
      await sleep(POLL_MS);
    }
  }

  untilComposer({timeout = DEFAULT_STEP_TIMEOUT_MS} = {}) {
    return this.waitFor('Ask anything...', {timeout, describe: 'the composer'});
  }

  async sendPrompt(text, {timeout = DEFAULT_STEP_TIMEOUT_MS} = {}) {
    this.send(text);
    this.key('Enter');
    return this.untilComposer({timeout});
  }

  alive() {
    const listing = this.owner.tmuxRun(['list-panes', '-t', this.session, '-F', '#{pane_dead}'], {allowFailure: true});
    return listing.trim() === '0';
  }

  /// Waits for the CLI to end; `remain-on-exit` keeps the pane, its last screen
  /// and its exit status.
  async waitForExit({timeout = DEFAULT_STEP_TIMEOUT_MS} = {}) {
    const line = await waitUntil(
      () => {
        const listing = this.owner.tmuxRun(
          ['list-panes', '-t', this.session, '-F', '#{pane_dead} #{pane_dead_status} #{pane_dead_signal}'],
          {allowFailure: true},
        ).trim();
        if (!listing) return false;
        const [dead, status] = listing.split(' ');
        return dead === '1' ? {status: Number(status)} : false;
      },
      {timeout, describe: `${this.session} to exit`},
    );
    return line;
  }
}

function tail(text, length) {
  return text.length > length ? `…${text.slice(-length)}` : text;
}
