'use client';

import { Check, Copy, GitBranch, Quote } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { pmokeVersion, sourceCommit } from '@/lib/version';

type Locale = 'en' | 'ja';

const repositoryUrl = 'https://github.com/Kerr-group/pmoke';

const copy = {
  en: {
    aria: 'pmoke software citation',
    kicker: 'SOFTWARE CITATION',
    title: 'Reproducible attribution',
    version: 'Version',
    source: 'Source',
    license: 'License',
    plain: 'Plain text',
    bibtex: 'BibTeX',
    copyPlain: 'Copy plain-text citation',
    copyBibtex: 'Copy BibTeX citation',
    copiedPlain: 'Plain-text citation copied',
    copiedBibtex: 'BibTeX citation copied',
    copyFailed: 'Clipboard unavailable',
    repository: 'Open source repository',
  },
  ja: {
    aria: 'pmoke software citation',
    kicker: 'SOFTWARE CITATION',
    title: '再現可能な帰属情報',
    version: 'Version',
    source: 'Source',
    license: 'License',
    plain: 'テキスト',
    bibtex: 'BibTeX',
    copyPlain: 'テキスト引用のコピー',
    copyBibtex: 'BibTeX引用のコピー',
    copiedPlain: 'テキスト引用のコピー完了',
    copiedBibtex: 'BibTeX引用のコピー完了',
    copyFailed: 'クリップボード利用不可',
    repository: 'source repositoryの表示',
  },
} as const;

export function CitationPanel({ locale }: { locale: Locale }) {
  const text = copy[locale];
  const [notice, setNotice] = useState('');
  const noticeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const shortCommit = sourceCommit === 'development' ? sourceCommit : sourceCommit.slice(0, 12);
  const sourceNote = sourceCommit === 'development' ? 'development build' : `source ${sourceCommit}`;
  const citation = `Kerr-group contributors (2026). pmoke (Version ${pmokeVersion}; ${sourceNote}) [Computer software]. GitHub. ${repositoryUrl}`;
  const bibtex = useMemo(() => [
    '@misc{kerr_group_pmoke_2026,',
    '  author  = {{Kerr-group contributors}},',
    '  title   = {pmoke: Pulsed-MOKE acquisition and analysis software},',
    '  year    = {2026},',
    `  version = {${pmokeVersion}},`,
    `  url     = {${repositoryUrl}},`,
    `  note    = {${sourceNote}},`,
    '}',
  ].join('\n'), [sourceNote]);

  useEffect(() => () => {
    if (noticeTimer.current) clearTimeout(noticeTimer.current);
  }, []);

  const copyText = async (value: string, success: string) => {
    try {
      await navigator.clipboard.writeText(value);
      setNotice(success);
    } catch {
      setNotice(text.copyFailed);
    }
    if (noticeTimer.current) clearTimeout(noticeTimer.current);
    noticeTimer.current = setTimeout(() => setNotice(''), 2_000);
  };

  return (
    <section className="citation-panel" aria-label={text.aria}>
      <header className="citation-panel__header">
        <span className="citation-panel__mark"><Quote aria-hidden="true" /></span>
        <div>
          <span>{text.kicker}</span>
          <h2>{text.title}</h2>
        </div>
        <a href={repositoryUrl} aria-label={text.repository} title={text.repository}>
          <GitBranch aria-hidden="true" />
        </a>
      </header>

      <dl className="citation-panel__metadata">
        <div><dt>{text.version}</dt><dd>{pmokeVersion}</dd></div>
        <div><dt>{text.source}</dt><dd title={sourceCommit}>{shortCommit}</dd></div>
        <div><dt>{text.license}</dt><dd>Apache-2.0</dd></div>
      </dl>

      <div className="citation-panel__formats">
        <CitationFormat
          label={text.plain}
          copyLabel={text.copyPlain}
          value={citation}
          onCopy={() => copyText(citation, text.copiedPlain)}
        />
        <CitationFormat
          label={text.bibtex}
          copyLabel={text.copyBibtex}
          value={bibtex}
          onCopy={() => copyText(bibtex, text.copiedBibtex)}
        />
      </div>
      <span className="sr-only" aria-live="polite">{notice}</span>
    </section>
  );
}

function CitationFormat({
  label,
  copyLabel,
  value,
  onCopy,
}: {
  label: string;
  copyLabel: string;
  value: string;
  onCopy: () => void;
}) {
  return (
    <div className="citation-format">
      <header>
        <span><Check aria-hidden="true" />{label}</span>
        <button type="button" aria-label={copyLabel} title={copyLabel} onClick={onCopy}>
          <Copy aria-hidden="true" />
        </button>
      </header>
      <pre tabIndex={0}><code>{value}</code></pre>
    </div>
  );
}
