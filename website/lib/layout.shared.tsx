import type { BaseLayoutProps } from 'fumadocs-ui/layouts/shared';
import { Activity, BookOpen, Terminal } from 'lucide-react';
import { appName, gitConfig } from './shared';

export function baseOptions(lang: 'en' | 'ja'): BaseLayoutProps {
  return {
    nav: {
      title: (
        <span className="brand-lockup">
          <Activity aria-hidden="true" size={18} />
          <span>{appName}</span>
        </span>
      ),
      url: `/${lang}`,
    },
    links: [
      {
        text: lang === 'ja' ? '概要' : 'Overview',
        url: `/${lang}/docs`,
        icon: <BookOpen aria-hidden="true" />,
      },
      {
        text: lang === 'ja' ? 'クイックスタート' : 'Quickstart',
        url: `/${lang}/docs/quickstart`,
        icon: <Terminal aria-hidden="true" />,
      },
    ],
    githubUrl: `https://github.com/${gitConfig.user}/${gitConfig.repo}`,
  };
}
