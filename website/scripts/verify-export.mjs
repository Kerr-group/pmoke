import { access, readFile, stat } from 'node:fs/promises';
import path from 'node:path';
import { gzipSync } from 'node:zlib';

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
  'workers/config-validator.worker.js',
  'workers/signal.worker.js',
  'og.png',
  'robots.txt',
  'sitemap.xml',
  '.nojekyll',
  '_meta/payload.json',
];

for (const relative of required) await access(path.join(output, relative));

const English = await readFile(path.join(output, 'en/index.html'), 'utf8');
const Japanese = await readFile(path.join(output, 'ja/index.html'), 'utf8');
const JapaneseQuickstart = await readFile(path.join(output, 'ja/docs/quickstart/index.html'), 'utf8');
const sitemap = await readFile(path.join(output, 'sitemap.xml'), 'utf8');
const robots = await readFile(path.join(output, 'robots.txt'), 'utf8');
if (!English.includes('<html lang="en"')) throw new Error('English lang metadata is missing');
if (!Japanese.includes('<html lang="ja"')) throw new Error('Japanese lang metadata is missing');
if (!English.includes('/pmoke/_next/')) throw new Error('GitHub Pages basePath is missing');
if (!Japanese.includes('精密信号ラボ')) throw new Error('Japanese home content is missing');
if (!JapaneseQuickstart.includes('https://kerr-group.github.io/pmoke/ja/docs/quickstart/')) {
  throw new Error('Canonical project URL is missing');
}
if (!JapaneseQuickstart.includes('hrefLang="x-default"')) throw new Error('x-default metadata is missing');
if (!JapaneseQuickstart.includes('https://kerr-group.github.io/pmoke/og.png')) {
  throw new Error('Open Graph image is missing');
}
if (!sitemap.includes('https://kerr-group.github.io/pmoke/en/docs/quickstart/')) {
  throw new Error('Localized sitemap entry is missing');
}
if (!robots.includes('Sitemap: https://kerr-group.github.io/pmoke/sitemap.xml\n')) {
  throw new Error('Robots sitemap declaration is missing');
}

const wasm = await stat(path.join(output, 'wasm/pmoke_web_wasm_bg.wasm'));
if (wasm.size > 600 * 1024) throw new Error(`M3 Wasm raw budget exceeded: ${wasm.size} bytes`);
const wasmCompressed = gzipSync(await readFile(path.join(output, 'wasm/pmoke_web_wasm_bg.wasm')), {
  level: 9,
});
if (wasmCompressed.byteLength > 210 * 1024) {
  throw new Error(`M3 Wasm gzip budget exceeded: ${wasmCompressed.byteLength} bytes`);
}

const search = await stat(path.join(output, 'api/search'));
if (search.size > 900 * 1024) throw new Error(`M2 search budget exceeded: ${search.size} bytes`);

console.log(`Verified ${required.length} static artifacts under /pmoke/.`);
