import { createHash } from 'node:crypto';
import { getLLMText, getPageMarkdownUrl, source } from '@/lib/source';
import { absoluteUrl, siteOrigin } from '@/lib/shared';
import { machineResources } from '@/lib/machine-resources';
import { versionMetadata } from '@/lib/version';

export async function buildFullLLMText(locales: string[] = ['en', 'ja']): Promise<string> {
  const pages = source
    .getPages()
    .filter((page) => locales.includes(page.locale ?? 'en'))
    .sort((left, right) =>
      `${left.locale}:${left.url}`.localeCompare(`${right.locale}:${right.url}`),
    );
  const content = await Promise.all(pages.map(getLLMText));
  const metadata = `# pmoke full documentation

- pmoke: ${versionMetadata.pmoke_version}
- config schema: ${versionMetadata.schema_version}
- source commit: ${versionMetadata.source_commit}
- locales: ${locales.join(', ')}
- authority: native pmoke runtime for semantic validation and hardware behavior`;
  return [metadata, ...content].join('\n\n');
}

export async function buildAIManifest() {
  const pages = source
    .getPages()
    .sort((left, right) => `${left.locale}:${left.url}`.localeCompare(`${right.locale}:${right.url}`));
  return {
    schema: 1,
    product: 'pmoke',
    pmoke_version: versionMetadata.pmoke_version,
    config_schema_version: versionMetadata.schema_version,
    source_commit: versionMetadata.source_commit,
    authority: {
      configuration: 'pmoke config validate',
      hardware: 'pmoke doctor hardware',
      documentation: absoluteUrl('/llms.txt'),
    },
    privacy: {
      telemetry: false,
      search_query_upload: false,
      browser_tools_local_only: true,
    },
    resources: machineResources.map((resource) => ({
      id: resource.id,
      url: absoluteUrl(resource.path),
      media_type: resource.mediaType,
      group: resource.group,
      locale: resource.locale,
    })),
    pages: await Promise.all(
      pages.map(async (page) => {
        const markdown = await getLLMText(page);
        const markdownPath = getPageMarkdownUrl(page).url;
        return {
          locale: page.locale,
          title: page.data.title,
          description: page.data.description ?? '',
          canonical_url: absoluteUrl(page.url),
          markdown_url: `${siteOrigin}${markdownPath}`,
          sha256: createHash('sha256').update(markdown).digest('hex'),
        };
      }),
    ),
  };
}
