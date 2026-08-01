'use client';
import SearchDialog from '@/components/search';
import { RootProvider } from 'fumadocs-ui/provider/next';
import { type ReactNode } from 'react';
import { i18nUI, type Language } from '@/lib/i18n';

export function Provider({ children, lang }: { children: ReactNode; lang: Language }) {
  return (
    <RootProvider
      search={{ SearchDialog }}
      i18n={i18nUI.provider(lang)}
      theme={{ attribute: 'class', defaultTheme: 'dark', enableSystem: true }}
    >
      {children}
    </RootProvider>
  );
}
