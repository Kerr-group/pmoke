'use client';

import {
  Bot,
  Check,
  Copy,
  ExternalLink,
  FileCode2,
  Files,
  Globe2,
  LoaderCircle,
  Network,
  ShieldCheck,
} from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { machineResources, type MachineResourceGroup } from '@/lib/machine-resources';
import { basePath, siteUrl } from '@/lib/shared';

type Locale = 'en' | 'ja';
type Manifest = {
  schema: number;
  product: string;
  pmoke_version: string;
  config_schema_version: number;
  source_commit: string;
  pages: unknown[];
  resources: unknown[];
};

const copy = {
  en: {
    aria: 'AI and machine resource console',
    kicker: 'MACHINE INTERFACE',
    title: 'Agent resource console',
    loading: 'VERIFYING',
    ready: 'MANIFEST ONLINE',
    error: 'STATIC LINKS ONLY',
    metrics: ['Documents', 'Endpoints', 'Config schema', 'Source'],
    modes: { discovery: 'Discovery', context: 'Context', contract: 'Contracts' },
    modeLabel: 'Resource type',
    scope: { en: 'English', ja: 'Japanese', bilingual: 'Bilingual', neutral: 'Language neutral' },
    open: 'Open resource',
    copyUrl: 'Copy endpoint URL',
    copiedUrl: 'Endpoint URL copied',
    bootstrap: 'AGENT BOOTSTRAP',
    bootstrapTitle: 'Smallest-context first',
    bootstrapCopy: 'Copy bootstrap',
    copiedBootstrap: 'Agent bootstrap copied',
    copyFailed: 'Clipboard unavailable',
    trust: ['Same-origin assets', 'No query upload', 'Runtime validation authority'],
    resources: {
      'llms-index': ['llms.txt', 'Concise map for selecting the smallest relevant source.'],
      manifest: ['Machine manifest', 'Versioned page inventory with canonical URLs and SHA-256 digests.'],
      'english-context': ['English context', 'Complete processed English documentation.'],
      'japanese-context': ['Japanese context', 'Complete processed Japanese documentation.'],
      'full-context': ['Bilingual context', 'Complete English and Japanese documentation in one feed.'],
      'cli-contract': ['CLI contract', 'Generated command, argument, and option tree.'],
      'config-contract': ['Config contract', 'Generated field registry with types and defaults.'],
      'config-schema': ['JSON Schema', 'Editor-facing structural schema for canonical config v4.'],
    },
    prompt: `Read ${siteUrl}/llms.txt first. Select the smallest relevant locale-specific Markdown source. Prefer generated JSON contracts for exact CLI and config fields. Preserve versions, field paths, units, and canonical URLs. Validate configuration semantics with pmoke config validate; do not infer hardware state, credentials, or laboratory addresses.`,
  },
  ja: {
    aria: 'AI・機械向けリソースコンソール',
    kicker: 'MACHINE INTERFACE',
    title: 'エージェントリソースコンソール',
    loading: '検証中',
    ready: 'MANIFEST ONLINE',
    error: '静的リンクのみ',
    metrics: ['文書', 'エンドポイント', '設定スキーマ', 'ソース'],
    modes: { discovery: '探索', context: 'コンテキスト', contract: '契約' },
    modeLabel: 'リソース種別',
    scope: { en: '英語', ja: '日本語', bilingual: '日英', neutral: '言語非依存' },
    open: 'リソース表示',
    copyUrl: 'エンドポイントURLのコピー',
    copiedUrl: 'エンドポイントURLのコピー完了',
    bootstrap: 'AGENT BOOTSTRAP',
    bootstrapTitle: '最小コンテキスト優先',
    bootstrapCopy: 'ブートストラップのコピー',
    copiedBootstrap: 'エージェントブートストラップのコピー完了',
    copyFailed: 'クリップボード利用不可',
    trust: ['同一オリジン資産', 'クエリ送信なし', '実行時検証の優先'],
    resources: {
      'llms-index': ['llms.txt', '最小の関連ソース選択に用いる簡潔な案内。'],
      manifest: ['機械向けマニフェスト', 'canonical URLとSHA-256 digestを持つ、バージョン管理されたpage台帳。'],
      'english-context': ['英語コンテキスト', '処理済み英語documentationの全文。'],
      'japanese-context': ['日本語コンテキスト', '処理済み日本語documentationの全文。'],
      'full-context': ['日英コンテキスト', '英語と日本語のdocumentationをまとめたfeed。'],
      'cli-contract': ['CLI契約', '生成済みcommand・argument・option tree。'],
      'config-contract': ['設定契約', '型とdefault値を持つ生成済みfield registry。'],
      'config-schema': ['JSON Schema', 'canonical config v4向けのeditor用構造schema。'],
    },
    prompt: `最初に ${siteUrl}/llms.txt を参照。localeに対応する最小のMarkdown sourceを選択。正確なCLI・config fieldには生成JSON契約を優先。version、field path、unit、canonical URLを保持。config semanticsはpmoke config validateで検証。hardware state、credential、実験室addressの推測禁止。`,
  },
} as const;

const groupIcons = {
  discovery: Network,
  context: Files,
  contract: FileCode2,
} as const;

export function AIResourceHub({ locale }: { locale: Locale }) {
  const text = copy[locale];
  const [mode, setMode] = useState<MachineResourceGroup>('discovery');
  const [manifest, setManifest] = useState<Manifest | null>(null);
  const [status, setStatus] = useState<'loading' | 'ready' | 'error'>('loading');
  const [notice, setNotice] = useState('');
  const noticeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    fetch(`${basePath}/ai-index.json`, { signal: controller.signal })
      .then(async (response) => {
        if (!response.ok) throw new Error(`manifest request failed with ${response.status}`);
        return response.json() as Promise<Manifest>;
      })
      .then((value) => {
        if (
          value.schema !== 1
          || value.product !== 'pmoke'
          || typeof value.pmoke_version !== 'string'
          || !Number.isInteger(value.config_schema_version)
          || typeof value.source_commit !== 'string'
          || value.source_commit.length < 7
          || !Array.isArray(value.pages)
          || value.pages.length === 0
          || !Array.isArray(value.resources)
          || value.resources.length !== machineResources.length
        ) {
          throw new Error('invalid manifest');
        }
        setManifest(value);
        setStatus('ready');
      })
      .catch((error: unknown) => {
        if (!(error instanceof DOMException && error.name === 'AbortError')) setStatus('error');
      });
    return () => controller.abort();
  }, []);

  useEffect(() => () => {
    if (noticeTimer.current) clearTimeout(noticeTimer.current);
  }, []);

  const visibleResources = useMemo(
    () => machineResources.filter((resource) => resource.group === mode),
    [mode],
  );
  const SourceIcon = groupIcons[mode];
  const metrics = [
    manifest?.pages.length ?? '--',
    manifest?.resources.length ?? machineResources.length,
    manifest?.config_schema_version ?? '--',
    manifest?.source_commit.slice(0, 7) ?? '--',
  ];

  const announce = (message: string) => {
    setNotice(message);
    if (noticeTimer.current) clearTimeout(noticeTimer.current);
    noticeTimer.current = setTimeout(() => setNotice(''), 2_000);
  };

  const copyText = async (value: string, success: string) => {
    try {
      await navigator.clipboard.writeText(value);
      announce(success);
    } catch {
      announce(text.copyFailed);
    }
  };

  return (
    <section className="ai-hub" aria-label={text.aria} data-manifest={status}>
      <header className="ai-hub__commandbar">
        <div className="ai-hub__identity">
          <span className="ai-hub__mark"><Bot aria-hidden="true" /></span>
          <div>
            <span>{text.kicker}</span>
            <h2>{text.title}</h2>
          </div>
        </div>
        <span className="ai-hub__status" data-state={status}>
          {status === 'loading' ? <LoaderCircle aria-hidden="true" /> : status === 'ready' ? <Check aria-hidden="true" /> : <Globe2 aria-hidden="true" />}
          {text[status]}
        </span>
      </header>

      <dl className="ai-hub__metrics">
        {text.metrics.map((label, index) => (
          <div key={label}>
            <dt>{label}</dt>
            <dd>{metrics[index]}</dd>
          </div>
        ))}
      </dl>

      <div className="ai-hub__workspace">
        <div className="ai-hub__modebar" role="group" aria-label={text.modeLabel}>
          {(Object.keys(text.modes) as MachineResourceGroup[]).map((group) => (
            <button
              key={group}
              type="button"
              aria-pressed={mode === group}
              onClick={() => setMode(group)}
            >
              {text.modes[group]}
            </button>
          ))}
        </div>

        <div className="ai-hub__resources" aria-live="polite">
          {visibleResources.map((resource) => {
            const [name, description] = text.resources[resource.id];
            const href = `${basePath}${resource.path}`;
            return (
              <article className="ai-resource" key={resource.id}>
                <span className="ai-resource__icon"><SourceIcon aria-hidden="true" /></span>
                <div className="ai-resource__body">
                  <div className="ai-resource__heading">
                    <h3>{name}</h3>
                    <span>{text.scope[resource.locale]}</span>
                  </div>
                  <p>{description}</p>
                  <code>{resource.path}</code>
                </div>
                <div className="ai-resource__actions">
                  <button
                    type="button"
                    title={text.copyUrl}
                    aria-label={`${text.copyUrl}: ${name}`}
                    onClick={() => copyText(new URL(href, window.location.origin).href, text.copiedUrl)}
                  >
                    <Copy aria-hidden="true" />
                  </button>
                  <a href={href} title={text.open} aria-label={`${text.open}: ${name}`}>
                    <ExternalLink aria-hidden="true" />
                  </a>
                </div>
              </article>
            );
          })}
        </div>
      </div>

      <div className="ai-hub__bootstrap">
        <div className="ai-hub__bootstrap-title">
          <span>{text.bootstrap}</span>
          <strong>{text.bootstrapTitle}</strong>
        </div>
        <p>{text.prompt}</p>
        <button type="button" onClick={() => copyText(text.prompt, text.copiedBootstrap)}>
          <Copy aria-hidden="true" />{text.bootstrapCopy}
        </button>
      </div>

      <footer className="ai-hub__trust">
        {text.trust.map((item) => <span key={item}><ShieldCheck aria-hidden="true" />{item}</span>)}
      </footer>
      <span className="sr-only" aria-live="polite">{notice}</span>
    </section>
  );
}
