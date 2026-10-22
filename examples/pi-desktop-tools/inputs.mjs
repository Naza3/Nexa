import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

export async function loadInputs(clientRoot) {
  clientRoot = resolve(clientRoot);
  const lock = JSON.parse(await readFile(new URL('./upstream-lock.json', import.meta.url), 'utf8'));
  for (const entry of lock.source_files) {
    const bytes = await readFile(join(clientRoot, 'upstream', entry.file));
    assert.equal(createHash('sha256').update(bytes).digest('hex'), entry.sha256, 'Source identity mismatch');
  }
  const npmLock = await readFile(join(clientRoot, 'package-lock.json'));
  assert.equal(createHash('sha256').update(npmLock).digest('hex'), lock.pi_ai.package_lock_sha256, 'Client dependency lock mismatch');
  const packageRoot = join(clientRoot, 'node_modules/@earendil-works/pi-ai');
  for (const [file, sha256] of Object.entries(lock.patched_files_sha256)) {
    const bytes = await readFile(join(packageRoot, file));
    assert.equal(createHash('sha256').update(bytes).digest('hex'), sha256, 'Patched client identity mismatch');
  }
  const manifest = JSON.parse(await readFile(join(packageRoot, 'package.json'), 'utf8'));
  assert.equal(manifest.version, '1.0.1');
  const { Type } = await import(pathToFileURL(join(packageRoot, 'dist/index.js')));
  const { stream } = await import(pathToFileURL(join(packageRoot, 'dist/api/openai-completions.js')));
  const { normalizeContext } = await import(pathToFileURL(join(packageRoot, 'dist/utils/transcript.js')));
  const { todoWriteDescription, todoWriteParameters } = await import(pathToFileURL(join(clientRoot, 'upstream/todo-tool.ts')));
  const { withExplicitRequired } = await import(pathToFileURL(join(clientRoot, 'upstream/tool-schema.ts')));
  const source = await readFile(join(clientRoot, 'upstream/runtime.ts'), 'utf8');
  // Only hash-verified, official declarative schema expressions are evaluated.
  // The surrounding runtime/host tools are never imported or executed.
  const pathParam = description => Type.Optional(Type.String({ description }));
  const aliasParam = canonical => Type.Optional(Type.String({ description: `Alias for \`${canonical}\`.` }));
  const start = source.indexOf('const parameters: Record');
  const expression = source.slice(source.indexOf('      Read: {', start), source.indexOf('      Write: {', start));
  const definitions = Function('Type', 'pathParam', 'aliasParam', `return ({${expression}})`)(Type, pathParam, aliasParam);
  const askStart = source.indexOf('      parameters: Type.Object({', source.indexOf('const askTool: AgentTool'));
  const askExpression = source.slice(askStart + '      parameters: '.length, source.indexOf(',\n      executionMode:', askStart));
  const askParameters = Function('Type', `return ${askExpression}`)(Type);
  const declarations = ['Read', 'Glob', 'Grep'].map(name => withExplicitRequired({
    name,
    description: `Synthetic validation declaration using official PI ${name} parameter schema. No tool execution.`,
    parameters: Type.Object(definitions[name]),
  }));
  declarations.push(
    withExplicitRequired({ name: 'asktool', description: 'Synthetic validation declaration; no UI execution.', parameters: askParameters }),
    withExplicitRequired({ name: 'TodoWrite', description: todoWriteDescription, parameters: Type.Object(todoWriteParameters) }),
  );
  const memoryTool = {
    name: 'lookup_test_color',
    description: 'Read a synthetic in-memory table. No files, network or system operations.',
    parameters: Type.Object({ code: Type.String({ enum: ['A1', 'B7', 'C9'] }) }, { additionalProperties: false }),
  };
  return { stream, normalizeContext, declarations, memoryTool, lock };
}

export function modelFor(baseUrl, id = 'fixture') {
  // Match PI's generic OpenAI-completions projection: do not turn off store or
  // change maxTokensField to hide compatibility differences in the real client.
  return {
    id, name: 'Synthetic', api: 'openai-completions', provider: 'nexa-fixture', baseUrl,
    reasoning: false, input: ['text'],
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    contextWindow: 2048, maxTokens: 256, compat: { supportsDeveloperRole: false },
  };
}

export function clearAmbientEnvironment() {
  for (const key of Object.keys(process.env)) {
    if (!['PATH', 'HOME', 'TMP', 'TEMP', 'SystemRoot', 'SYSTEMROOT', 'WINDIR'].includes(key)) delete process.env[key];
  }
}

export const syntheticUser = { role: 'user', content: '请查询 B7 的颜色和 fixture_tag。', timestamp: 0 };
export const syntheticSystem = 'Synthetic validation. Use the memory lookup only when requested.';
export const memoryAnswer = { code: 'B7', color: '蓝色', fixture_tag: 'fixture-b7-v1' };
