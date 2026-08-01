import { loader } from 'fumadocs-core/source';
import { basePath, docsContentRoute, docsRoute, siteOrigin } from './shared';
import { defineDocs } from 'fumadocs-mdx/macro';
import { metaSchema, pageSchema } from 'fumadocs-core/source/schema';
import { i18n } from './i18n';

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
  const segments = page.slugs.length === 0 ? ['content.md'] : page.slugs;

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
  const canonical = new URL(`${basePath}${page.url}/`, siteOrigin);

  return `# ${page.data.title} (${canonical})

${processed}`;
}
