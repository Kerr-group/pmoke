import { machineResources } from '@/lib/machine-resources';
import { getPageMarkdownUrl, getSortedPages } from '@/lib/source';
import { absoluteUrl, siteDescription, siteOrigin } from '@/lib/shared';
import { versionMetadata } from '@/lib/version';

export const revalidate = false;

export function GET() {
  const sections = ['en', 'ja'].map((locale) => {
    const title = locale === 'ja' ? '日本語' : 'English';
    const pages = getSortedPages(locale).map((page) => {
      const url = new URL(getPageMarkdownUrl(page).url, siteOrigin);
      return `- [${page.data.title}](${url}): ${page.data.description ?? ''}`;
    });
    return `## ${title}\n\n${pages.join('\n')}`;
  });
  const contracts = machineResources
    .filter((resource) => resource.group === 'contract' || resource.id === 'manifest')
    .map((resource) => `- [${resource.id}](${absoluteUrl(resource.path)}): ${resource.mediaType}`)
    .join('\n');
  const optional = machineResources
    .filter((resource) => resource.group === 'context')
    .map((resource) => `- [${resource.id}](${absoluteUrl(resource.path)}): ${resource.locale} full context`)
    .join('\n');
  const body = `# pmoke\n\n> ${siteDescription}\n\npmoke ${versionMetadata.pmoke_version}; config schema ${versionMetadata.schema_version}; source commit ${versionMetadata.source_commit}. Native pmoke runtime is authoritative for configuration semantics and hardware behavior. Search and browser tools are local-only with no query upload or telemetry. Select the matching locale and smallest relevant page, prefer generated JSON for exact fields, preserve units and source links, confirm configs with pmoke config validate, and never infer laboratory addresses or credentials.\n\n## Machine contracts\n\n${contracts}\n\n${sections.join('\n\n')}\n\n## Optional\n\n${optional}\n`;
  return new Response(body, {
    headers: { 'Content-Type': 'text/plain; charset=utf-8' },
  });
}
