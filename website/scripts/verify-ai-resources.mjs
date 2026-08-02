import { createHash } from 'node:crypto';
import { readFile, stat } from 'node:fs/promises';
import path from 'node:path';

const output = path.resolve('out');
const files = {
  concise: 'llms.txt',
  full: 'llms-full.txt',
  english: 'llms-en.txt',
  japanese: 'llms-ja.txt',
  manifest: 'ai-index.json',
};
const content = Object.fromEntries(
  await Promise.all(
    Object.entries(files).map(async ([name, relative]) => [name, await readFile(path.join(output, relative), 'utf8')]),
  ),
);

if (Buffer.byteLength(content.concise) > 100 * 1024) throw new Error('llms.txt exceeds 100 KiB');
if (Buffer.byteLength(content.full) > 5 * 1024 * 1024) throw new Error('llms-full.txt exceeds 5 MiB');
if (content.english.includes('- locale: ja')) throw new Error('English LLM feed contains Japanese pages');
if (content.japanese.includes('- locale: en')) throw new Error('Japanese LLM feed contains English pages');

const manifest = JSON.parse(content.manifest);
if (manifest.schema !== 1 || manifest.privacy?.search_query_upload !== false) {
  throw new Error('AI manifest schema or privacy boundary is invalid');
}
if (!Array.isArray(manifest.pages) || manifest.pages.length < 26) {
  throw new Error('AI manifest is missing localized pages');
}
const seen = new Set();
for (const page of manifest.pages) {
  if (!['en', 'ja'].includes(page.locale)) throw new Error(`invalid manifest locale: ${page.locale}`);
  if (seen.has(page.canonical_url)) throw new Error(`duplicate canonical URL: ${page.canonical_url}`);
  seen.add(page.canonical_url);
  const url = new URL(page.markdown_url);
  if (url.origin !== 'https://kerr-group.github.io' || !url.pathname.startsWith('/pmoke/llm/')) {
    throw new Error(`invalid Markdown URL: ${page.markdown_url}`);
  }
  const exported = path.join(output, url.pathname.slice('/pmoke/'.length));
  const markdown = await readFile(exported);
  const digest = createHash('sha256').update(markdown).digest('hex');
  if (digest !== page.sha256) throw new Error(`Markdown digest mismatch: ${page.markdown_url}`);
}

const forbidden = [
  { name: 'private IPv4 address', pattern: /\b(?:10(?:\.\d{1,3}){3}|192\.168(?:\.\d{1,3}){2}|172\.(?:1[6-9]|2\d|3[01])(?:\.\d{1,3}){2})\b/u },
  { name: 'local Unix path', pattern: /\/(?:home|Users)\/[^\s]+/u },
  { name: 'Windows user path', pattern: /[A-Z]:\\Users\\[^\s]+/u },
  { name: 'credential assignment', pattern: /\b(?:api[_-]?key|password|token)\s*[=:]\s*["'][^"']{8,}/iu },
];
for (const [name, value] of Object.entries(content)) {
  for (const rule of forbidden) {
    if (rule.pattern.test(value)) throw new Error(`${rule.name} found in ${files[name]}`);
  }
}

const sizes = Object.fromEntries(
  await Promise.all(Object.entries(files).map(async ([name, relative]) => [name, (await stat(path.join(output, relative))).size])),
);
console.log(`Verified ${manifest.pages.length} AI manifest pages: ${JSON.stringify(sizes)}`);
