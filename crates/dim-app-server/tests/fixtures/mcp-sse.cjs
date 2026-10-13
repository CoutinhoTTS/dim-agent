// Official SDK legacy dual-endpoint server, bound only to loopback for tests.
const { createRequire } = require('node:module');
const { resolve } = require('node:path');
const { createServer } = require('node:http');
const sdk = createRequire(resolve(__dirname, '../../../../runtime/package.json'));
const { McpServer } = sdk('@modelcontextprotocol/sdk/server/mcp.js');
const { SSEServerTransport } = sdk('@modelcontextprotocol/sdk/server/sse.js');
const { z } = sdk('zod');
const sessions = new Map();
const stats = { gets: 0, calls: 0, cancelled: 0 };

const http = createServer(async (req, res) => {
  try {
    if (req.headers.origin || !req.headers.host?.startsWith('127.0.0.1:')) {
      res.writeHead(403).end();
      return;
    }
    const url = new URL(req.url, 'http://127.0.0.1');
    if (req.method === 'GET' && url.pathname === '/stats') {
      res.writeHead(200, { 'content-type': 'application/json' }).end(JSON.stringify({ ...stats, active: sessions.size }));
      return;
    }
    if (req.headers.authorization !== 'Bearer fixture-only' || req.headers['x-fixture'] !== 'custom-header') {
      res.writeHead(401).end();
      return;
    }
    if (req.method === 'GET' && url.pathname === '/sse') {
      stats.gets++;
      const transport = new SSEServerTransport('/messages', res);
      const server = new McpServer({ name: 'legacy-sse-interop', version: '1' });
      server.registerTool('compute', { inputSchema: { a: z.number(), b: z.number() } }, async ({ a, b }) => {
        stats.calls++;
        await server.server.sendToolListChanged();
        return { content: [{ type: 'text', text: String(a * 100 + b) }], structuredContent: { cursor: 'page-7' } };
      });
      server.registerTool('slow', { inputSchema: {} }, async (_, { signal }) => {
        stats.calls++;
        await new Promise(resolve => signal.addEventListener('abort', resolve, { once: true }));
        stats.cancelled++;
        return { content: [{ type: 'text', text: 'cancelled' }] };
      });
      server.registerTool('drop', { inputSchema: {} }, async () => {
        stats.calls++;
        await transport.close();
        return { content: [] };
      });
      sessions.set(transport.sessionId, transport);
      res.on('close', () => sessions.delete(transport.sessionId));
      await server.connect(transport);
    } else if (req.method === 'POST' && url.pathname === '/messages') {
      const transport = sessions.get(url.searchParams.get('sessionId'));
      if (transport) await transport.handlePostMessage(req, res);
      else res.writeHead(404).end();
    } else {
      res.writeHead(404).end();
    }
  } catch {
    if (!res.headersSent) res.writeHead(500);
    res.end();
  }
});
http.listen(0, '127.0.0.1', () => process.stdout.write(`http://127.0.0.1:${http.address().port}\n`));
process.stdin.resume();
process.stdin.on('end', async () => {
  await Promise.all([...sessions.values()].map(transport => transport.close()));
  http.close();
  http.closeAllConnections();
});
