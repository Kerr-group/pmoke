'use client';

import React, { useState, useEffect, useRef } from 'react';
import { CheckCircle2, AlertTriangle, XCircle, Copy, Download, RefreshCw, Sparkles, FileCode } from 'lucide-react';

interface Diagnostic {
  kind: 'parse' | 'schema' | 'validation' | 'migration' | 'io';
  path?: string;
  message: string;
  suggestion?: string;
}

interface ConfigSummary {
  version: number;
  scope_model?: string;
  scope_connection?: string;
  generator_model?: string;
  generator_connection?: string;
  sensor_channels: number[];
  reference_channel?: number;
  signal_channels: number[];
  lockin_workers: number;
  plot_mode: string;
}

interface ValidationReport {
  valid: boolean;
  version?: number;
  diagnostics: Diagnostic[];
  warnings: string[];
  normalized_toml?: string;
  summary?: ConfigSummary;
}

const DEFAULT_V4_SAMPLE = `version = 4

[scope]
model = "dsox1204a"
connection = "usbtmc://0x0957/0x1799/MY12345678"

[generator]
model = "dg1022z"
connection = "usbtmc://0x1ab1/0x0642/DG12345678"

[data]
output = "both"
input = "fetch"
screenshot = true

[pulse]
background_before = { start = -1e-6, end = -0.1e-6 }
background_after = { start = 0.1e-6, end = 1.0e-6 }

[reference]
channel = 1
fft_window = "hann"
stride_samples = 1
window_samples = 1000

[lockin]
signal_channels = [1, 2]
workers = 4
stride_samples = 1
filter = { kind = "boxcar_legacy", half_window_cycles = 1.0 }

[phase]
offsets = [0.0, 0.0]

[kerr]
sensor = 1
method = "polar"
factor = 1.0

[plot]
mode = "save"
max_points = 10000
`;

const INVALID_SAMPLE = `version = 4

[scope]
# Missing connection URI!
model = "dsox1204a"

[lockin]
# Invalid negative worker count
workers = -2
`;

export function ConfigValidator({ locale = 'en' }: { locale?: 'en' | 'ja' }) {
  const [tomlInput, setTomlInput] = useState(DEFAULT_V4_SAMPLE);
  const [report, setReport] = useState<ValidationReport | null>(null);
  const [loading, setLoading] = useState(true);
  const [copied, setCopied] = useState(false);
  const [wasmModule, setWasmModule] = useState<any>(null);
  const ariaAnnouncementRef = useRef<HTMLDivElement>(null);

  const isJa = locale === 'ja';

  useEffect(() => {
    let isMounted = true;
    async function loadWasm() {
      try {
        const wasm = await import('../public/wasm/pmoke_web_wasm.js');
        await wasm.default();
        if (isMounted) {
          setWasmModule(wasm);
          setLoading(false);
        }
      } catch (err) {
        console.error('Failed to load Wasm validator:', err);
        if (isMounted) {
          setLoading(false);
        }
      }
    }
    loadWasm();
    return () => {
      isMounted = false;
    };
  }, []);

  useEffect(() => {
    if (!wasmModule) return;
    const timer = setTimeout(() => {
      try {
        const jsonStr = wasmModule.validate_config_toml(tomlInput);
        const parsed = JSON.parse(jsonStr) as ValidationReport;
        setReport(parsed);
      } catch (e) {
        console.error('Validation error:', e);
      }
    }, 150);

    return () => clearTimeout(timer);
  }, [tomlInput, wasmModule]);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(tomlInput);
      setCopied(true);
      if (ariaAnnouncementRef.current) {
        ariaAnnouncementRef.current.textContent = isJa ? '設定をクリップボードにコピー完了。' : 'Configuration copied to clipboard.';
      }
      setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      console.error('Failed to copy:', err);
    }
  };

  const handleDownload = () => {
    const blob = new Blob([tomlInput], { type: 'text/plain;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const link = document.createElement('a');
    link.href = url;
    link.download = 'config.toml';
    link.click();
    URL.revokeObjectURL(url);
  };

  return (
    <div className="my-8 rounded-xl border border-fd-border bg-fd-card p-6 shadow-md transition-all">
      <div className="flex flex-wrap items-center justify-between gap-4 border-b border-fd-border pb-4 mb-4">
        <div className="flex items-center gap-2">
          <FileCode className="h-5 w-5 text-fd-primary" />
          <h3 className="text-lg font-semibold text-fd-foreground">
            {isJa ? 'TOML 設定インタラクティブ検証ツール' : 'Interactive TOML Config Validator'}
          </h3>
          <span className="rounded-full bg-fd-primary/10 px-2.5 py-0.5 text-xs font-medium text-fd-primary">
            Wasm Core
          </span>
        </div>

        <div className="flex items-center gap-2">
          <button
            onClick={() => setTomlInput(DEFAULT_V4_SAMPLE)}
            className="rounded-md border border-fd-border bg-fd-background px-3 py-1.5 text-xs font-medium text-fd-foreground hover:bg-fd-accent hover:text-fd-accent-foreground transition"
          >
            {isJa ? '標準 v4 サンプル' : 'Sample v4'}
          </button>
          <button
            onClick={() => setTomlInput(INVALID_SAMPLE)}
            aria-label={isJa ? 'エラーサンプル読み込み' : 'Load invalid sample'}
            className="rounded-md border border-fd-border bg-fd-background px-3 py-1.5 text-xs font-medium text-fd-foreground hover:bg-fd-accent hover:text-fd-accent-foreground transition"
          >
            {isJa ? 'エラーサンプル' : 'Invalid Sample'}
          </button>
        </div>
      </div>

      {/* Editor & Results layout */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* Editor column */}
        <div className="flex flex-col gap-2">
          <div className="flex items-center justify-between text-xs text-fd-muted-foreground">
            <span>{isJa ? 'TOML 設定入力' : 'TOML Configuration Input'}</span>
            <span>{tomlInput.length} bytes</span>
          </div>
          <textarea
            value={tomlInput}
            onChange={(e) => setTomlInput(e.target.value)}
            placeholder={isJa ? 'TOML設定を入力...' : 'Enter TOML configuration...'}
            className="h-80 w-full rounded-lg border border-fd-border bg-fd-background p-4 font-mono text-sm leading-relaxed text-fd-foreground shadow-inner focus:border-fd-primary focus:outline-none focus:ring-1 focus:ring-fd-primary"
            spellCheck={false}
          />
          <div className="flex items-center justify-end gap-2 pt-2">
            <button
              onClick={handleCopy}
              className="inline-flex items-center gap-1.5 rounded-md bg-fd-secondary px-3 py-1.5 text-xs font-medium text-fd-secondary-foreground hover:bg-fd-secondary/80 transition"
            >
              <Copy className="h-3.5 w-3.5" />
              {copied ? (isJa ? 'コピー完了' : 'Copied!') : (isJa ? 'コピー' : 'Copy')}
            </button>
            <button
              onClick={handleDownload}
              className="inline-flex items-center gap-1.5 rounded-md bg-fd-primary px-3 py-1.5 text-xs font-medium text-fd-primary-foreground hover:bg-fd-primary/90 transition"
            >
              <Download className="h-3.5 w-3.5" />
              {isJa ? 'ダウンロード' : 'Download'}
            </button>
          </div>
        </div>

        {/* Diagnostic Results column */}
        <div className="flex flex-col gap-4 rounded-lg border border-fd-border bg-fd-background/50 p-4">
          <div className="flex items-center justify-between border-b border-fd-border pb-3">
            <span className="text-xs font-medium text-fd-muted-foreground uppercase tracking-wider">
              {isJa ? '検証結果 (Validation Status)' : 'Validation Status'}
            </span>
            {loading ? (
              <span className="inline-flex items-center gap-1.5 text-xs text-fd-muted-foreground">
                <RefreshCw className="h-3.5 w-3.5 animate-spin" />
                {isJa ? 'Wasm 読み込み中...' : 'Loading Wasm...'}
              </span>
            ) : report?.valid ? (
              <span className="inline-flex items-center gap-1.5 rounded-full bg-emerald-500/10 px-3 py-1 text-xs font-semibold text-emerald-500">
                <CheckCircle2 className="h-4 w-4" />
                {isJa ? '設定正常 (Valid)' : 'Valid Configuration'}
              </span>
            ) : (
              <span className="inline-flex items-center gap-1.5 rounded-full bg-rose-500/10 px-3 py-1 text-xs font-semibold text-rose-500">
                <XCircle className="h-4 w-4" />
                {isJa ? '設定エラー (Invalid)' : 'Invalid Configuration'}
              </span>
            )}
          </div>

          {/* Diagnostics list */}
          {report && (
            <div className="space-y-3 overflow-y-auto max-h-64">
              {report.warnings.map((warn, i) => (
                <div key={`warn-${i}`} className="flex items-start gap-2.5 rounded-md border border-amber-500/30 bg-amber-500/10 p-3 text-xs text-amber-600 dark:text-amber-400">
                  <AlertTriangle className="h-4 w-4 shrink-0 mt-0.5" />
                  <div>
                    <span className="font-semibold">{isJa ? '警告: ' : 'Warning: '}</span>
                    {warn}
                  </div>
                </div>
              ))}

              {report.diagnostics.map((diag, i) => (
                <div key={`diag-${i}`} className="flex items-start gap-2.5 rounded-md border border-rose-500/30 bg-rose-500/10 p-3 text-xs text-rose-600 dark:text-rose-400">
                  <XCircle className="h-4 w-4 shrink-0 mt-0.5" />
                  <div className="space-y-1">
                    <div className="font-semibold">
                      [{diag.kind.toUpperCase()}] {diag.path ? `@ ${diag.path}` : ''}
                    </div>
                    <div>{diag.message}</div>
                    {diag.suggestion && (
                      <div className="text-fd-muted-foreground italic mt-1">
                        💡 {diag.suggestion}
                      </div>
                    )}
                  </div>
                </div>
              ))}

              {report.valid && report.summary && (
                <div className="rounded-md border border-fd-border bg-fd-card p-4 space-y-2 text-xs">
                  <div className="font-semibold text-fd-foreground flex items-center gap-1.5">
                    <Sparkles className="h-4 w-4 text-fd-primary" />
                    {isJa ? '解析・構造サマリー' : 'Configuration Structure Summary'}
                  </div>
                  <div className="grid grid-cols-2 gap-2 text-fd-muted-foreground pt-1">
                    <div>Version: <span className="font-mono text-fd-foreground">{report.summary.version}</span></div>
                    <div>Oscilloscope: <span className="font-mono text-fd-foreground">{report.summary.scope_model || 'None'}</span></div>
                    <div>Generator: <span className="font-mono text-fd-foreground">{report.summary.generator_model || 'None'}</span></div>
                    <div>Lockin Workers: <span className="font-mono text-fd-foreground">{report.summary.lockin_workers}</span></div>
                    <div>Plot Mode: <span className="font-mono text-fd-foreground">{report.summary.plot_mode}</span></div>
                  </div>
                </div>
              )}
            </div>
          )}
        </div>
      </div>

      <div ref={ariaAnnouncementRef} className="sr-only" aria-live="polite" />
    </div>
  );
}
