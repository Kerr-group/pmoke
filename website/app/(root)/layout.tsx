import type { Metadata } from 'next';
import type { ReactNode } from 'react';
import { FontVariables } from '@/components/font-variables';
import { absoluteUrl, faviconImage, siteDescription, siteUrl, socialImage } from '@/lib/shared';
import '../global.css';

export const metadata: Metadata = {
  metadataBase: new URL(`${siteUrl}/`),
  title: 'pmoke | Documentation language',
  description: siteDescription,
  icons: {
    icon: [{ url: faviconImage, type: 'image/png', sizes: '64x64' }],
    shortcut: faviconImage,
  },
  alternates: {
    canonical: absoluteUrl('/'),
    languages: {
      en: absoluteUrl('/en'),
      ja: absoluteUrl('/ja'),
      'x-default': absoluteUrl('/'),
    },
  },
  openGraph: {
    type: 'website',
    title: 'pmoke | Documentation language',
    description: siteDescription,
    url: absoluteUrl('/'),
    siteName: 'pmoke',
    images: [socialImage],
  },
  twitter: {
    card: 'summary_large_image',
    title: 'pmoke | Documentation language',
    description: siteDescription,
    images: [socialImage.url],
  },
};

export default function RootRouteLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" className={FontVariables} suppressHydrationWarning>
      <body>
        {children}
      </body>
    </html>
  );
}
