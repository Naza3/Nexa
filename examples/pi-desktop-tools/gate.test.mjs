import assert from 'node:assert/strict';
import test from 'node:test';
import { executableMemoryCall } from './transport.mjs';
const good = { stopReason: 'toolUse', content: [{ type: 'toolCall', id: 'call_fixture_0', name: 'lookup_test_color', arguments: { code: 'B7' } }] };
test('only a complete successful tool response opens the memory-tool gate', () => {
  assert.equal(executableMemoryCall(good, ['toolcall_end', 'done']).name, 'lookup_test_color');
});
test('toolcall_end before terminal error cannot execute even with valid arguments', () => {
  assert.throws(() => executableMemoryCall(good, ['toolcall_end', 'error']));
  assert.throws(() => executableMemoryCall({ ...good, stopReason: 'error' }, ['toolcall_end', 'done']));
  assert.throws(() => executableMemoryCall(good, ['toolcall_end']));
});
test('only the exact single bounded in-memory operation is allowed', () => {
  for (const content of [[], [good.content[0], good.content[0]], [{ ...good.content[0], name: 'Bash' }], [{ ...good.content[0], arguments: { code: 'B7', command: 'anything' } }]]) {
    assert.throws(() => executableMemoryCall({ ...good, content }, ['done']));
  }
});
