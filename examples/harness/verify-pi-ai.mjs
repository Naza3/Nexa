// Exercises the pinned official pi-ai serializer/parser. No DSH agent is loaded.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { once } from 'node:events';
import { resolve, join } from 'node:path';
import { pathToFileURL } from 'node:url';

const watchdog = setTimeout(() => { console.error('Synthetic client test timed out'); process.exit(2); }, 15_000);
watchdog.unref();
// Dependencies receive no ambient cloud credentials, proxy or telemetry settings.
const allowedEnv = new Set(['PATH', 'SystemRoot', 'SYSTEMROOT', 'WINDIR', 'TEMP', 'TMP', 'HOME', 'NEXA_TEST_TOKEN']);
for (const key of Object.keys(process.env)) if (!allowedEnv.has(key)) delete process.env[key];
const args = process.argv.slice(2);
assert.ok(args.length === 1 || args.length === 2, 'Usage: node verify-pi-ai.mjs CLIENT_ROOT [SYNTHETIC_RUNTIME_BASE_URL]');
const clientRoot = resolve(args[0]);
const packageRoot = join(clientRoot, 'node_modules/@earendil-works/pi-ai');
const manifest = JSON.parse(await readFile(join(packageRoot, 'package.json'), 'utf8'));
assert.equal(manifest.version, '0.87.1');
const fixture = JSON.parse(await readFile(new URL('./fixtures/text-request.json', import.meta.url), 'utf8'));
const responseText = '你好🙂\n"\\';
const usage = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } };
let scenario = 'stop';
let requests = [];
let server;
let baseUrl = args[1];
const runtimeMode = baseUrl !== undefined;
const token = runtimeMode ? process.env.NEXA_TEST_TOKEN : 'nexa-synthetic-wire-token';
delete process.env.NEXA_TEST_TOKEN;
assert.ok(token, 'Synthetic runtime test token is required');
if (!runtimeMode) {
  server = createServer(async (req, res) => {
    let body = '';
    for await (const part of req) {
      body += part;
      if (body.length > 1024 * 1024) { res.writeHead(413).end(); return; }
    }
    requests.push({ method: req.method, path: req.url, body: JSON.parse(body), authorized: req.headers.authorization === `Bearer ${token}` });
    if (scenario === '401' || scenario === '404') {
      res.writeHead(Number(scenario), { 'content-type': 'application/json' });
      res.end(JSON.stringify({ error: { message: scenario === '401' ? 'A valid Bearer API token is required.' : 'The registered model was not found.', type: 'invalid_request_error', param: scenario === '404' ? 'model' : null, code: scenario === '401' ? 'invalid_api_key' : 'model_not_found' } }));
      return;
    }
    res.writeHead(200, { 'content-type': 'text/event-stream' });
    const chunk = (delta, finish_reason = null) => ({ id: 'chatcmpl-synthetic', object: 'chat.completion.chunk', created: 1, model: 'fixture', choices: [{ index: 0, delta, finish_reason }] });
    const frames = [chunk({ role: 'assistant' }), chunk({}), chunk({ content: responseText })];
    if (scenario === 'stream-error') {
      frames.push({ error: { message: 'The runtime could not complete this operation.', type: 'server_error', param: null, code: 'internal_error' } });
    } else if (scenario !== 'missing-finish' && scenario !== 'partial-eof') {
      frames.push(chunk({}, scenario));
      frames.push({ id: 'chatcmpl-synthetic', object: 'chat.completion.chunk', created: 1, model: 'fixture', choices: [], usage: { prompt_tokens: 3, completion_tokens: 1, total_tokens: 4 } });
    }
    for (const frame of frames) res.write(`data: ${JSON.stringify(frame)}\n\n`);
    if (scenario !== 'partial-eof' && scenario !== 'stream-error') res.write('data: [DONE]\n\n');
    res.end();
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  baseUrl = `http://127.0.0.1:${server.address().port}/v1`;
}
const endpoint = new URL(baseUrl);
assert.equal(endpoint.hostname, '127.0.0.1');
assert.equal(endpoint.protocol, 'http:');
assert.equal(endpoint.pathname, '/v1');
assert.equal(endpoint.username + endpoint.password + endpoint.search + endpoint.hash, '');
const nativeFetch = globalThis.fetch;
// Deny any external request and redirect. Deterministic one-byte body fragments
// exercise the official SDK's UTF-8 and SSE parsers, even if TCP coalesces writes.
globalThis.fetch = async (input, options) => {
  const url = new URL(typeof input === 'string' || input instanceof URL ? input : input.url);
  assert.equal(url.href, `${baseUrl}/chat/completions`, 'Only the synthetic test endpoint is permitted');
  const response = await nativeFetch(input, { ...options, redirect: 'error' });
  if (!response.ok || !response.body) return response;
  const reader = response.body.getReader();
  let pending = new Uint8Array();
  let offset = 0;
  const body = new ReadableStream({
    async pull(controller) {
      if (offset === pending.length) {
        const next = await reader.read();
        if (next.done) { controller.close(); return; }
        pending = next.value;
        offset = 0;
      }
      controller.enqueue(pending.subarray(offset, ++offset));
    },
    cancel(reason) { return reader.cancel(reason); },
  });
  return new Response(body, { status: response.status, headers: response.headers });
};

try {
  const { stream } = await import(pathToFileURL(join(packageRoot, 'dist/api/openai-completions.js')));
  const { normalizeContext } = await import(pathToFileURL(join(packageRoot, 'dist/utils/transcript.js')));
  const model = { id: 'fixture', name: 'Synthetic text fixture', api: 'openai-completions', provider: 'nexa-local', baseUrl, reasoning: false, input: ['text'], cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }, contextWindow: 2048, maxTokens: 128, compat: { supportsStore: false, supportsDeveloperRole: false, supportsReasoningEffort: false, supportsUsageInStreaming: true, supportsFinishReason: true, supportsStrictMode: false, maxTokensField: 'max_tokens' } };
  const context = normalizeContext({ systemPrompt: fixture.messages[0].content, messages: fixture.messages.slice(1).map(message => message.role === 'assistant' ? { ...message, content: [{ type: 'text', text: message.content }], api: model.api, provider: model.provider, model: model.id, usage, stopReason: 'stop', timestamp: 0 } : { ...message, timestamp: 0 }) });
  const outcomes = [];
  for (scenario of runtimeMode ? ['runtime-stop'] : ['stop', 'length', 'missing-finish', 'partial-eof', 'stream-error', '401', '404']) {
    let payload;
    const events = stream(model, context, { apiKey: token, env: {}, maxTokens: 128, temperature: 0, cacheRetention: 'none', maxRetries: 0, timeoutMs: 5000, fetch: globalThis.fetch, onPayload(value) { payload = JSON.parse(JSON.stringify(value)); } });
    const received = [];
    for await (const event of events) received.push(event.type);
    const output = await events.result();
    assert.deepEqual(payload, fixture, 'Official serializer diverged from the reviewed text fixture');
    const success = ['stop', 'length', 'runtime-stop'].includes(scenario);
    if (success) {
      assert.equal(output.stopReason, scenario === 'length' ? 'length' : 'stop');
      assert.equal(output.content.map(block => block.text ?? '').join(''), responseText);
      assert.equal(output.usage.input, 3);
      assert.equal(output.usage.output, 1);
      assert.equal(output.usage.totalTokens, 4);
      assert.equal(received.filter(type => type === 'done').length, 1);
      assert.ok(!received.includes('error'));
    } else {
      assert.equal(output.stopReason, 'error');
      assert.equal(received.filter(type => type === 'error').length, 1);
      assert.ok(!received.includes('done'));
      if (['missing-finish', 'partial-eof'].includes(scenario)) assert.match(output.errorMessage, /without finish_reason/);
      if (scenario === '401' || scenario === '404') assert.ok(output.errorMessage.includes(scenario));
    }
    outcomes.push({ scenario, stopReason: output.stopReason, terminal: received.at(-1) });
  }
  if (!runtimeMode) {
    assert.equal(requests.length, outcomes.length, 'Retries are disabled');
    for (const request of requests) {
      assert.equal(request.method, 'POST');
      assert.equal(request.path, '/v1/chat/completions');
      assert.equal(request.authorized, true);
      assert.deepEqual(request.body, fixture);
    }
  }
  console.log(JSON.stringify({ client: manifest.name, version: manifest.version, mode: runtimeMode ? 'pi-ai-to-nexa-synthetic-executor' : 'pi-ai-to-synthetic-http-server', dshAdapterExecuted: false, realModelTested: false, byteFragmentation: 1, outcomes }, null, 2));
} finally {
  clearTimeout(watchdog);
  globalThis.fetch = nativeFetch;
  if (server) { server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); }
}
