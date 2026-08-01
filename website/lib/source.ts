import { loader } from 'fumadocs-core/source';
import { absoluteUrl, basePath, docsContentRoute, docsRoute } from './shared';
import { defineDocs } from 'fumadocs-mdx/macro';
import { metaSchema, pageSchema } from 'fumadocs-core/source/schema';
import { i18n } from './i18n';
import { versionMetadata } from './version';

const docs = defineDocs({
  dir: 'content/docs',
  docs: {
    schema: pageSchema,
    postprocess: {
      includeProcessedMarkdown: true,
    },
  },
  meta: {
    schema: metaSchema,
  },
});

// See https://fumadocs.dev/docs/headless/source-api for more info
export const source = loader({
  baseUrl: docsRoute,
  i18n,
  source: docs.toFumadocsSource(),
  plugins: [],
});

export function getPageMarkdownUrl(page: (typeof source)['$inferPage']) {
  const last = page.slugs.at(-1);
  const segments = last
    ? [...page.slugs.slice(0, -1), `${last}.md`]
    : ['content.md'];

  return {
    segments,
    url: withBasePath(
      '/' + [docsContentRoute, page.locale, ...segments].filter(Boolean).join('/'),
    ),
  };
}

function withBasePath(path: string): string {
  return `${basePath}${path.replaceAll('//', '/')}`;
}

export async function getLLMText(page: (typeof source)['$inferPage']) {
  const processed = await page.data.getText('processed');
  const canonical = absoluteUrl(page.url);

  return `# ${page.data.title} (${canonical})

- locale: ${page.locale}
- pmoke: ${versionMetadata.pmoke_version}
- config schema: ${versionMetadata.schema_version}
- source commit: ${versionMetadata.source_commit}

${processed}`;
}
