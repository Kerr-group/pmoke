import type { MetadataRoute } from 'next';
import { absoluteUrl, basePath, siteOrigin } from '@/lib/shared';

export const dynamic = 'force-static';

export default function robots(): MetadataRoute.Robots {
  return {
    rules: {
      userAgent: '*',
      allow: `${basePath || ''}/`,
    },
    sitemap: absoluteUrl('/sitemap.xml'),
    host: siteOrigin,
  };
}
