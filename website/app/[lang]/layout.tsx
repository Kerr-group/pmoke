import type { Metadata } from 'next';
import type { ReactNode } from 'react';
import { notFound } from 'next/navigation';
import { FontVariables } from '@/components/font-variables';
import { SiteThemeProvider } from '@/components/site-theme-provider';
import { isLanguage, languages } from '@/lib/i18n';
import { faviconImage, siteDescription, siteUrl, socialImage } from '@/lib/shared';
import '../global.css';

export const metadata: Metadata = {
  metadataBase: new URL(`${siteUrl}/`),
  title: { default: 'pmoke', template: '%s | pmoke' },
  description: siteDescription,
  applicationName: 'pmoke',
  category: 'science',
  icons: {
    icon: [{ url: faviconImage, type: 'image/png', sizes: '64x64' }],
    shortcut: faviconImage,
  },
  openGraph: {
    type: 'website',
    siteName: 'pmoke',
    description: siteDescription,
    images: [socialImage],
  },
  twitter: {
    card: 'summary_large_image',
    description: siteDescription,
    images: [socialImage.url],
  },
};

export function generateStaticParams() {
  return languages.map((lang) => ({ lang }));
}

export default async function LocaleLayout({
  children,
  params,
}: {
  children: ReactNode;
  params: Promise<{ lang: string }>;
}) {
  const { lang } = await params;
  if (!isLanguage(lang)) notFound();

  return (
    <html lang={lang} className={FontVariables} suppressHydrationWarning>
      <body>
        <SiteThemeProvider>{children}</SiteThemeProvider>
      </body>
    </html>
  );
}
