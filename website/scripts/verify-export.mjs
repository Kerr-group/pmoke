import { access, readFile, stat } from 'node:fs/promises';
import path from 'node:path';

const output = path.resolve('out');
const required = [
  'index.html',
  'en/index.html',
  'ja/index.html',
  'en/docs/index.html',
  'ja/docs/index.html',
  'en/docs/quickstart/index.html',
  'ja/docs/quickstart/index.html',
  'llms.txt',
  'llms-full.txt',
  'api/search',
  'llm/en/content.md',
  'llm/ja/content.md',
  'wasm/pmoke_web_wasm.js',
  'wasm/pmoke_web_wasm_bg.wasm',
  'workers/signal.worker.js',
  '.nojekyll',
  '_meta/payload.json',
];

for (const relative of required) await access(path.join(output, relative));

const English = await readFile(path.join(output, 'en/index.html'), 'utf8');
const Japanese = await readFile(path.join(output, 'ja/index.html'), 'utf8');
if (!English.includes('<html lang="en"')) throw new Error('English lang metadata is missing');
if (!Japanese.includes('<html lang="ja"')) throw new Error('Japanese lang metadata is missing');
if (!English.includes('/pmoke/_next/')) throw new Error('GitHub Pages basePath is missing');
if (!Japanese.includes('精密信号ラボ')) throw new Error('Japanese home content is missing');

const wasm = await stat(path.join(output, 'wasm/pmoke_web_wasm_bg.wasm'));
if (wasm.size > 200 * 1024) throw new Error(`M0 Wasm budget exceeded: ${wasm.size} bytes`);

const search = await stat(path.join(output, 'api/search'));
if (search.size > 500 * 1024) throw new Error(`M0 search budget exceeded: ${search.size} bytes`);

console.log(`Verified ${required.length} static artifacts under /pmoke/.`);
