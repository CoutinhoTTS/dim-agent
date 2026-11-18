import {appendFileSync} from 'node:fs';
import http from 'node:http';

const DONE = 'data: [DONE]\n\n';
const MODEL_ID = 'fixture-model';

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function frame(delta, finishReason = null) {
  return `data: ${JSON.stringify({
    id: 'fixture',
    object: 'chat.completion.chunk',
    created: 0,
    model: MODEL_ID,
    choices: [{index: 0, delta, finish_reason: finishReason}],
  })}\n\n`;
}

/// The OpenAI-compatible reply for one scripted turn: streamed text in
/// `chunkSize` pieces, or tool calls the real runtime executes.
export function replyFrames(reply) {
  const frames = [];
  if (reply.toolCalls?.length) {
    frames.push(
      frame({
        tool_calls: reply.toolCalls.map((call, index) => ({
          index,
          id: call.id ?? `call-${index + 1}`,
          type: 'function',
          function: {name: call.name, arguments: JSON.stringify(call.arguments ?? {})},
        })),
      }),
    );
    frames.push(frame({}, reply.finishReason ?? 'tool_calls'));
    return frames;
  }
  const characters = Array.from(reply.text ?? '');
  const size = reply.chunkSize ?? Math.max(1, Math.ceil(characters.length / 4));
  for (let index = 0; index < characters.length; index += size) {
    frames.push(frame({content: characters.slice(index, index + size).join('')}));
  }
  frames.push(frame({}, reply.finishReason ?? 'stop'));
  return frames;
}

/// A model config selecting the fixture endpoint as the Session's model.
export function modelConfig(baseUrl) {
  return {
    defaultModel: {providerId: 'local', modelId: MODEL_ID},
    providers: [
      {
        id: 'local',
        kind: 'openai-compatible',
        baseUrl,
        models: [
          {
            id: MODEL_ID,
            contextWindow: 128000,
            maxOutputTokens: 4096,
            config: {maxTokens: 4096},
          },
        ],
      },
    ],
  };
}

/// The deterministic model endpoint. Everything else a scenario drives is the
/// production CLI and App Server; only the model's answers are scripted, so a
/// scenario's prompts map to fixed output. `respond(request, {index})` returns
/// `{text, toolCalls, chunkSize, chunkDelayMs, finishReason, status}`.
export async function startModelFixture({respond, logFile}) {
  const requests = [];
  const server = http.createServer((request, response) => {
    let body = '';
    request.setEncoding('utf8');
    request.on('data', (chunk) => {
      body += chunk;
    });
    request.on('end', async () => {
      let parsed = {};
      try {
        parsed = JSON.parse(body);
      } catch {
        parsed = {};
      }
      const entry = {
        index: requests.length,
        url: request.url,
        method: request.method,
        authorization: request.headers.authorization ?? request.headers['x-api-key'],
        body: parsed,
      };
      requests.push(entry);
      if (logFile) appendFileSync(logFile, `${JSON.stringify(entry)}\n`);
      // A provider's model listing, which the provider panel probes; every
      // other request is a chat completion the scenario scripts.
      if (request.method === 'GET' && request.url.endsWith('/models')) {
        response.writeHead(200, {'content-type': 'application/json'});
        response.end(JSON.stringify({data: [{id: MODEL_ID, context_window: 128000, max_output_tokens: 4096}]}));
        return;
      }
      let reply;
      try {
        reply = (await respond(parsed, {index: entry.index, requests, response})) ?? {};
      } catch (error) {
        reply = {status: 500, json: {error: {message: `fixture: ${error.message}`}}};
      }
      if (response.destroyed) return;
      if (reply.status && reply.status !== 200) {
        response.writeHead(reply.status, {'content-type': 'application/json'});
        response.end(JSON.stringify(reply.json ?? {error: {message: 'fixture failure'}}));
        return;
      }
      response.writeHead(200, {'content-type': 'text/event-stream', 'cache-control': 'no-cache'});
      try {
        for (const chunk of replyFrames(reply)) {
          if (reply.chunkDelayMs) await sleep(reply.chunkDelayMs);
          response.write(chunk);
        }
        response.end(DONE);
      } catch {
        response.destroy();
      }
    });
  });
  server.listen(0, '127.0.0.1');
  await new Promise((resolve, reject) => {
    server.once('listening', resolve);
    server.once('error', reject);
  });
  const {port} = server.address();
  return {
    url: `http://127.0.0.1:${port}/v1`,
    requests,
    close: () =>
      new Promise((resolve) => {
        server.closeAllConnections?.();
        server.close(() => resolve());
      }),
  };
}
