import { mkdir, readdir, stat, writeFile } from 'node:fs/promises';
import path from 'node:path';

const output = path.resolve('out');
const records = [];

async function scan(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const absolute = path.join(directory, entry.name);
    if (entry.isDirectory()) await scan(absolute);
    else {
      const metadata = await stat(absolute);
      records.push({ path: path.relative(output, absolute), bytes: metadata.size });
    }
  }
}

await writeFile(path.join(output, '.nojekyll'), '');
await scan(output);
records.sort((left, right) => right.bytes - left.bytes);
const summary = {
  generatedAt: new Date().toISOString(),
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
