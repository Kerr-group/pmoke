import type { Metadata } from 'next';
import Link from 'next/link';
import { Activity, ArrowRight, BookOpen, CodeXml, Cpu, Search, Terminal } from 'lucide-react';
import { notFound } from 'next/navigation';
import { SignalHero } from '@/components/signal-hero';
import { ThemeToggle } from '@/components/theme-toggle';
import { isLanguage } from '@/lib/i18n';
import { absoluteUrl, siteDescription, siteDescriptionJa, socialImage } from '@/lib/shared';

const copy = {
  en: {
    kicker: 'PRECISION SIGNAL LAB',
    lead: 'Acquire instruments. Resolve phase. Extract Kerr response.',
    description:
      'A reproducible Rust workflow for pulsed-MOKE acquisition, lock-in processing, and hardware diagnostics.',
    docs: 'Read the docs',
    quickstart: 'Quickstart',
    signal: 'Live Wasm signal preview',
    toggleTheme: 'Toggle color theme',
    lightTheme: 'Switch to light theme',
    darkTheme: 'Switch to dark theme',
    cards: [
      ['Instrument control', 'Typed TCP/IP, GPIB, and Prologix transports.', 'terminal'],
      ['Analysis pipeline', 'Reference, sensor, lock-in, phase, and Kerr stages.', 'activity'],
      ['Searchable knowledge', 'Bilingual static search and agent-ready text.', 'search'],
    ],
  },
  ja: {
    kicker: '精密信号ラボ',
    lead: '装置を制御し、位相を推定し、Kerr応答を抽出します。',
    description: 'パルスMOKEの取得、ロックイン処理、ハードウェア診断を行う、再現可能なRustワークフローです。',
    docs: 'ドキュメント',
    quickstart: 'クイックスタート',
    signal: 'Wasm 信号プレビュー',
    toggleTheme: 'カラーテーマ切替',
    lightTheme: 'ライトテーマへ切替',
    darkTheme: 'ダークテーマへ切替',
    cards: [
      ['装置制御', 'TCP/IP、GPIB、Prologix の型付き通信。', 'terminal'],
      ['解析パイプライン', 'Reference、sensor、lock-in、phase、Kerrを順に処理します。', 'activity'],
      ['検索可能な知識', '日英静的検索と AI エージェント向けテキスト。', 'search'],
    ],
  },
} as const;

const icons = { terminal: Terminal, activity: Activity, search: Search };

export default async function HomePage({ params }: { params: Promise<{ lang: string }> }) {
  const { lang } = await params;
  if (!isLanguage(lang)) notFound();
  const text = copy[lang];

  return (
    <main className="home-shell">
      <header className="site-header">
        <Link className="brand-lockup" href={`/${lang}`} aria-label="pmoke home">
          <Activity aria-hidden="true" size={19} />
          <strong>pmoke</strong>
        </Link>
        <nav aria-label={lang === 'ja' ? '主要ナビゲーション' : 'Primary navigation'}>
          <Link href={`/${lang}/docs`} prefetch={false}><BookOpen aria-hidden="true" />{text.docs}</Link>
          <Link href={`/${lang === 'en' ? 'ja' : 'en'}`} lang={lang === 'en' ? 'ja' : 'en'}>
            {lang === 'en' ? '日本語' : 'English'}
          </Link>
          <ThemeToggle toggleLabel={text.toggleTheme} lightLabel={text.lightTheme} darkLabel={text.darkTheme} />
          <a href="https://github.com/Kerr-group/pmoke" aria-label="GitHub"><CodeXml aria-hidden="true" /></a>
        </nav>
      </header>

      <section className="hero-band">
        <SignalHero label={text.signal} />
        <div className="hero-copy">
          <p className="eyebrow"><Cpu aria-hidden="true" />{text.kicker}</p>
          <h1>pmoke</h1>
          <p className="hero-lead">{text.lead}</p>
          <p className="hero-description">{text.description}</p>
          <div className="hero-actions">
            <Link className="primary-action" href={`/${lang}/docs`} prefetch={false}>{text.docs}<ArrowRight aria-hidden="true" /></Link>
            <Link className="secondary-action" href={`/${lang}/docs/quickstart`} prefetch={false}>{text.quickstart}</Link>
          </div>
        </div>
      </section>

      <section className="capability-grid" aria-label={lang === 'ja' ? '主要機能' : 'Core capabilities'}>
        {text.cards.map(([title, description, icon]) => {
          const Icon = icons[icon];
          return <article key={title}><Icon aria-hidden="true" /><h2>{title}</h2><p>{description}</p></article>;
        })}
      </section>
    </main>
  );
}

export async function generateMetadata({
  params,
}: {
  params: Promise<{ lang: string }>;
}): Promise<Metadata> {
  const { lang } = await params;
  if (!isLanguage(lang)) notFound();
  const title = lang === 'ja' ? 'pmoke | パルス MOKE 精密信号ラボ' : 'pmoke | Pulsed-MOKE precision signal lab';
  const description = lang === 'ja' ? siteDescriptionJa : siteDescription;
  const canonical = absoluteUrl(`/${lang}`);

  return {
    title: { absolute: title },
    description,
    alternates: {
      canonical,
      languages: {
        en: absoluteUrl('/en'),
        ja: absoluteUrl('/ja'),
        'x-default': absoluteUrl('/'),
      },
    },
    openGraph: {
      title,
      description,
      url: canonical,
      locale: lang === 'ja' ? 'ja_JP' : 'en_US',
      alternateLocale: lang === 'ja' ? ['en_US'] : ['ja_JP'],
      images: [socialImage],
    },
    twitter: { title, description, images: [socialImage.url] },
  };
}
