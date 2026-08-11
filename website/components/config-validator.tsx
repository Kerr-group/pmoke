'use client';

import { useEffect, useMemo, useRef, useState } from 'react';
import {
  AlertTriangle,
  Check,
  Copy,
  Download,
  FileCode2,
  RefreshCw,
  RotateCcw,
  WandSparkles,
  X,
} from 'lucide-react';
import { basePath } from '@/lib/shared';
import { configSchemaVersion, sourceCommit } from '@/lib/version';

type Diagnostic = {
  code: string;
  severity: 'error' | 'warning';
  path: string | null;
  span: { start: number; end: number; line: number; column: number } | null;
  message: string;
  suggestion: string | null;
};

type ConfigSummary = {
  version: number;
  scope_model: string;
  scope_connection: string;
  generator_model: string | null;
  generator_connection: string | null;
  sensor_channels: number[];
  reference_channel: number;
  signal_channels: number[];
  lockin_filter: string;
  lockin_workers: number;
  plot_mode: string;
};

type ValidationReport = {
  format_version: number;
  core_version: string;
  core_commit: string;
  schema_version: number | null;
  valid: boolean;
  diagnostics: Diagnostic[];
  normalized_toml: string | null;
  summary: ConfigSummary | null;
};

type WorkerResponse =
  | { type: 'ready'; build: string }
  | { type: 'result'; id: number; report: ValidationReport }
  | { type: 'error'; id?: number; message: string };

const REPORT_FORMAT_VERSION = 1;

const DEFAULT_V5_SAMPLE = `version = 5

[scope]
model = "DHO5108"
connection = "tcp://192.0.2.10:55255"

[data]
output = "raw"
input = "raw"
screenshot = true

[[sensors]]
channel = 1
scale = { max_abs = 55.0, polarity = -1 }
label = '$\\mu_0H$'
unit = "T"

[pulse]
background_before = { start = -5e-3, end = -0.1e-3 }
background_after = { start = 43e-3, end = 46e-3 }

[reference]
channel = 2
fft_window = { start = 0.0, end = 15e-3 }
stride_samples = 10_000
window_samples = 1_000

[lockin]
signal_channels = [3]
workers = 2
stride_samples = 100
filter = { kind = "boxcar_legacy", half_window_cycles = 1.0 }

[phase]
offsets = [0, 0, 0, 0, 0, 0]

[kerr]
sensor = 1
method = "harmonics"
factor = -1.0

[plot]
mode = "save"
decimation = "min_max"
`;

const INVALID_SAMPLE = DEFAULT_V5_SAMPLE.replace('channel = 2', 'channel = 1').replace(
  'workers = 2',
  'workers = 0',
);

const copy = {
  en: {
    title: 'TOML config validator',
    local: 'Browser-local Wasm',
    sample: 'Load valid sample',
    invalid: 'Load diagnostic sample',
    input: 'Configuration input',
    copy: 'Copy',
    copied: 'Copied',
    download: 'Download',
    normalize: 'Use normalized config',
    loading: 'Loading Wasm',
    validating: 'Validating',
    valid: 'Valid config',
    invalidStatus: 'Config errors',
    unavailable: 'Validator unavailable',
    retry: 'Retry Wasm',
    diagnostics: 'Diagnostics',
    noDiagnostics: 'No diagnostics',
    summary: 'Resolved structure',
    mismatch: 'Validator build does not match this documentation build.',
    copySuccess: 'Configuration copied to clipboard.',
    copyFailure: 'Clipboard copy failed.',
    normalized: 'Normalized configuration loaded.',
    workerFailure: 'Wasm validator failed. Input remains available.',
    samples: 'Configuration samples',
  },
  ja: {
    title: 'TOML 設定検証',
    local: 'ブラウザ内 Wasm',
    sample: 'サンプルを読み込む',
    invalid: 'エラー例を読み込む',
    input: '設定入力',
    copy: 'コピー',
    copied: 'コピー済み',
    download: 'ダウンロード',
    normalize: '正規化した設定を使用',
    loading: 'Wasmを読み込み中',
    validating: '検証中',
    valid: '有効な設定',
    invalidStatus: '設定エラー',
    unavailable: '検証機能は利用不可',
    retry: 'Wasmを再読み込み',
    diagnostics: '診断',
    noDiagnostics: '診断なし',
    summary: '解決後の構造',
    mismatch: 'ドキュメントと検証コアのビルド不一致。',
    copySuccess: '設定をクリップボードにコピー済み。',
    copyFailure: 'クリップボードへのコピー失敗。',
    normalized: '正規化した設定を読み込み済み。',
    workerFailure: 'Wasm検証コアのエラー。入力内容は保持。',
    samples: '設定サンプル',
  },
} as const;

export function ConfigValidator({ locale = 'en' }: { locale?: 'en' | 'ja' }) {
  const text = copy[locale];
  const [input, setInput] = useState(DEFAULT_V5_SAMPLE);
  const [report, setReport] = useState<ValidationReport>();
  const [workerState, setWorkerState] = useState<'loading' | 'ready' | 'error'>('loading');
  const [validating, setValidating] = useState(false);
  const [copied, setCopied] = useState(false);
  const [announcement, setAnnouncement] = useState('');
  const [workerEpoch, setWorkerEpoch] = useState(0);
  const workerRef = useRef<Worker | null>(null);
  const requestId = useRef(0);
  const latestResultId = useRef(0);
  const byteLength = useMemo(() => new TextEncoder().encode(input).byteLength, [input]);

  useEffect(() => {
    const worker = new Worker(`${basePath}/workers/config-validator.worker.js`, {
      type: 'module',
      name: 'pmoke-config-validator',
    });
    workerRef.current = worker;
    worker.onmessage = (event: MessageEvent<WorkerResponse>) => {
      if (workerRef.current !== worker) return;
      const response = event.data;
      if (response.type === 'ready') {
        setWorkerState('ready');
      } else if (response.type === 'result' && response.id === latestResultId.current) {
        setReport(response.report);
        setValidating(false);
      } else if (
        response.type === 'error' &&
        (response.id === undefined || response.id === latestResultId.current)
      ) {
        setReport(undefined);
        setWorkerState('error');
        setValidating(false);
        setAnnouncement(text.workerFailure);
      }
    };
    worker.onerror = () => {
      if (workerRef.current !== worker) return;
      setReport(undefined);
      setWorkerState('error');
      setValidating(false);
      setAnnouncement(text.workerFailure);
    };
    worker.postMessage({ type: 'init', basePath });
    return () => {
      worker.terminate();
      if (workerRef.current === worker) workerRef.current = null;
    };
  }, [text.workerFailure, workerEpoch]);

  useEffect(() => {
    if (workerState !== 'ready' || !workerRef.current) return;
    const id = ++requestId.current;
    latestResultId.current = id;
    setValidating(true);
    const timer = window.setTimeout(() => {
      workerRef.current?.postMessage({ type: 'validate', id, input });
    }, 180);
    return () => window.clearTimeout(timer);
  }, [input, workerState]);

  const buildMismatch =
    report !== undefined &&
    (report.format_version !== REPORT_FORMAT_VERSION ||
      report.schema_version !== configSchemaVersion ||
      (sourceCommit !== 'development' &&
        report.core_commit !== 'development' &&
        report.core_commit !== sourceCommit));
  const errors = report?.diagnostics.filter((item) => item.severity === 'error') ?? [];
  const warnings = report?.diagnostics.filter((item) => item.severity === 'warning') ?? [];

  async function handleCopy() {
    try {
      await navigator.clipboard.writeText(input);
      setCopied(true);
      setAnnouncement(text.copySuccess);
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      setAnnouncement(text.copyFailure);
    }
  }

  function handleDownload() {
    const url = URL.createObjectURL(new Blob([input], { type: 'text/plain;charset=utf-8' }));
    const link = document.createElement('a');
    link.href = url;
    link.download = 'config.toml';
    document.body.append(link);
    link.click();
    link.remove();
    window.setTimeout(() => URL.revokeObjectURL(url), 0);
  }

  function useNormalized() {
    if (!report?.normalized_toml) return;
    updateInput(report.normalized_toml);
    setAnnouncement(text.normalized);
  }

  function updateInput(value: string) {
    latestResultId.current = ++requestId.current;
    setReport(undefined);
    if (workerState === 'ready') setValidating(true);
    setInput(value);
  }

  function retryWorker() {
    setReport(undefined);
    setWorkerState('loading');
    setWorkerEpoch((value) => value + 1);
  }

  const status = workerState === 'loading'
    ? { icon: RefreshCw, label: text.loading, tone: 'pending' }
    : workerState === 'error'
      ? { icon: X, label: text.unavailable, tone: 'error' }
      : validating || !report
        ? { icon: RefreshCw, label: text.validating, tone: 'pending' }
        : report.valid
          ? { icon: Check, label: text.valid, tone: 'valid' }
          : { icon: X, label: text.invalidStatus, tone: 'error' };
  const StatusIcon = status.icon;

  return (
    <section className="config-validator" aria-labelledby="config-validator-title">
      <header className="config-validator__header">
        <div>
          <span className="config-validator__eyebrow"><FileCode2 aria-hidden="true" /> {text.local}</span>
          <h3 id="config-validator-title">{text.title}</h3>
        </div>
        <div className="config-validator__samples" aria-label={text.samples}>
          <button type="button" onClick={() => updateInput(DEFAULT_V5_SAMPLE)} title={text.sample}>
            <RotateCcw aria-hidden="true" /> {text.sample}
          </button>
          <button type="button" onClick={() => updateInput(INVALID_SAMPLE)} title={text.invalid}>
            <AlertTriangle aria-hidden="true" /> {text.invalid}
          </button>
        </div>
      </header>

      <div className="config-validator__workspace">
        <div className="config-validator__editor">
          <label htmlFor={`config-input-${locale}`}>{text.input}</label>
          <span className="config-validator__bytes">{byteLength.toLocaleString()} / 1,048,576 B</span>
          <textarea
            id={`config-input-${locale}`}
            value={input}
            onChange={(event) => updateInput(event.target.value)}
            spellCheck={false}
            aria-describedby={`config-status-${locale}`}
          />
          <div className="config-validator__actions">
            <button type="button" onClick={handleCopy} title={text.copy}>
              <Copy aria-hidden="true" /> {copied ? text.copied : text.copy}
            </button>
            <button type="button" onClick={handleDownload} title={text.download}>
              <Download aria-hidden="true" /> {text.download}
            </button>
            {report?.valid && report.normalized_toml && (
              <button type="button" onClick={useNormalized} title={text.normalize}>
                <WandSparkles aria-hidden="true" /> {text.normalize}
              </button>
            )}
          </div>
        </div>

        <div className="config-validator__results">
          <div id={`config-status-${locale}`} className={`config-validator__status is-${status.tone}`} role="status">
            <StatusIcon className={status.tone === 'pending' ? 'is-spinning' : undefined} aria-hidden="true" />
            <span>{status.label}</span>
            {report && <code>core {report.core_version}</code>}
          </div>
          {workerState === 'error' && (
            <button className="config-validator__retry" type="button" onClick={retryWorker}>
              <RefreshCw aria-hidden="true" /> {text.retry}
            </button>
          )}
          {buildMismatch && <p className="config-validator__mismatch"><AlertTriangle aria-hidden="true" /> {text.mismatch}</p>}

          <div className="config-validator__diagnostics">
            <h4>{text.diagnostics} <span>{errors.length} / {warnings.length}</span></h4>
            {report && report.diagnostics.length === 0 && <p>{text.noDiagnostics}</p>}
            {report?.diagnostics.map((diagnostic, index) => (
              <article className={`diagnostic is-${diagnostic.severity}`} key={`${diagnostic.code}-${diagnostic.path}-${index}`}>
                {diagnostic.severity === 'error' ? <X aria-hidden="true" /> : <AlertTriangle aria-hidden="true" />}
                <div>
                  <div className="diagnostic__meta">
                    <code>{diagnostic.code}</code>
                    {diagnostic.path && <code>{diagnostic.path}</code>}
                    {diagnostic.span && <span>L{diagnostic.span.line}:C{diagnostic.span.column}</span>}
                  </div>
                  <p>{diagnostic.message}</p>
                  {diagnostic.suggestion && <small>{diagnostic.suggestion}</small>}
                </div>
              </article>
            ))}
          </div>

          {report?.valid && report.summary && (
            <dl className="config-validator__summary" aria-label={text.summary}>
              <div><dt>scope</dt><dd>{report.summary.scope_model}</dd></div>
              <div><dt>channels</dt><dd>{[...report.summary.sensor_channels, report.summary.reference_channel, ...report.summary.signal_channels].join(' / ')}</dd></div>
              <div><dt>filter</dt><dd>{report.summary.lockin_filter}</dd></div>
              <div><dt>workers</dt><dd>{report.summary.lockin_workers}</dd></div>
            </dl>
          )}
        </div>
      </div>
      <p className="sr-only" aria-live="polite">{announcement}</p>
    </section>
  );
}
