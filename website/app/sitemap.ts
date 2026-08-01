import type { MetadataRoute } from 'next';
import { languages } from '@/lib/i18n';
import { source } from '@/lib/source';
import { absoluteUrl } from '@/lib/shared';

export const dynamic = 'force-static';

export default function sitemap(): MetadataRoute.Sitemap {
  const rootLanguages = {
    en: absoluteUrl('/en'),
    ja: absoluteUrl('/ja'),
    'x-default': absoluteUrl('/'),
  };
  const entries: MetadataRoute.Sitemap = [
    {
      url: absoluteUrl('/'),
      changeFrequency: 'monthly',
      priority: 0.8,
      alternates: { languages: rootLanguages },
    },
  ];
  const seen = new Set<string>();

  for (const language of languages) {
    for (const page of source.getPages(language)) {
      const key = page.slugs.join('/');
      if (seen.has(key)) continue;
      seen.add(key);
      const english = source.getPage(page.slugs, 'en');
      const japanese = source.getPage(page.slugs, 'ja');
      const alternates: Record<string, string> = {};
      if (english) {
        alternates.en = absoluteUrl(english.url);
        alternates['x-default'] = absoluteUrl(english.url);
      }
      if (japanese) alternates.ja = absoluteUrl(japanese.url);

      for (const localizedPage of [english, japanese]) {
        if (!localizedPage) continue;
        entries.push({
          url: absoluteUrl(localizedPage.url),
          changeFrequency: 'weekly',
          priority: localizedPage.slugs.length === 0 ? 1 : 0.8,
          alternates: { languages: alternates },
        });
      }
    }
  }

  entries.push(
    ...languages.map((language) => ({
      url: absoluteUrl(`/${language}`),
      changeFrequency: 'weekly' as const,
      priority: 1,
      alternates: { languages: rootLanguages },
    })),
  );

  return entries;
}
