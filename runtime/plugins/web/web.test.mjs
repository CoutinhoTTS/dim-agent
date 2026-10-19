import assert from 'node:assert/strict';
import { test } from 'node:test';
import http from 'node:http';
import https from 'node:https';
import net from 'node:net';
import tls from 'node:tls';
import dns from 'node:dns/promises';
import { once } from 'node:events';
import { gzipSync } from 'node:zlib';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { load } from '@arcships/rutis/testing';
import web from './index.ts';

async function listen(t, server) {
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  t.after(async () => {
    server.closeAllConnections?.();
    await new Promise(resolve => server.close(resolve));
  });
  return `http://127.0.0.1:${server.address().port}`;
}

async function plugin(t, document = { allowPrivateHosts: [] }) {
  let current = document;
  const loaded = await load(web, { services: { settings: {
    async read() { return { ok: true, value: { revision: 'revision', document: current } }; },
  } } });
  t.after(() => loaded.unload());
  return {
    loaded,
    set(next) { current = next; },
    call(name, args, cancelled = new Promise(() => {})) {
      return loaded.service(`toolContribution:${name}`).execute({
        sessionId: 'session', branchId: 'main', turnId: 'turn', callId: 'call', arguments: args,
      }, cancelled);
    },
  };
}

const error = (result, code) => {
  assert.equal(result.isError, true, JSON.stringify(result));
  assert.match(result.content, new RegExp(`failed: ${code}:`));
};

test('search uses current Settings, filters canonical Brave results and keeps secrets out of results', async t => {
  const key = 'private-brave-fixture-key';
  let requests = 0;
  const base = await listen(t, http.createServer((req, res) => {
    requests++;
    assert.equal(req.method, 'GET');
    if (req.url === '/private-type') { res.setHeader('Content-Type', `application/${key}`); return res.end('body'); }
    assert.equal(req.headers['x-subscription-token'], key);
    const query = new URL(req.url, 'http://fixture').searchParams.get('q');
    res.setHeader('Content-Type', 'application/json');
    if (query === 'invalid') return res.end('{}');
    if (query === 'failed') { res.writeHead(500); return res.end(key); }
    if (query === 'empty') return res.end(JSON.stringify({ type: 'search' }));
    res.end(JSON.stringify({ type: 'search', web: { results: [
      { title: `<b>Title ${key}</b>` + '😀'.repeat(2000), url: 'https://docs.example.com/a#section', description: key + ' <b>snippet</b>\n' + 'x'.repeat(600) },
      { title: 'Other', url: 'https://other.test/', description: 'other' },
    ] } }));
  }));
  const p = await plugin(t);
  error(await p.call('web_search', { query: 'initial' }), 'search_unavailable');
  p.set({ allowPrivateHosts: ['127.0.0.1'], searchService: { kind: 'brave', endpoint: base, apiKey: key } });
  const result = await p.call('web_search', { query: 'query', allowed_domains: ['example.com'], max_results: 1 });
  assert.notEqual(result.isError, true);
  assert.match(result.content, /\[1\] Title \[redacted\]/);
  assert.equal(result.structuredContent.results.length, 1);
  assert.equal(result.structuredContent.results[0].url, 'https://docs.example.com/a#section');
  assert.equal([...result.structuredContent.results[0].title].length, 500);
  assert.equal([...result.structuredContent.results[0].snippet].length, 500);
  assert.ok(!JSON.stringify(result).includes(key));
  assert.match((await p.call('web_search', { query: 'empty' })).content, /^No results/);
  error(await p.call('web_search', { query: 'invalid' }), 'search_failed');
  error(await p.call('web_search', { query: 'failed' }), 'search_failed');
  const privateType = await p.call('web_fetch', { url: base + '/private-type' });
  error(privateType, 'unsupported_content_type');
  assert.ok(!JSON.stringify(privateType).includes(key));
  const before = requests;
  for (const patch of [{ endpoint: 'not a url' }, { endpoint: base, apiKey: 'unsafe\nheader' }]) {
    p.set({ allowPrivateHosts: ['127.0.0.1'], searchService: { kind: 'brave', apiKey: key, ...patch } });
    error(await p.call('web_search', { query: 'query' }), 'search_unavailable');
  }
  p.set({ allowPrivateHosts: ['127.0.0.1'], proxy: 'not a url', searchService: { kind: 'brave', endpoint: base, apiKey: key } });
  error(await p.call('web_search', { query: 'query' }), 'search_unavailable');
  error(await p.call('web_fetch', { url: base }), 'connect_failed');
  assert.equal(requests, before);
  p.set({ allowPrivateHosts: [] });
  error(await p.call('web_search', { query: 'removed' }), 'search_unavailable');
});

test('fetch converts HTML and text, pages Unicode scalars and reuses the final-URL cache', async t => {
  const counts = new Map();
  const base = await listen(t, http.createServer((req, res) => {
    counts.set(req.url, (counts.get(req.url) ?? 0) + 1);
    if (req.url === '/html') {
      res.setHeader('Content-Type', 'text/html');
      return res.end('<title>Title &amp; page</title><nav>navigation</nav><main><h1>Hello</h1><script>hidden</script><p>😀' + 'a'.repeat(1100) + '</p><a href="/next">Next</a><input value="hidden"><style>hidden</style></main><p>outside</p>');
    }
    if (req.url === '/json') { res.setHeader('Content-Type', 'application/json'); return res.end('{"value":1,"large":9007199254740993,"precise":1.234567890123456789}'); }
    if (req.url === '/long-title') {
      res.setHeader('Content-Type', 'text/html');
      return res.end('<title>' + '😀'.repeat(200_000) + '</title><main>' + 'x'.repeat(1200) + '</main>');
    }
    if (req.url === '/csv') { res.setHeader('Content-Type', 'text/csv'); return res.end('a,b\n1,2'); }
    if (req.url === '/charset') { res.setHeader('Content-Type', 'text/html'); return res.end(Buffer.from('<meta charset="windows-1252"><main>caf\xe9</main>', 'latin1')); }
    if (req.url === '/empty') { res.setHeader('Content-Type', 'text/plain'); return res.end(); }
    if (req.url === '/binary') { res.setHeader('Content-Type', 'application/pdf'); return res.end('PDF'); }
    if (req.url === '/missing') { res.writeHead(404); return res.end('private error body'); }
    if (req.url.startsWith('/redirect/')) {
      const hops = Number(req.url.split('/').at(-1));
      res.writeHead(302, { Location: hops ? `/redirect/${hops - 1}` : '/json' });
      return res.end();
    }
    if (req.url === '/cross') { res.writeHead(302, { Location: 'https://other.example/path' }); return res.end(); }
    res.setHeader('Content-Type', 'text/plain');
    res.end('😀'.repeat(1200));
  }));
  const p = await plugin(t, { allowPrivateHosts: ['127.0.0.1', 'bad/entry'] });
  const html = await p.call('web_fetch', { url: base + '/html#section', max_chars: 1000 });
  assert.notEqual(html.isError, true, JSON.stringify(html));
  assert.match(html.content, /Title: Title & page/);
  assert.match(html.content, /# Hello/);
  assert.doesNotMatch(html.content, /navigation|hidden|outside/);
  assert.equal(html.structuredContent.truncated, true);
  assert.equal(html.structuredContent.final_url, base + '/html');
  assert.equal(html.structuredContent.diagnostics[0].code, 'invalid_private_host');
  const next = await p.call('web_fetch', { url: base + '/html', start_char: 1000, max_chars: 1000 });
  assert.match(next.content, new RegExp(`\\[Next\\]\\(${base}/next\\)`));
  assert.equal(counts.get('/html'), 1);
  for (const start_char of [0, 1000]) {
    const titled = await p.call('web_fetch', { url: base + '/long-title', start_char, max_chars: 1000 });
    assert.equal([...titled.structuredContent.title].length, 500);
    assert.ok([...titled.content].length < 1800, 'remote titles must not bypass the page output budget');
    assert.equal(titled.structuredContent.total, 1200);
  }
  assert.equal(counts.get('/long-title'), 1);
  const unicode = await p.call('web_fetch', { url: base + '/unicode', start_char: 1000, max_chars: 1000 });
  assert.equal(unicode.structuredContent.total, 1200);
  assert.equal(unicode.structuredContent.end, 1200);
  assert.equal([...unicode.content.split('\n\n').at(-1)].length, 200);
  const past = await p.call('web_fetch', { url: base + '/unicode', start_char: 1300 });
  assert.equal(past.structuredContent.end, 1300);
  assert.equal(past.content.split('\n\n').at(-1), '');
  const json = await p.call('web_fetch', { url: base + '/json' });
  assert.match(json.content, /"value": 1/);
  assert.match(json.content, /9007199254740993/);
  assert.match(json.content, /1\.234567890123456789/);
  assert.match((await p.call('web_fetch', { url: base + '/charset' })).content, /café/);
  assert.match((await p.call('web_fetch', { url: base + '/csv' })).content, /a,b\n1,2/);
  assert.match((await p.call('web_fetch', { url: base + '/empty' })).content, /\(no text content\)/);
  error(await p.call('web_fetch', { url: base + '/binary' }), 'unsupported_content_type');
  error(await p.call('web_fetch', { url: base + '/missing' }), 'http_status');
  assert.notEqual((await p.call('web_fetch', { url: base + '/redirect/4' })).isError, true);
  error(await p.call('web_fetch', { url: base + '/redirect/5' }), 'too_many_redirects');
  assert.equal((await p.call('web_fetch', { url: base + '/cross' })).content, 'Redirected to https://other.example/path; call web_fetch with that URL to follow it.');
  error(await p.call('web_fetch', { url: base, max_chars: 999 }), 'invalid_input');
  error(await p.call('web_fetch', { url: 'http://user:password@127.0.0.1/' }), 'invalid_input');
  error(await p.call('web_search', { query: 'x', allowed_domains: [], blocked_domains: [] }), 'invalid_input');
});

test('address policy rejects non-public literals and every private DNS answer before connecting', async t => {
  const p = await plugin(t);
  for (const host of ['127.1', '0x7f000001', '0.0.0.0', '10.0.0.1', '172.16.0.1', '192.168.0.1', '100.64.0.1', '169.254.169.254', '198.18.0.1', '224.0.0.1', '255.255.255.255', '[::1]', '[::]', '[::ffff:127.0.0.1]', '[::127.0.0.1]', '[fe80::1]', '[fc00::1]', '[ff02::1]', '[2001:db8::1]', '[2001:2::1]', '[3fff::1]']) {
    error(await p.call('web_fetch', { url: `http://${host}/` }), 'blocked_address');
  }
  t.mock.method(dns, 'lookup', async () => [{ address: '8.8.8.8', family: 4 }, { address: '127.0.0.1', family: 4 }]);
  for (const host of ['public.test', 'localhost', 'sub.localhost']) error(await p.call('web_fetch', { url: `http://${host}/` }), 'blocked_address');
});

test('direct and CONNECT requests pin DNS addresses while preserving TLS SNI and Host', async t => {
  const directory = mkdtempSync(path.join(tmpdir(), 'dim-web-tls-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  execFileSync('openssl', ['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-keyout', path.join(directory, 'key.pem'), '-out', path.join(directory, 'cert.pem'), '-days', '1', '-subj', '/CN=public.test', '-addext', 'subjectAltName=DNS:public.test'], { stdio: 'ignore' });
  const cert = readFileSync(path.join(directory, 'cert.pem'), 'utf8');
  const authorities = tls.getCACertificates('default');
  tls.setDefaultCACertificates([...authorities, cert]);
  t.after(() => tls.setDefaultCACertificates(authorities));
  const hosts = [];
  const secure = https.createServer({ key: readFileSync(path.join(directory, 'key.pem')), cert }, (req, res) => {
    hosts.push([req.headers.host, req.socket.servername]);
    if (req.url.startsWith('/search?')) {
      res.setHeader('Content-Type', 'application/json');
      return res.end(JSON.stringify({ type: 'search', web: { results: [{ title: 'Public result', url: 'https://other.test/' }] } }));
    }
    res.setHeader('Content-Type', 'text/plain');
    res.end('pinned');
  });
  await listen(t, secure);
  const port = secure.address().port;
  const targets = [];
  const proxy = http.createServer();
  const tunnels = new Set();
  t.after(() => { for (const socket of tunnels) socket.destroy(); });
  proxy.on('connect', (req, client, head) => {
    targets.push(req.url);
    if (req.url.startsWith('[::1]:')) return client.end('HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n');
    const upstream = net.connect({ host: '127.0.0.1', port }, () => {
      client.write('HTTP/1.1 200 Connection Established\r\n\r\n');
      if (head.length) upstream.write(head);
      upstream.pipe(client).pipe(upstream);
    });
    for (const socket of [client, upstream]) { tunnels.add(socket); socket.on('close', () => tunnels.delete(socket)); socket.on('error', () => socket.destroy()); }
  });
  const proxyUrl = await listen(t, proxy);
  let lookups = 0;
  let fallback = false;
  let fakeIp = false;
  t.mock.method(dns, 'lookup', async host => {
    assert.ok(['public.test', 'wrong.test'].includes(host));
    lookups++;
    if (fakeIp) return [{ address: '198.18.0.1', family: 4 }];
    if (fallback) return [{ address: '::1', family: 6 }, { address: '127.0.0.1', family: 4 }];
    return [{ address: lookups % 2 ? '127.0.0.1' : '127.0.0.2', family: 4 }];
  });
  const p = await plugin(t, { allowPrivateHosts: ['public.test'] });
  assert.match((await p.call('web_fetch', { url: `https://public.test:${port}/direct` })).content, /pinned/);
  assert.equal(lookups, 1, 'HTTP/TLS must not resolve the name again');
  lookups = 0;
  p.set({ allowPrivateHosts: ['public.test'], proxy: proxyUrl });
  const throughProxy = await p.call('web_fetch', { url: `https://public.test:${port}/proxy` });
  assert.notEqual(throughProxy.isError, true, JSON.stringify(throughProxy));
  assert.equal(lookups, 1, 'the proxy receives an IP, never a resolvable target name');
  assert.deepEqual(targets, [`127.0.0.1:${port}`]);
  assert.deepEqual(hosts, [[`public.test:${port}`, 'public.test'], [`public.test:${port}`, 'public.test']]);
  fallback = true;
  for (const proxy of [undefined, proxyUrl]) {
    lookups = 0;
    p.set({ allowPrivateHosts: ['public.test'], ...(proxy ? { proxy } : {}) });
    const result = await p.call('web_fetch', { url: `https://public.test:${port}/fallback-${Boolean(proxy)}` });
    assert.notEqual(result.isError, true, JSON.stringify(result));
    assert.match(result.content, /pinned/);
    assert.equal(lookups, 1, 'fallback must reuse the validated DNS answers');
  }
  assert.deepEqual(targets.slice(1), [`[::1]:${port}`, `127.0.0.1:${port}`]);
  assert.deepEqual(hosts.slice(2), [[`public.test:${port}`, 'public.test'], [`public.test:${port}`, 'public.test']]);
  fallback = false;
  lookups = 0;
  p.set({ allowPrivateHosts: ['wrong.test'] });
  error(await p.call('web_fetch', { url: `https://wrong.test:${port}/wrong-identity` }), 'tls_failed');
  const refused = http.createServer();
  refused.on('connect', (_req, socket) => socket.end('HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n'));
  const refusedUrl = await listen(t, refused);
  lookups = 0;
  p.set({ allowPrivateHosts: ['public.test'], proxy: refusedUrl });
  error(await p.call('web_fetch', { url: `https://public.test:${port}/refused` }), 'connect_failed');
  assert.equal(hosts.length, 4, 'TLS failure and proxy refusal must not reach the origin handler');
  fakeIp = true;
  const searchService = { kind: 'brave', endpoint: `https://public.test:${port}/search`, apiKey: 'fixture-key' };
  const beforeFakeIp = targets.length;
  for (const proxy of [undefined, proxyUrl]) {
    p.set({ allowPrivateHosts: [], searchService, ...(proxy ? { proxy } : {}) });
    for (const [name, args] of [['web_fetch', { url: `https://public.test:${port}/fake-ip` }], ['web_search', { query: 'fixture' }]]) {
      const result = await p.call(name, args);
      error(result, 'blocked_address');
      assert.match(result.content, /Fake-IP DNS/);
    }
  }
  assert.equal(targets.length, beforeFakeIp, 'Fake-IP rejection must happen before opening a proxy tunnel');
  p.set({ allowPrivateHosts: ['public.test'], proxy: proxyUrl, searchService });
  assert.match((await p.call('web_fetch', { url: `https://public.test:${port}/trusted-fake-ip` })).content, /pinned/);
  assert.equal((await p.call('web_search', { query: 'fixture' })).structuredContent.results[0].title, 'Public result');
  error(await p.call('web_fetch', { url: `https://wrong.test:${port}/untrusted-fake-ip` }), 'blocked_address');
  assert.deepEqual(targets.slice(beforeFakeIp), [`198.18.0.1:${port}`, `198.18.0.1:${port}`]);
  assert.deepEqual(hosts.slice(4), [[`public.test:${port}`, 'public.test'], [`public.test:${port}`, 'public.test']]);
});

test('concurrent fetches return the first completed page and paginate that same cache entry', async t => {
  const responses = [];
  const both = Promise.withResolvers();
  const base = await listen(t, http.createServer((_req, res) => {
    res.setHeader('Content-Type', 'text/plain');
    responses.push(res);
    if (responses.length === 2) both.resolve();
  }));
  const p = await plugin(t, { allowPrivateHosts: ['127.0.0.1'] });
  const calls = [p.call('web_fetch', { url: base }), p.call('web_fetch', { url: base })];
  await both.promise;
  responses[1].end('first completed');
  assert.match((await Promise.any(calls)).content, /first completed/);
  responses[0].end('second completed');
  for (const result of await Promise.all(calls)) assert.match(result.content, /first completed/);
  assert.match((await p.call('web_fetch', { url: base, start_char: 6 })).content, /\n\ncompleted$/);
  assert.equal(responses.length, 2, 'pagination must use the completed cache entry');
});

test('cancellation and unload close active sockets; already-cancelled calls make no request', async t => {
  let requests = 0;
  let entered = Promise.withResolvers();
  let closed = Promise.withResolvers();
  const address = await listen(t, http.createServer((_req, res) => {
    requests++;
    res.writeHead(200, { 'Content-Type': 'text/plain' });
    res.write('pending');
    res.once('close', closed.resolve);
    entered.resolve();
  }));
  const base = address.replace('127.0.0.1', 'cancel.test');
  t.mock.method(dns, 'lookup', async () => [{ address: '::1', family: 6 }, { address: '127.0.0.1', family: 4 }]);
  const p = await plugin(t, { allowPrivateHosts: ['cancel.test'] });
  error(await p.call('web_fetch', { url: base }, Promise.resolve()), 'cancelled');
  assert.equal(requests, 0);
  const cancelled = Promise.withResolvers();
  const pending = p.call('web_fetch', { url: base }, cancelled.promise);
  await entered.promise;
  cancelled.resolve();
  error(await pending, 'cancelled');
  await closed.promise;
  entered = Promise.withResolvers();
  closed = Promise.withResolvers();
  const unloading = p.call('web_fetch', { url: base + '/unload' });
  await entered.promise;
  await p.loaded.unload();
  error(await unloading, 'cancelled');
  await closed.promise;
});

test('decoded download limit, cache retention and expiration apply through the tool', async t => {
  const counts = new Map();
  const body = gzipSync('x'.repeat(5 * 1024 * 1024 + 1));
  const base = await listen(t, http.createServer((req, res) => {
    counts.set(req.url, (counts.get(req.url) ?? 0) + 1);
    res.writeHead(200, { 'Content-Type': 'text/plain', 'Content-Encoding': 'gzip' });
    res.end(body);
  }));
  const p = await plugin(t, { allowPrivateHosts: ['127.0.0.1'] });
  for (let i = 0; i < 11; i++) {
    const result = await p.call('web_fetch', { url: `${base}/${i}`, max_chars: 1000 });
    assert.equal(result.structuredContent.total, 5 * 1024 * 1024);
    assert.equal(result.structuredContent.download_limited, true);
    assert.match(result.content, /Download limit reached/);
  }
  await p.call('web_fetch', { url: base + '/10', start_char: 1000, max_chars: 1000 });
  assert.equal(counts.get('/10'), 1);
  await p.call('web_fetch', { url: base + '/0', max_chars: 1000 });
  assert.equal(counts.get('/0'), 2, 'oldest page must be evicted by the 50 MiB budget');
  t.mock.timers.enable({ apis: ['Date'], now: Date.now() });
  t.mock.timers.tick(15 * 60_000 + 1);
  await p.call('web_fetch', { url: base + '/10', max_chars: 1000 });
  assert.equal(counts.get('/10'), 2);
});

test('connect and whole-call deadlines abort stalled work', async t => {
  const p = await plugin(t, { allowPrivateHosts: [] });
  const resolving = Promise.withResolvers();
  const lookup = t.mock.method(dns, 'lookup', () => { resolving.resolve(); return new Promise(() => {}); });
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const connecting = p.call('web_fetch', { url: 'https://never.test/' });
  await resolving.promise;
  t.mock.timers.tick(10_001);
  error(await connecting, 'timeout');
  lookup.mock.restore();
  t.mock.timers.reset();
  const entered = Promise.withResolvers();
  const base = await listen(t, http.createServer((_req, res) => {
    res.writeHead(200, { 'Content-Type': 'text/plain' }); res.write('pending'); entered.resolve();
  }));
  p.set({ allowPrivateHosts: ['127.0.0.1'] });
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const reading = p.call('web_fetch', { url: base });
  await entered.promise;
  t.mock.timers.tick(30_001);
  error(await reading, 'timeout');
});
