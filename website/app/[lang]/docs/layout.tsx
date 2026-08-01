import type { ReactNode } from 'react';
import { notFound } from 'next/navigation';
import { DocsLayout } from 'fumadocs-ui/layouts/docs';
import { baseOptions } from '@/lib/layout.shared';
import { isLanguage } from '@/lib/i18n';
import { source } from '@/lib/source';
import { Provider } from '@/components/provider';

export default async function Layout({
  children,
  params,
}: {
  children: ReactNode;
  params: Promise<{ lang: string }>;
}) {
  const { lang } = await params;
  if (!isLanguage(lang)) notFound();
  return (
    <Provider lang={lang}>
      <DocsLayout tree={source.getPageTree(lang)} {...baseOptions(lang)}>{children}</DocsLayout>
    </Provider>
  );
}
