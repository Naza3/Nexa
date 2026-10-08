import { build } from 'esbuild';
import { mkdir, readFile, copyFile, rm, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { join, dirname, resolve } from 'node:path';
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const out = join(root, 'build/plugin');
await rm(out, { recursive: true, force: true });
await mkdir(join(out, 'renderer'), { recursive: true });
await mkdir(join(out, 'licenses'), { recursive: true });
await build({ entryPoints: [join(root, 'src/main.mjs')], outfile: join(out, 'main.cjs'), bundle: true, platform: 'node', format: 'cjs', target: 'node22', legalComments: 'eof' });
await build({ entryPoints: [join(root, 'renderer/app.js')], outfile: join(out, 'renderer/app.js'), bundle: true, platform: 'browser', format: 'iife', target: 'chrome130', legalComments: 'eof' });
for (const file of ['manifest.json', 'README.md', 'THIRD_PARTY_NOTICES.md', 'LICENSE']) await copyFile(join(root, file), join(out, file));
for (const file of ['index.html', 'style.css']) await copyFile(join(root, 'renderer', file), join(out, 'renderer', file));
for (const [pkg, file] of [['marked', 'LICENSE'], ['dompurify', 'LICENSE'], ['dompurify', 'LICENSE-MPL'], ['smol-toml', 'LICENSE']]) {
  await copyFile(join(root, 'node_modules', pkg, file), join(out, 'licenses', `${pkg}-${file}.txt`));
}
await copyFile(join(root, 'licenses/side-chat-MIT.txt'), join(out, 'licenses/side-chat-MIT.txt'));
const pkg = JSON.parse(await readFile(join(root, 'package.json'), 'utf8'));
await writeFile(join(out, 'build-info.json'), JSON.stringify({ pluginVersion: pkg.version, piDesktopReference: '779e16d9c3ca2e966a7ae3db9dd0707243a2831f', dependencies: pkg.dependencies }, null, 2) + '\n');
console.log('Plugin built: build/plugin');
