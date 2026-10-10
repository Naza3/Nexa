import assert from 'node:assert/strict';
import { request as httpRequest } from 'node:http';

// Node's explicit loopback HTTP transport avoids ambient proxy settings. Only
// this one endpoint is permitted, redirects are never followed, and no external
// tool, filesystem operation or network lookup is dispatched by model output.
export function loopbackFetch(baseUrl, { byteFragmentation = false } = {}) {
  const endpoint = new URL(baseUrl);
  assert.equal(endpoint.protocol, 'http:');
  assert.equal(endpoint.hostname, '127.0.0.1');
  assert.equal(endpoint.pathname, '/v1');
  assert.equal(endpoint.username + endpoint.password + endpoint.search + endpoint.hash, '');
  return async (input, init) => {
    const url = new URL(typeof input === 'string' || input instanceof URL ? input : input.url);
    assert.equal(url.href, `${baseUrl}/chat/completions`);
    return new Promise((resolve, reject) => {
      const request = httpRequest(url, {
        method: init.method,
        headers: Object.fromEntries(new Headers(init.headers)),
        signal: init.signal,
      }, response => {
        const chunks = [];
        let bytesRead = 0;
        response.on('data', chunk => {
          bytesRead += chunk.length;
          if (bytesRead > 256 * 1024) {
            response.destroy(new Error('test_response_limit'));
            return;
          }
          chunks.push(chunk);
        });
        response.on('end', () => {
          const bytes = Buffer.concat(chunks);
          let offset = 0;
          const body = new ReadableStream({
            pull(controller) {
              if (offset === bytes.length) controller.close();
              else if (byteFragmentation) controller.enqueue(bytes.subarray(offset, ++offset));
              else { controller.enqueue(bytes); offset = bytes.length; }
            },
          });
          resolve(new Response(body, { status: response.statusCode, headers: response.headers }));
        });
        response.on('error', reject);
      });
      request.on('error', reject);
      request.end(init.body);
    });
  };
}

// toolcall_end is not authorization or a successful response: pi-ai can emit
// it before the stream's final error. Callers must wait for the complete stream.
export function executableMemoryCall(output, eventTypes) {
  assert.equal(eventTypes.at(-1), 'done', 'A successful final event is required');
  assert.ok(!eventTypes.includes('error'), 'An errored stream cannot execute tools');
  assert.equal(output.stopReason, 'toolUse');
  const calls = output.content.filter(block => block.type === 'toolCall');
  assert.equal(calls.length, 1);
  const call = calls[0];
  assert.equal(call.name, 'lookup_test_color');
  assert.deepEqual(call.arguments, { code: 'B7' });
  assert.match(call.id, /^[A-Za-z0-9_-]{1,64}$/);
  return call;
}

export async function collect(events) {
  const types = [];
  for await (const event of events) types.push(event.type);
  return { output: await events.result(), types };
}
