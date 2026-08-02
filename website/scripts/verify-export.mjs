import { access, readFile, readdir, stat } from 'node:fs/promises';
import { createHash } from 'node:crypto';
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
  'en/docs/interactive/waveform-analyzer/index.html',
  'ja/docs/interactive/waveform-analyzer/index.html',
  'llms.txt',
  'llms-full.txt',
  'llms-en.txt',
  'llms-ja.txt',
  'ai-index.json',
  'api/search',
  'api/semantic-search',
  'llm/en/content.md',
  'llm/ja/content.md',
  'wasm/pmoke_web_wasm.js',
  'wasm/pmoke_web_wasm_bg.wasm',
  'workers/config-validator.worker.js',
  'workers/signal.worker.js',
  'workers/waveform-analyzer.worker.js',
  'fixtures/LICENSE',
  'fixtures/m4-synthetic-reference.json',
  'og.png',
  'robots.txt',
  'sitemap.xml',
  '.nojekyll',
  '_meta/payload.json',
  '_meta/sbom.cdx.json',
  'SHA256SUMS',
];

for (const relative of required) await access(path.join(output, relative));

const English = await readFile(path.join(output, 'en/index.html'), 'utf8');
const Japanese = await readFile(path.join(output, 'ja/index.html'), 'utf8');
const JapaneseQuickstart = await readFile(path.join(output, 'ja/docs/quickstart/index.html'), 'utf8');
const EnglishAnalyzer = await readFile(path.join(output, 'en/docs/interactive/waveform-analyzer/index.html'), 'utf8');
const JapaneseAnalyzer = await readFile(path.join(output, 'ja/docs/interactive/waveform-analyzer/index.html'), 'utf8');
const sitemap = await readFile(path.join(output, 'sitemap.xml'), 'utf8');
const robots = await readFile(path.join(output, 'robots.txt'), 'utf8');
if (!English.includes('<html lang="en"')) throw new Error('English lang metadata is missing');
if (!Japanese.includes('<html lang="ja"')) throw new Error('Japanese lang metadata is missing');
if (!English.includes('Content-Security-Policy')) throw new Error('Static content security policy is missing');
if (!English.includes('strict-origin-when-cross-origin')) throw new Error('Referrer policy is missing');
if (!English.includes('/pmoke/_next/')) throw new Error('GitHub Pages basePath is missing');
if (!Japanese.includes('精密信号ラボ')) throw new Error('Japanese home content is missing');
if (!EnglishAnalyzer.includes('waveform-analyzer')) throw new Error('English waveform tool is missing');
if (!JapaneseAnalyzer.includes('波形アナライザー')) throw new Error('Japanese waveform tool is missing');
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
if (wasm.size > 600 * 1024) throw new Error(`M4 Wasm raw budget exceeded: ${wasm.size} bytes`);
const wasmCompressed = gzipSync(await readFile(path.join(output, 'wasm/pmoke_web_wasm_bg.wasm')), {
  level: 9,
});
if (wasmCompressed.byteLength > 210 * 1024) {
  throw new Error(`M4 Wasm gzip budget exceeded: ${wasmCompressed.byteLength} bytes`);
}

const analyzerWorker = await stat(path.join(output, 'workers/waveform-analyzer.worker.js'));
if (analyzerWorker.size > 32 * 1024) {
  throw new Error(`M4 analysis Worker budget exceeded: ${analyzerWorker.size} bytes`);
}

const coreFixture = await readFile(
  path.resolve('../crates/pmoke-analysis-core/tests/fixtures/m4-synthetic-reference.json'),
);
const publicFixture = await readFile(path.resolve('public/fixtures/m4-synthetic-reference.json'));
const exportedFixture = await readFile(path.join(output, 'fixtures/m4-synthetic-reference.json'));
if (!coreFixture.equals(publicFixture) || !coreFixture.equals(exportedFixture)) {
  throw new Error('M4 public golden fixture differs from the analysis-core fixture');
}
const fixture = JSON.parse(coreFixture.toString('utf8'));
if (fixture.expected?.harmonics?.length !== 6 || fixture.license !== 'CC0-1.0') {
  throw new Error('M4 golden fixture metadata or harmonic outputs are incomplete');
}

const search = await stat(path.join(output, 'api/search'));
if (search.size > 900 * 1024) throw new Error(`M2 search budget exceeded: ${search.size} bytes`);

const semanticPath = path.join(output, 'api/semantic-search');
const semanticFile = await readFile(semanticPath);
if (semanticFile.byteLength > 512 * 1024) {
  throw new Error(`M5 concept index budget exceeded: ${semanticFile.byteLength} bytes`);
}
const semantic = JSON.parse(semanticFile.toString('utf8'));
if (semantic.model !== 'pmoke-domain-v1' || semantic.records?.length < 100) {
  throw new Error('M5 concept index metadata or records are incomplete');
}
if (semantic.records.some((record) => record.embedding?.length !== semantic.dimensions)) {
  throw new Error('M5 concept index contains an invalid vector');
}

const chunkRoot = path.join(output, '_next/static/chunks');
const chunks = await recursiveFiles(chunkRoot);
const semanticChunks = [];
for (const chunk of chunks.filter((file) => file.endsWith('.js'))) {
  const bytes = await readFile(chunk);
  const source = bytes.toString('utf8');
  if (source.includes('createHybridEngine') && source.includes('hybridWeights')) semanticChunks.push(bytes);
}
if (semanticChunks.length !== 1) throw new Error(`expected one lazy semantic chunk, found ${semanticChunks.length}`);
const semanticGzip = gzipSync(semanticChunks[0], { level: 9 }).byteLength;
if (semanticGzip > 100 * 1024) throw new Error(`M5 semantic chunk gzip budget exceeded: ${semanticGzip} bytes`);

const sbom = JSON.parse(await readFile(path.join(output, '_meta/sbom.cdx.json'), 'utf8'));
if (sbom.bomFormat !== 'CycloneDX' || sbom.specVersion !== '1.6' || sbom.components?.length < 100) {
  throw new Error('M6 CycloneDX SBOM is missing or incomplete');
}
const payload = JSON.parse(await readFile(path.join(output, '_meta/payload.json'), 'utf8'));
if (!payload.sourceRevision || payload.sourceRevision !== sbom.metadata.component.version) {
  throw new Error('M6 release metadata does not identify one source revision');
}
await access(path.join(output, '_next/static', payload.sourceRevision, '_buildManifest.js'));
const checksumLines = (await readFile(path.join(output, 'SHA256SUMS'), 'utf8')).trim().split('\n');
const checksummed = new Set();
for (const line of checksumLines) {
  const match = /^([a-f0-9]{64})  (.+)$/u.exec(line);
  if (!match) throw new Error(`invalid checksum entry: ${line}`);
  const [, expected, relative] = match;
  if (checksummed.has(relative)) throw new Error(`duplicate checksum entry: ${relative}`);
  checksummed.add(relative);
  const actual = createHash('sha256').update(await readFile(path.join(output, relative))).digest('hex');
  if (actual !== expected) throw new Error(`checksum mismatch: ${relative}`);
}
const exportedFiles = (await recursiveFiles(output))
  .map((file) => path.relative(output, file).split(path.sep).join('/'))
  .filter((file) => file !== 'SHA256SUMS');
if (checksummed.size !== exportedFiles.length || exportedFiles.some((file) => !checksummed.has(file))) {
  throw new Error('M6 checksum manifest does not cover the complete static export');
}
for (const relative of exportedFiles.filter((file) => file.endsWith('.html'))) {
  const html = await readFile(path.join(output, relative), 'utf8');
  const policy = html.indexOf('Content-Security-Policy');
  if (policy < 0 || policy > html.indexOf('<script')) {
    throw new Error(`M6 CSP is missing or follows executable content: ${relative}`);
  }
  if (html.indexOf('Content-Security-Policy', policy + 1) >= 0) {
    throw new Error(`M6 CSP is duplicated: ${relative}`);
  }
}

console.log(`Verified ${required.length} static artifacts under /pmoke/.`);

async function recursiveFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  return (
    await Promise.all(
      entries.map((entry) => {
        const target = path.join(directory, entry.name);
        return entry.isDirectory() ? recursiveFiles(target) : [target];
      }),
    )
  ).flat();
}
