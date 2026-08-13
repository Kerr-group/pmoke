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
    kicker: 'PULSED-FIELD MOKE MEASUREMENTS',
    lead: 'A Rust workflow for pulsed-field MOKE measurements',
    description:
      'From waveform acquisition to Kerr-angle extraction—in one workflow.',
    docs: 'Read the docs',
    quickstart: 'Quickstart',
    signal: {
      label: 'Pulsed-field MOKE measurement workflow',
      description:
        'Pulsed-field MOKE measurement workflow. The pulsed field defines the common time axis for lock-in X/Y extraction, phase alignment, and Kerr-angle calculation.',
      pipeline: 'MEASUREMENT WORKFLOW',
      sequence: 'Pulsed-field MOKE measurement and analysis stages',
      fieldPulse: 'FIELD PULSE',
      lockIn: 'NUMERICAL LI ANALYSIS',
      lockInHeading: 'NUMERICAL LOCK-IN ANALYSIS',
      phaseAlignment: 'PHASE ALIGNMENT',
      kerrAngle: 'KERR ANGLE',
      fieldSummary: 'Integrate the time-derivative waveform to calculate the magnetic-field pulse waveform',
      lockInSummary: 'High-performance numerical lock-in architecture for signal analysis',
      phaseSummary: 'Rotate the lock-in X/Y vector to align its phase with the response from the zero-area-loop Sagnac interferometer',
      kerrSummary: 'Calculate the Kerr angle from the phase-aligned signal and display it in mrad',
      currentTime: 'TIME',
      timeAxis: 't (ms)',
      fieldAxis: 'μ₀H (T)',
      lockInAxis: 'LOCK-IN (mV)',
      kerrAxis: 'Kerr angle θ_K (mrad)',
      liX: 'LI X',
      liY: 'LI Y',
      rawVector: 'RAW X/Y',
      correctedVector: 'ALIGNED X/Y',
      phaseShift: 'PHASE ROTATION',
      quadratureZero: 'ALIGNED Y′',
      kerrResult: 'KERR ANGLE',
      replayStage: 'Replay stage',
    },
    toggleTheme: 'Toggle color theme',
    lightTheme: 'Switch to light theme',
    darkTheme: 'Switch to dark theme',
    cards: [
      ['Instrument control', 'Control instruments over TCP/IP, GPIB, and Prologix.', 'terminal'],
      ['Analysis workflow', 'Perform numerical lock-in analysis, align the phase, and calculate the Kerr angle.', 'activity'],
      ['Documentation search', 'Search the English and Japanese docs and access text exports for AI tools.', 'search'],
    ],
  },
  ja: {
    kicker: 'PULSED-FIELD MOKE MEASUREMENTS',
    lead: 'パルス磁場下MOKE測定のためのRustワークフロー',
    description:
      '波形取得からKerr角度の算出までを一つの流れに。',
    docs: 'ドキュメント',
    quickstart: 'クイックスタート',
    signal: {
      label: 'パルス磁場下でのMOKE測定ワークフロー',
      description:
        'パルス磁場下でのMOKE測定を示す処理図。共通の時間軸上でロックインX/Yを抽出し、位相整合後にKerr角度を算出。',
      pipeline: '測定ワークフロー',
      sequence: 'パルス磁場下でのMOKE測定と解析の工程',
      fieldPulse: 'パルス磁場',
      lockIn: '数値LI検波',
      lockInHeading: '数値Lock-in検波',
      phaseAlignment: '位相整合',
      kerrAngle: 'Kerr角度',
      fieldSummary: '磁場の時間微分波形を積分し、パルス磁場波形を算出',
      lockInSummary: '高性能な数値ロックインアーキテクチャによる検波',
      phaseSummary: 'zero-area-loop Sagnac干渉系の応答に合わせ、ロックインX/Yベクトルを回転して位相を整合',
      kerrSummary: '位相整合後の信号からKerr角度を算出し、mrad単位で表示',
      currentTime: '時刻',
      timeAxis: 't (ms)',
      fieldAxis: 'μ₀H (T)',
      lockInAxis: 'ロックインX/Y (mV)',
      kerrAxis: 'Kerr角度 θ_K (mrad)',
      liX: 'LI X',
      liY: 'LI Y',
      rawVector: '整合前 X/Y',
      correctedVector: '整合後 X/Y',
      phaseShift: '位相回転量',
      quadratureZero: '整合後 Y′',
      kerrResult: 'Kerr角度',
      replayStage: 'この工程を再生',
    },
    toggleTheme: 'カラーテーマを切り替える',
    lightTheme: 'ライトテーマに切り替える',
    darkTheme: 'ダークテーマに切り替える',
    cards: [
      ['装置制御', 'TCP/IP、GPIB、Prologixで測定装置を制御。', 'terminal'],
      ['解析ワークフロー', '数値Lock-in検波、位相整合、Kerr角度の算出を一つの流れで実行。', 'activity'],
      ['ドキュメント検索', '日英対応の検索とAI向けテキスト。', 'search'],
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
  const title = lang === 'ja'
    ? 'pmoke | パルス磁場下でのMOKE測定ワークフロー'
    : 'pmoke | Pulsed-field MOKE measurement workflow';
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
