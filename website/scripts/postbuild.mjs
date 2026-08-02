import { mkdir, readFile, readdir, stat, writeFile } from 'node:fs/promises';
import path from 'node:path';

const output = path.resolve('out');
const records = [];
const securityMetadata = [
  '<meta http-equiv="Content-Security-Policy" content="default-src \'self\'; base-uri \'self\'; object-src \'none\'; frame-src \'none\'; form-action \'self\'; img-src \'self\' data:; font-src \'self\' data:; style-src \'self\' \'unsafe-inline\'; script-src \'self\' \'unsafe-inline\' \'wasm-unsafe-eval\'; worker-src \'self\'; connect-src \'self\'"/>',
  '<meta name="referrer" content="strict-origin-when-cross-origin"/>',
].join('');

async function scan(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const absolute = path.join(directory, entry.name);
    if (entry.isDirectory()) await scan(absolute);
    else {
      if (entry.name.endsWith('.html')) await injectSecurityMetadata(absolute);
      const metadata = await stat(absolute);
      records.push({ path: path.relative(output, absolute), bytes: metadata.size });
    }
  }
}

await writeFile(path.join(output, '.nojekyll'), '');
await scan(output);
records.sort((left, right) => right.bytes - left.bytes);
const summary = {
  sourceRevision: process.env.GITHUB_SHA ?? 'development',
  totalBytes: records.reduce((sum, item) => sum + item.bytes, 0),
  searchBytes: records.filter((item) => item.path.startsWith('api/search')).reduce((sum, item) => sum + item.bytes, 0),
  wasmBytes: records.filter((item) => item.path.startsWith('wasm/')).reduce((sum, item) => sum + item.bytes, 0),
  largestFiles: records.slice(0, 12),
};
await mkdir(path.join(output, '_meta'), { recursive: true });
await writeFile(path.join(output, '_meta', 'payload.json'), `${JSON.stringify(summary, null, 2)}\n`);
console.log(`Static payload: ${(summary.totalBytes / 1024).toFixed(1)} KiB`);
console.log(`Search index: ${(summary.searchBytes / 1024).toFixed(1)} KiB`);
console.log(`Wasm package: ${(summary.wasmBytes / 1024).toFixed(1)} KiB`);

async function injectSecurityMetadata(file) {
  const html = await readFile(file, 'utf8');
  if (!html.includes('<head>')) throw new Error(`missing head element: ${path.relative(output, file)}`);
  if (html.includes('Content-Security-Policy')) throw new Error(`duplicate CSP metadata: ${path.relative(output, file)}`);
  await writeFile(file, html.replace('<head>', `<head>${securityMetadata}`));
}
