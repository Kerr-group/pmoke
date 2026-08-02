import { source } from '@/lib/source';
import { basePath, siteDescription, siteOrigin } from '@/lib/shared';
import { versionMetadata } from '@/lib/version';

export const revalidate = false;

export function GET() {
  const sections = ['en', 'ja'].map((locale) => {
    const title = locale === 'ja' ? '日本語' : 'English';
    const pages = source.getPages(locale).map((page) => {
      const url = new URL(`${basePath}${page.url}/`, siteOrigin);
      return `- [${page.data.title}](${url}): ${page.data.description ?? ''}`;
    });
    return `## ${title}\n\n${pages.join('\n')}`;
  });
  const resources = [
    `- [English full context](${new URL(`${basePath}/llms-en.txt`, siteOrigin)})`,
    `- [Japanese full context](${new URL(`${basePath}/llms-ja.txt`, siteOrigin)})`,
    `- [Bilingual full context](${new URL(`${basePath}/llms-full.txt`, siteOrigin)})`,
    `- [Machine manifest](${new URL(`${basePath}/ai-index.json`, siteOrigin)})`,
  ].join('\n');
  const body = `# pmoke\n\n> ${siteDescription}\n\n- pmoke: ${versionMetadata.pmoke_version}\n- config schema: ${versionMetadata.schema_version}\n- source commit: ${versionMetadata.source_commit}\n- canonical root: ${new URL(`${basePath}/`, siteOrigin)}\n- search privacy: browser-local; no query upload or telemetry\n- authority: native pmoke runtime for config semantics and hardware behavior\n\n## Agent resources\n\n${resources}\n\n## Retrieval policy\n\n1. Select the matching locale and smallest relevant page.\n2. Prefer generated JSON for exact command and configuration fields.\n3. Preserve units, field paths, versions, and source links.\n4. Confirm semantic configuration with pmoke config validate.\n5. Never infer laboratory addresses or credentials.\n\n${sections.join('\n\n')}\n`;
  return new Response(body, {
    headers: { 'Content-Type': 'text/plain; charset=utf-8' },
  });
}
