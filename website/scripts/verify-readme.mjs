import { access, readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const websiteRoot = path.resolve(scriptDirectory, '..');
const repositoryRoot = path.resolve(websiteRoot, '..');
const readmePath = path.join(repositoryRoot, 'README.md');
const readme = await readFile(readmePath, 'utf8');
const docsOrigin = 'https://kerr-group.github.io';
const docsPrefix = '/pmoke/';
const links = new Set();
const headings = new Set(
  [...readme.matchAll(/^#{1,6}\s+(.+?)\s*$/gmu)].map((match) => slug(match[1])),
);

for (const match of readme.matchAll(/!?\[[^\]]*\]\(([^)\s]+)(?:\s+"[^"]*")?\)/gu)) {
  links.add(match[1]);
}
for (const match of readme.matchAll(/\b(?:href|src)="([^"]+)"/gu)) {
  links.add(match[1]);
}

for (const target of links) {
  if (target.startsWith('#')) {
    verifyFragment(target.slice(1));
    continue;
  }

  let url;
  try {
    url = new URL(target);
  } catch {
    await verifyLocalPath(target);
    continue;
  }

  if (url.origin === docsOrigin && url.pathname.startsWith(docsPrefix)) {
    await verifyDocumentationPath(url);
  }
}

console.log(`README verification passed for ${links.size} unique links and images.`);

function verifyFragment(fragment) {
  if (!headings.has(fragment)) {
    throw new Error(`README fragment does not match a heading: #${fragment}`);
  }
}

async function verifyLocalPath(target) {
  const clean = target.split('#', 1)[0];
  if (!clean) return;
  await access(path.resolve(repositoryRoot, clean)).catch(() => {
    throw new Error(`README local target does not exist: ${target}`);
  });
}

async function verifyDocumentationPath(url) {
  const relative = url.pathname.slice(docsPrefix.length).replace(/\/$/u, '');
  if (!relative) return;

  const [locale, docsSegment, ...segments] = relative.split('/');
  if (!['en', 'ja'].includes(locale)) {
    throw new Error(`documentation URL must include an en or ja locale: ${url.href}`);
  }
  if (!docsSegment) return;
  if (docsSegment !== 'docs' || segments.length === 0) {
    throw new Error(`unrecognized documentation URL: ${url.href}`);
  }

  const contentPath = path.resolve(
    websiteRoot,
    'content/docs',
    locale,
    ...segments.slice(0, -1),
    `${segments.at(-1)}.mdx`,
  );
  const indexPath = path.resolve(websiteRoot, 'content/docs', locale, ...segments, 'index.mdx');
  const found = await Promise.any([access(contentPath), access(indexPath)]).then(
    () => true,
    () => false,
  );
  if (!found) throw new Error(`documentation URL has no matching source page: ${url.href}`);
}

function slug(heading) {
  return heading
    .normalize('NFKD')
    .toLowerCase()
    .replace(/[^\p{Letter}\p{Number}\s-]/gu, '')
    .trim()
    .replace(/\s+/gu, '-');
}
