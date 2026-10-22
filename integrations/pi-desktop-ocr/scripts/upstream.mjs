import { spawnSync } from 'node:child_process';
import { access, mkdir, readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
const command = process.argv[2];
if (!['check', 'pack'].includes(command)) throw new Error('Expected check or pack');
const cli = process.env.PI_PLUGIN_DEVKIT_CLI || '/workspace/onboarding/pi-ocr/upstream-devkit/packages/plugin-devkit/dist/cli.js';
try { await access(cli); } catch { throw new Error('Set PI_PLUGIN_DEVKIT_CLI to the built, pinned PI Desktop plugin-devkit dist/cli.js (see README).'); }
await mkdir('dist', { recursive: true });
const args = [cli, command, resolve('build/plugin'), ...(command === 'pack' ? ['--out', resolve('dist')] : [])];
const result = spawnSync(process.execPath, args, { stdio: 'inherit' });
if (result.status !== 0) process.exit(result.status ?? 1);
if (command === 'pack') {
  const { createHash } = await import('node:crypto');
  const manifest = JSON.parse(await readFile('manifest.json', 'utf8'));
  const name = `${manifest.id}-${manifest.version}.piplug`;
  const bytes = await readFile(`dist/${name}`);
  const hash = createHash('sha256').update(bytes).digest('hex');
  await writeFile(`dist/${name}.sha256`, `${hash}  ${name}\n`);
}
