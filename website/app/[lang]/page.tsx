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
    kicker: 'PULSED-FIELD MOKE / REPRODUCIBLE MEASUREMENT',
    lead: 'Capture the field pulse. Resolve the phase. Extract the Kerr angle.',
    description:
      'A reproducible Rust workflow for pulsed-field MOKE—from instrument trigger and waveform capture to phase-aware lock-in analysis and Kerr-angle extraction.',
    docs: 'Read the docs',
    quickstart: 'Quickstart',
    signal: {
      label: 'Illustrative pulsed-field MOKE signal',
      description:
        'Illustrative pulsed-field MOKE sequence: a magnetic-field pulse marks the acquisition window; reference and Kerr-response waveforms are shown and resolved into lock-in X and Y channels.',
      sequence: 'Pulsed-field MOKE signal sequence',
      fieldPulse: 'FIELD PULSE',
      acquisitionWindow: 'ACQUISITION WINDOW',
      reference: 'REFERENCE',
      kerrResponse: 'KERR RESPONSE',
      lockInX: 'LOCK-IN X',
      lockInY: 'LOCK-IN Y',
      kerrAngle: 'KERR ANGLE',
      pause: 'Pause animation',
      resume: 'Resume animation',
      reducedMotion: 'Static view (reduced motion)',
      staticFallback: 'Static fallback (WASM unavailable)',
      wasmLoading: 'WASM LOADING',
      wasmReady: 'WASM ONLINE',
      wasmFallback: 'STATIC FALLBACK',
    },
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
    kicker: 'パルス磁場MOKE / 再現可能な計測',
    lead: '磁場パルスを捉え、位相を解析し、Kerr角を抽出する。',
    description:
      '装置トリガーと波形の取得から、位相回転を含むロックイン解析、Kerr角の算出までを一貫して扱う、再現可能なRustワークフロー。',
    docs: 'ドキュメント',
    quickstart: 'クイックスタート',
    signal: {
      label: 'パルス磁場MOKEの説明図',
      description:
        'パルス磁場MOKEの流れを示す説明図。磁場パルスに合わせた取得窓の中で、参照信号とKerr応答を可視化し、ロックイン解析でX/Y成分とKerr角を導出する様子。',
      sequence: 'パルス磁場MOKEの信号シーケンス',
      fieldPulse: '磁場パルス',
      acquisitionWindow: '取得窓',
      reference: '参照信号',
      kerrResponse: 'Kerr応答',
      lockInX: 'ロックイン X',
      lockInY: 'ロックイン Y',
      kerrAngle: 'Kerr角',
      pause: 'アニメーションを一時停止',
      resume: 'アニメーションを再開',
      reducedMotion: '静止表示（視覚効果を低減）',
      staticFallback: '静的フォールバック（WASM利用不可）',
      wasmLoading: 'WASMを読み込み中',
      wasmReady: 'WASM ONLINE',
      wasmFallback: '静的フォールバック',
    },
    toggleTheme: 'カラーテーマ切替',
    lightTheme: 'ライトテーマへ切替',
    darkTheme: 'ダークテーマへ切替',
    cards: [
      ['装置制御', 'TCP/IP、GPIB、Prologix の型付き通信。', 'terminal'],
      ['解析パイプライン', 'Reference、sensor、lock-in、phase、Kerrの連続処理。', 'activity'],
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
        <SignalHero labels={text.signal} />
        <div className="hero-copy">
          <div className="hero-copy-panel">
            <p className="eyebrow"><Cpu aria-hidden="true" />{text.kicker}</p>
            <h1>pmoke</h1>
            <p className="hero-lead">{text.lead}</p>
            <p className="hero-description">{text.description}</p>
            <div className="hero-actions">
              <Link className="primary-action" href={`/${lang}/docs`} prefetch={false}>{text.docs}<ArrowRight aria-hidden="true" /></Link>
              <Link className="secondary-action" href={`/${lang}/docs/quickstart`} prefetch={false}>{text.quickstart}</Link>
            </div>
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
  const title = lang === 'ja' ? 'pmoke | パルス磁場MOKE ワークフロー' : 'pmoke | Pulsed-field MOKE workflow';
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
