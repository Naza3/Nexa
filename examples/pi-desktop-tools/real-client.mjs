// Private one-shot client used only by run-real.py's owned temporary runtime.
// Connection credentials arrive over stdin, are never written into evidence,
// and cannot authorize any operation except this exact bounded in-memory tool.
import assert from 'node:assert/strict';
import { clearAmbientEnvironment, loadInputs, modelFor, syntheticUser, memoryAnswer } from './inputs.mjs';
import { loopbackFetch, collect, executableMemoryCall } from './transport.mjs';
clearAmbientEnvironment();
let input = '';
for await (const bytes of process.stdin) {
  input += bytes;
  assert.ok(input.length <= 16384);
}
const connection = JSON.parse(input);
input = '';
assert.equal(connection.kind, 'nexa-owned-temporary-tools-test');
const { stream, normalizeContext, memoryTool } = await loadInputs(process.argv[2]);
const model = modelFor(connection.baseUrl, 'qa-tools');
const fetch = loopbackFetch(connection.baseUrl);
const report = { schema_version: 1, status: 'failed', mode: 'real-nexa-model-pinned-pi-client', modelRequests: 0, toolExecutions: 0, piGuiTested: false, systemToolsExecuted: false, turns: [] };
const systemPrompt = '需要测试编号的颜色时调用 lookup_test_color。收到工具结果后，用一句中文说明编号、实际颜色和 fixture_tag。缺少编号时先询问，不猜测。';
async function run(messages) {
  assert.ok(++report.modelRequests <= 2);
  const result = await collect(stream(model, normalizeContext({ systemPrompt, tools: [memoryTool], messages }), {
    apiKey: connection.token, env: {}, maxTokens: 256, temperature: 0,
    cacheRetention: 'none', maxRetries: 0, timeoutMs: 180000, fetch,
  }));
  report.turns.push({ stopReason: result.output.stopReason, events: result.types, usage: result.output.usage });
  assert.ok(result.output.usage.input > 0);
  assert.ok(result.output.usage.input + 256 <= 2048, 'Actual full prompt must fit the selected context');
  return result;
}
try {
  const first = await run([syntheticUser]);
  const call = executableMemoryCall(first.output, first.types);
  report.toolExecutions++;
  const second = await run([syntheticUser, first.output, {
    role: 'toolResult', toolCallId: call.id, toolName: call.name,
    content: [{ type: 'text', text: JSON.stringify(memoryAnswer) }], isError: false, timestamp: 1,
  }]);
  assert.equal(second.types.at(-1), 'done');
  assert.equal(second.output.stopReason, 'stop');
  assert.ok(!second.output.content.some(block => block.type === 'toolCall'));
  const text = second.output.content.map(block => block.text ?? '').join('');
  assert.ok(['B7', '蓝色', 'fixture-b7-v1'].every(anchor => text.includes(anchor)));
  report.status = 'pass';
} catch (error) {
  report.failureKind = error?.name === 'AssertionError' ? 'contract_assertion' : 'client_error';
} finally {
  connection.token = '';
  console.log(JSON.stringify(report));
  process.exitCode = report.status === 'pass' ? 0 : 1;
}
