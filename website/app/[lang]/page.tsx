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
    lead: 'Capture the field pulse. Rotate the phase. Extract the Kerr angle.',
    description:
      'A reproducible Rust workflow for pulsed-field MOKE—from instrument trigger and waveform capture through lock-in X/Y extraction, per-harmonic phase rotation, and Kerr-angle extraction.',
    docs: 'Read the docs',
    quickstart: 'Quickstart',
    signal: {
      label: 'Illustrative pulsed-field MOKE signal',
      description:
        'Illustrative pulsed-field MOKE pipeline: a unipolar field pulse defines a triggered measurement window; reference and Kerr-response signals are resolved into lock-in X/Y components, phase-rotated for each harmonic, and combined for Kerr-angle extraction.',
      pipeline: 'SIGNAL PIPELINE',
      sequence: 'Pulsed-field MOKE signal processing stages',
      fieldPulse: 'FIELD PULSE',
      triggeredWindow: 'TRIGGERED WINDOW',
      referenceResponse: 'REFERENCE + RESPONSE',
      lockIn: 'LOCK-IN X / Y',
      rotatePhase: 'ROTATE PHASE',
      kerrAngle: 'KERR ANGLE',
      timeDomain: 'TIME DOMAIN',
      phaseSpace: 'PHASE SPACE',
      harmonicExtraction: 'HARMONIC EXTRACTION',
      perHarmonic: 'PER HARMONIC · n = 1…6',
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
    lead: '磁場パルスを捉え、位相回転を経て、Kerr角を導出する。',
    description:
      '装置トリガーと波形の取得から、ロックインX/Y抽出、高調波ごとの位相回転、Kerr角の算出までを一貫して扱う、再現可能なRustワークフロー。',
    docs: 'ドキュメント',
    quickstart: 'クイックスタート',
    signal: {
      label: 'パルス磁場MOKEの説明図',
      description:
        'パルス磁場MOKEの概念図。単極性の磁場パルスがトリガー窓を定め、参照信号とKerr応答をロックインX/Y成分へ変換する。高調波ごとの位相回転を経て、Kerr角を導出する。',
      pipeline: '信号処理の流れ',
      sequence: 'パルス磁場MOKEの信号処理ステップ',
      fieldPulse: '磁場パルス',
      triggeredWindow: 'トリガー窓',
      referenceResponse: '参照信号 + Kerr応答',
      lockIn: 'ロックイン X / Y',
      rotatePhase: '位相回転',
      kerrAngle: 'Kerr角',
      timeDomain: '時間領域',
      phaseSpace: '位相空間',
      harmonicExtraction: '高調波抽出',
      perHarmonic: '高調波ごと · n = 1…6',
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
        <div className="hero-copy">
          <div className="hero-copy-panel" data-signal-region="copy">
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
        <SignalHero labels={text.signal} />
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
