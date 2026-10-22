// Pinned official serializer/parser against synthetic HTTP only. Not PI GUI,
// not Nexa inference, and never an executor for arbitrary model-requested tools.
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { once } from 'node:events';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { clearAmbientEnvironment, loadInputs, modelFor, syntheticUser, syntheticSystem, memoryAnswer } from './inputs.mjs';
import { loopbackFetch, collect, executableMemoryCall } from './transport.mjs';

const [clientRoot, outputDirectory] = process.argv.slice(2);
assert.ok(clientRoot, 'Usage: node verify.mjs CLIENT_ROOT [NEW_CAPTURE_DIRECTORY]');
clearAmbientEnvironment();
const { stream, normalizeContext, declarations, memoryTool, lock } = await loadInputs(clientRoot);
const watchdog = setTimeout(() => { console.error('Synthetic validation timeout'); process.exit(2); }, 30_000);
watchdog.unref();
let scenario = 'ordinary';
const requests = [];
const outcomes = [];
const token = 'synthetic-only-test-token';
const server = createServer(async (req, res) => {
  let raw = '';
  for await (const part of req) {
    raw += part;
    if (raw.length > 1024 * 1024) { res.writeHead(413).end(); return; }
  }
  assert.equal(req.url, '/v1/chat/completions');
  assert.equal(req.headers.authorization, `Bearer ${token}`);
  requests.push({ scenario, body: JSON.parse(raw) });
  res.writeHead(200, { 'content-type': 'text/event-stream' });
  const chunk = (delta, finish_reason = null) => ({
    id: 'chatcmpl-fixture', object: 'chat.completion.chunk', created: 1, model: 'fixture',
    choices: [{ index: 0, delta, finish_reason }],
  });
  const frames = [chunk({ role: 'assistant' })];
  if (scenario === 'tool-first' || scenario === 'truncated') {
    frames.push(
      chunk({ tool_calls: [{ index: 0, id: 'call_fixture_0', type: 'function', function: { name: 'lookup_test_color', arguments: '' } }] }),
      chunk({ tool_calls: [{ index: 0, function: { arguments: '{"code":' } }] }),
    );
    if (scenario !== 'truncated') frames.push(
      chunk({ tool_calls: [{ index: 0, function: { arguments: '"B7"}' } }] }),
      chunk({}, 'tool_calls'),
    );
  } else {
    frames.push(chunk({ content: scenario === 'tool-second' ? 'B7 是蓝色，fixture-b7-v1。' : '你好，普通文本。' }), chunk({}, 'stop'));
  }
  if (scenario !== 'truncated') frames.push({
    id: 'chatcmpl-fixture', object: 'chat.completion.chunk', created: 1, model: 'fixture',
    choices: [], usage: { prompt_tokens: 200, completion_tokens: 20, total_tokens: 220 },
  });
  for (const frame of frames) res.write(`data: ${JSON.stringify(frame)}\n\n`);
  if (scenario !== 'truncated') res.write('data: [DONE]\n\n');
  res.end();
});
server.listen(0, '127.0.0.1');
await once(server, 'listening');
const baseUrl = `http://127.0.0.1:${server.address().port}/v1`;
const model = modelFor(baseUrl);
const fetch = loopbackFetch(baseUrl, { byteFragmentation: true });
async function run(name, tools, messages = [syntheticUser]) {
  scenario = name;
  const result = await collect(stream(model, normalizeContext({ systemPrompt: syntheticSystem, tools, messages }), {
    apiKey: token, env: {}, maxTokens: 256, temperature: 0, cacheRetention: 'none', maxRetries: 0, timeoutMs: 5000, fetch,
  }));
  outcomes.push({ scenario: name, events: result.types, stopReason: result.output.stopReason });
  return result;
}
try {
  assert.equal((await run('ordinary')).output.stopReason, 'stop');
  assert.equal((await run('core-tools', declarations)).output.stopReason, 'stop');
  const first = await run('tool-first', [memoryTool]);
  const call = executableMemoryCall(first.output, first.types);
  // The only dispatch is an exact constant in-memory fixture, after final done.
  const toolExecutions = 1;
  const second = await run('tool-second', [memoryTool], [syntheticUser, first.output, {
    role: 'toolResult', toolCallId: call.id, toolName: call.name,
    content: [{ type: 'text', text: JSON.stringify(memoryAnswer) }], isError: false, timestamp: 1,
  }]);
  assert.equal(second.output.stopReason, 'stop');
  assert.ok(second.output.content.some(block => block.text?.includes('fixture-b7-v1')));
  const bad = await run('truncated', [memoryTool]);
  assert.equal(bad.output.stopReason, 'error');
  assert.ok(bad.types.indexOf('toolcall_end') < bad.types.indexOf('error'));
  assert.throws(() => executableMemoryCall(bad.output, bad.types));
  const core = requests.find(item => item.scenario === 'core-tools').body;
  assert.equal(core.store, false);
  assert.equal(core.max_completion_tokens, 256);
  assert.equal(core.tools.length, 5);
  assert.deepEqual(core.tools[0].function.parameters.required, []);
  assert.equal(core.tools[4].function.parameters.properties.todos.items.properties.status.anyOf.length, 4);
  assert.equal(core.tools[4].function.strict, undefined);
  for (const request of requests) {
    const baseline = JSON.parse(await readFile(new URL(`../../tests/fixtures/pi-desktop/${request.scenario}.json`, import.meta.url), 'utf8'));
    assert.deepEqual(request.body, baseline, 'Captured official serializer differs from reviewed fixture');
  }
  const report = {
    schema_version: 1, status: 'pass', client: '@earendil-works/pi-ai', version: '1.0.1',
    piDesktopSource: lock.pi_desktop.ref, byteFragmentation: 1, toolExecutions,
    scope: 'Pinned serializer/parser with source-extracted schemas and synthetic descriptions/responses; no PI GUI, agent runtime, Nexa inference or system tools.',
    outcomes,
  };
  if (outputDirectory) {
    const out = resolve(outputDirectory);
    await mkdir(out, { recursive: true });
    for (const request of requests) await writeFile(join(out, `${request.scenario}.json`), JSON.stringify(request.body, null, 2) + '\n');
    await writeFile(join(out, 'report.json'), JSON.stringify(report, null, 2) + '\n');
  }
  console.log(JSON.stringify(report, null, 2));
} finally {
  clearTimeout(watchdog);
  server.close();
  server.closeAllConnections();
}
