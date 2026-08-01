import { source } from '@/lib/source';
import { basePath, siteDescription, siteOrigin } from '@/lib/shared';

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
  const body = `# pmoke\n\n> ${siteDescription}\n\n${sections.join('\n\n')}\n`;
  return new Response(body, {
    headers: { 'Content-Type': 'text/plain; charset=utf-8' },
  });
}
