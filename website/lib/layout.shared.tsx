import type { BaseLayoutProps } from 'fumadocs-ui/layouts/shared';
import { Activity, CodeXml, House } from 'lucide-react';
import { appName, gitConfig } from './shared';

export function baseOptions(lang: 'en' | 'ja'): BaseLayoutProps {
  return {
    nav: {
      title: (
        <span className="brand-lockup">
          <Activity aria-hidden="true" size={18} />
          <span>{appName}</span>
          <span className="brand-context">DOCS</span>
        </span>
      ),
      url: `/${lang}`,
    },
    links: [
      {
        text: lang === 'ja' ? 'ホーム' : 'Home',
        url: `/${lang}`,
        icon: <House aria-hidden="true" />,
      },
      {
        type: 'icon',
        text: 'GitHub',
        label: 'GitHub',
        url: `https://github.com/${gitConfig.user}/${gitConfig.repo}`,
        external: true,
        icon: <CodeXml aria-hidden="true" />,
      },
    ],
    i18n: true,
  };
}
