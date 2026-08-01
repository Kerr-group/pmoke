import { readdir, readFile } from 'node:fs/promises';
import path from 'node:path';

const docsRoot = path.resolve('content/docs');
const generatedRoot = path.resolve('generated');
const publicSchema = path.resolve('public/config.schema.json');
const english = await relativeFiles(path.join(docsRoot, 'en'));
const japanese = await relativeFiles(path.join(docsRoot, 'ja'));

assertSameFiles(english, japanese);

const contentFiles = [
  ...english.map((file) => path.join(docsRoot, 'en', file)),
  ...japanese.map((file) => path.join(docsRoot, 'ja', file)),
  ...(await absoluteFiles(generatedRoot)),
  publicSchema,
];
const forbidden = [
  {
    name: 'private IPv4 address',
    pattern: /\b(?:10(?:\.\d{1,3}){3}|192\.168(?:\.\d{1,3}){2}|172\.(?:1[6-9]|2\d|3[01])(?:\.\d{1,3}){2})\b/u,
  },
  { name: 'unfinished placeholder', pattern: /\b(?:TODO|TBD|coming soon)\b/iu },
];
for (const file of contentFiles) {
  const source = await readFile(file, 'utf8');
  for (const rule of forbidden) {
    if (rule.pattern.test(source)) {
      throw new Error(`${rule.name} found in ${path.relative(process.cwd(), file)}`);
    }
  }
}

const cli = JSON.parse(await readFile(path.join(generatedRoot, 'cli-reference.json'), 'utf8'));
const config = JSON.parse(await readFile(path.join(generatedRoot, 'config-reference.json'), 'utf8'));
const schema = JSON.parse(await readFile(publicSchema, 'utf8'));
if (cli.pmoke_version !== config.pmoke_version) throw new Error('generated pmoke versions differ');
if (config.schema_version !== schema['x-pmoke']?.schema_version) {
  throw new Error('generated config schema versions differ');
}
if (!Array.isArray(config.fields) || config.fields.length !== schema['x-pmoke']?.fields?.length) {
  throw new Error('config registry and JSON Schema field metadata differ');
}

console.log(`Content verification passed for ${english.length} paired locale files.`);

async function relativeFiles(directory, prefix = '') {
  const entries = await readdir(directory, { withFileTypes: true });
  const nested = await Promise.all(
    entries.map((entry) => {
      const relative = path.join(prefix, entry.name);
      return entry.isDirectory()
        ? relativeFiles(path.join(directory, entry.name), relative)
        : [relative];
    }),
  );
  return nested.flat().sort();
}

async function absoluteFiles(directory) {
  return (await relativeFiles(directory)).map((file) => path.join(directory, file));
}

function assertSameFiles(left, right) {
  const leftOnly = left.filter((file) => !right.includes(file));
  const rightOnly = right.filter((file) => !left.includes(file));
  if (leftOnly.length || rightOnly.length) {
    throw new Error(`locale parity failed; en-only=${leftOnly.join(',')}; ja-only=${rightOnly.join(',')}`);
  }
}
