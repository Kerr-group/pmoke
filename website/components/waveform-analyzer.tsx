'use client';

import {
  Check,
  CircleAlert,
  Copy,
  Download,
  FileUp,
  Play,
  RotateCcw,
  Square,
  Waves,
} from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { basePath } from '@/lib/shared';

type Locale = 'en' | 'ja';
type SourceMode = 'synthetic' | 'csv';
type RunState = 'loading' | 'ready' | 'running' | 'complete' | 'error';

type Parameters = {
  samples: number;
  sampleRateHz: number;
  referenceFrequencyHz: number;
  amplitude: number;
  signalPhaseRad: number;
  noiseRms: number;
  kerrAngleRad: number;
  seed: number;
  referencePhaseRad: number;
  halfWindowCycles: number;
  strideSamples: number;
  harmonic: number;
  rotationRad: number;
  kerrFactor: number;
};

type AnalysisLimits = {
  max_demo_samples: number;
  max_upload_samples: number;
  max_upload_bytes: number;
  max_total_harmonic_points: number;
  lockin_header_values: number;
};

type AnalysisResult = {
  source: {
    kind: 'synthetic' | 'upload';
    name: string;
    samples: number;
    startTimeS: number;
    sampleRateHz: number;
  };
  parameters: Parameters;
  metadata: {
    outputSamples: number;
    outputRateHz: number;
    halfWindowS: number;
    supportS: number;
    estimatedEnbwHz: number;
    firstInputIndex: number;
    lastInputIndex: number;
    selectedHarmonic: number;
    modulationDepth: number;
    elapsedMs: number;
    algorithm: string;
    parity: string;
  };
  warnings: string[];
  display: {
    input: { time: Float64Array; value: Float64Array };
    lockin: [
      Float64Array,
      Float64Array,
      Float64Array,
      Float64Array,
      Float64Array,
      Float64Array,
    ];
    response: { frequency: Float64Array; magnitude: Float64Array };
  };
  export: {
    time: Float64Array;
    x: Float64Array;
    y: Float64Array;
    inPhase: Float64Array;
    outOfPhase: Float64Array;
    magnitude: Float64Array;
    phase: Float64Array;
    kerr: Float64Array;
  };
};

type WorkerMessage =
  | { type: 'ready'; limits: AnalysisLimits; build: string }
  | { type: 'progress'; generation: number; fraction: number; stage: string }
  | { type: 'result'; generation: number; result: AnalysisResult }
  | { type: 'error'; generation: number; message: string };

const DEFAULTS: Parameters = {
  samples: 20_000,
  sampleRateHz: 100_000,
  referenceFrequencyHz: 1_000,
  amplitude: 1,
  signalPhaseRad: 0.2,
  noiseRms: 0.002,
  kerrAngleRad: 0.01,
  seed: 42,
  referencePhaseRad: 0,
  halfWindowCycles: 1,
  strideSamples: 20,
  harmonic: 1,
  rotationRad: 0.2,
  kerrFactor: 1,
};

const COPY = {
  en: {
    source: 'SOURCE', synthetic: 'Synthetic', csv: 'Local CSV', upload: 'Select CSV',
    noFile: 'No file selected', run: 'Run analysis', cancel: 'Cancel analysis', reset: 'Reset',
    retry: 'Reload core', loading: 'Loading analysis core', ready: 'Ready', running: 'ANALYZING',
    complete: 'PARITY VERIFIED', error: 'Analysis unavailable', progress: 'RUN PROGRESS',
    signal: 'INPUT WAVEFORM', lockin: 'LOCK-IN X / Y', polar: 'MAGNITUDE / PHASE',
    kerr: 'KERR / FILTER RESPONSE', controls: 'ANALYSIS CONTROL', result: 'RUN METRICS',
    filter: 'FILTER', filterValue: 'Boxcar legacy', parity: 'Native exact', unsupported: 'Not in browser parity',
    export: 'Export CSV', copy: 'Copy summary', copied: 'Summary copied', samples: 'Samples',
    sampleRate: 'Sample rate', frequency: 'Reference', amplitude: 'Amplitude', phase: 'Signal phase',
    noise: 'Noise RMS', angle: 'Kerr angle', window: 'Half-window', stride: 'Stride',
    harmonic: 'Harmonic', rotation: 'Rotation', factor: 'Kerr factor', seed: 'Seed',
    elapsed: 'Runtime', inputRate: 'Input rate', outputRate: 'Output rate', enbw: 'Estimated ENBW', support: 'Support',
    cutoff: 'Cutoff', cutoffValue: 'N/A · boxcar', trim: 'Input trim', settling: 'Settling', settlingValue: 'N/A · boxcar',
    modulation: 'Modulation depth', output: 'Output samples', kerrMedian: 'Kerr median', inputUnit: 'signal', timeUnit: 'time (s)',
    responseUnit: 'frequency (Hz)', csvHint: 'time,signal or signal-only; uniform finite values',
    uploadLimit: '16 MiB / 1,000,000 samples', local: 'Browser-local processing',
    unsupportedList: 'historical FIR/IIR · phase fitting · standard Kerr',
    stagePrepare: 'Preparing waveform', stageLockin: 'Lock-in harmonic', stageKerr: 'Harmonics Kerr',
    stageDecimate: 'Display decimation', stageComplete: 'Analysis complete',
    warningLong: 'Long boxcar support reduces time resolution.',
    warningSparse: 'Output sampling is sparse relative to the reference frequency.',
    warningLocal: 'Local CSV data remains inside this browser session.',
  },
  ja: {
    source: '入力', synthetic: '合成波形', csv: 'ローカルCSV', upload: 'CSVを選択',
    noFile: 'ファイルが選択されていません', run: '解析を実行', cancel: '解析を中断', reset: '初期化',
    retry: 'コアを再読み込み', loading: '解析コアを読み込み中', ready: '実行待機', running: '解析中',
    complete: '一致性検証済み', error: '解析機能の利用不可', progress: '実行進捗',
    signal: '入力波形', lockin: 'LOCK-IN X / Y', polar: '振幅 / 位相',
    kerr: 'KERR / フィルター応答', controls: '解析制御', result: '実行指標',
    filter: 'フィルター', filterValue: '従来型Boxcar', parity: 'ネイティブと完全一致',
    unsupported: 'ブラウザの一致対象外', export: 'CSVを出力', copy: '要約をコピー', copied: '要約コピー済み',
    samples: 'サンプル数', sampleRate: 'サンプルレート', frequency: '参照周波数', amplitude: '振幅',
    phase: '信号位相', noise: 'ノイズRMS', angle: 'Kerr角', window: '半窓周期',
    stride: 'ストライド', harmonic: '高調波', rotation: '位相回転', factor: 'Kerr係数', seed: '乱数種',
    elapsed: '計算時間', inputRate: '入力レート', outputRate: '出力レート', enbw: '推定 ENBW', support: '窓幅',
    cutoff: 'カットオフ', cutoffValue: '対象外・Boxcar', trim: '入力トリム', settling: '整定時間', settlingValue: '対象外・Boxcar',
    modulation: '変調深度', output: '出力点数', kerrMedian: 'Kerr 中央値', inputUnit: '信号', timeUnit: '時間 (s)',
    responseUnit: '周波数 (Hz)', csvHint: 'time,signalまたは信号単列（有限・等間隔の値）',
    uploadLimit: '16 MiB / 1,000,000 点', local: 'ブラウザ内処理',
    unsupportedList: '過去のFIR/IIR（移行専用）・位相フィッティング・標準Kerr',
    stagePrepare: '波形準備', stageLockin: 'Lock-in高調波', stageKerr: '高調波Kerrの計算',
    stageDecimate: '表示用間引き', stageComplete: '解析完了',
    warningLong: '長いBoxcar窓による時間分解能低下',
    warningSparse: '参照周波数に対して疎な出力サンプリング',
    warningLocal: 'ローカルCSVのブラウザセッション内保持',
  },
} as const;

export function WaveformAnalyzer({ locale = 'en' }: { locale?: Locale }) {
  const text = COPY[locale];
  const workerRef = useRef<Worker | null>(null);
  const generationRef = useRef(0);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const parametersRef = useRef(DEFAULTS);
  const limitsRef = useRef<AnalysisLimits | null>(null);
  const fileRef = useRef<File | null>(null);
  const sourceModeRef = useRef<SourceMode>('synthetic');
  const autoRunRef = useRef(true);
  const ignoreResultsRef = useRef(false);
  const [parameters, setParametersState] = useState(DEFAULTS);
  const [sourceMode, setSourceModeState] = useState<SourceMode>('synthetic');
  const [file, setFileState] = useState<File | null>(null);
  const [state, setState] = useState<RunState>('loading');
  const [limits, setLimits] = useState<AnalysisLimits | null>(null);
  const [build, setBuild] = useState('');
  const [progress, setProgress] = useState<{ fraction: number; stage: string }>({
    fraction: 0,
    stage: text.loading,
  });
  const [result, setResult] = useState<AnalysisResult | null>(null);
  const [error, setError] = useState('');
  const [copied, setCopied] = useState(false);
  const [retryAvailable, setRetryAvailable] = useState(false);
  const busy = state === 'loading' || state === 'running';

  const setParameters = useCallback((next: Parameters | ((current: Parameters) => Parameters)) => {
    setParametersState((current) => {
      const value = typeof next === 'function' ? next(current) : next;
      parametersRef.current = value;
      return value;
    });
  }, []);

  const setSourceMode = useCallback((mode: SourceMode) => {
    sourceModeRef.current = mode;
    setSourceModeState(mode);
  }, []);

  const setFile = useCallback((next: File | null) => {
    fileRef.current = next;
    setFileState(next);
  }, []);

  const clearTimeoutHandle = useCallback(() => {
    if (timeoutRef.current) clearTimeout(timeoutRef.current);
    timeoutRef.current = null;
  }, []);

  const runAnalysis = useCallback(async () => {
    const worker = workerRef.current;
    if (!worker) return;
    const generation = ++generationRef.current;
    ignoreResultsRef.current = false;
    clearTimeoutHandle();
    setState('running');
    setError('');
    setCopied(false);
    setProgress({ fraction: 0.01, stage: text.running });
    let source: { type: 'synthetic' } | { type: 'csv'; name: string; buffer: ArrayBuffer } = {
      type: 'synthetic',
    };
    const transfers: Transferable[] = [];
    if (sourceModeRef.current === 'csv') {
      const selected = fileRef.current;
      if (!selected) {
        setState('error');
        setError(locale === 'ja' ? 'CSV ファイル未選択' : 'csv_required: select a CSV file');
        return;
      }
      const currentLimits = limitsRef.current;
      if (currentLimits && selected.size > currentLimits.max_upload_bytes) {
        setState('error');
        setError(`input_too_large: CSV exceeds ${currentLimits.max_upload_bytes} bytes`);
        return;
      }
      const buffer = await selected.arrayBuffer();
      if (generation !== generationRef.current) return;
      source = { type: 'csv', name: selected.name, buffer };
      transfers.push(buffer);
    }
    worker.postMessage(
      { type: 'run', generation, source, parameters: parametersRef.current },
      transfers,
    );
    timeoutRef.current = setTimeout(() => {
      if (generation !== generationRef.current) return;
      worker.terminate();
      workerRef.current = null;
      setRetryAvailable(true);
      setState('error');
      setError(locale === 'ja' ? '解析時間上限超過・入力内容の保持' : 'analysis_timeout: input preserved');
    }, 20_000);
  }, [clearTimeoutHandle, locale, text.running]);

  const createWorker = useCallback(() => {
    clearTimeoutHandle();
    workerRef.current?.terminate();
    generationRef.current += 1;
    setState('loading');
    setRetryAvailable(false);
    setError('');
    setProgress({ fraction: 0, stage: text.loading });
    const worker = new Worker(`${basePath}/workers/waveform-analyzer.worker.js`, { type: 'module' });
    workerRef.current = worker;
    worker.onmessage = (event: MessageEvent<WorkerMessage>) => {
      if (workerRef.current !== worker) return;
      const message = event.data;
      if (message.type === 'ready') {
        limitsRef.current = message.limits;
        setLimits(message.limits);
        setBuild(message.build);
        setState('ready');
        if (autoRunRef.current) {
          autoRunRef.current = false;
          queueMicrotask(() => void runAnalysis());
        }
        return;
      }
      if (message.type === 'error' && message.generation === 0) {
        clearTimeoutHandle();
        worker.terminate();
        workerRef.current = null;
        setRetryAvailable(true);
        setError(message.message);
        setState('error');
        return;
      }
      if (message.generation !== generationRef.current) return;
      if (ignoreResultsRef.current && message.type !== 'progress') return;
      if (message.type === 'progress') {
        setProgress({ fraction: message.fraction, stage: progressLabel(locale, message.stage) });
      } else if (message.type === 'result') {
        clearTimeoutHandle();
        setResult(message.result);
        setState('complete');
      } else if (message.type === 'error') {
        clearTimeoutHandle();
        setError(message.message);
        setState('error');
      }
    };
    worker.onerror = (event) => {
      if (workerRef.current !== worker) return;
      clearTimeoutHandle();
      worker.terminate();
      workerRef.current = null;
      setRetryAvailable(true);
      setState('error');
      setError(event.message || (locale === 'ja' ? 'Worker 障害・入力内容の保持' : 'worker_failure: input preserved'));
    };
    worker.postMessage({ type: 'init', basePath });
  }, [clearTimeoutHandle, locale, runAnalysis, text.loading]);

  useEffect(() => {
    const start = setTimeout(createWorker, 0);
    return () => {
      clearTimeout(start);
      clearTimeoutHandle();
      workerRef.current?.terminate();
    };
  }, [clearTimeoutHandle, createWorker]);

  const cancel = useCallback(() => {
    autoRunRef.current = false;
    ignoreResultsRef.current = true;
    createWorker();
    setError(locale === 'ja' ? '解析中断・入力内容の保持' : 'analysis_cancelled: input preserved');
  }, [createWorker, locale]);

  const retryWorker = useCallback(() => {
    autoRunRef.current = false;
    createWorker();
  }, [createWorker]);

  const reset = useCallback(() => {
    autoRunRef.current = false;
    setParameters(DEFAULTS);
    setSourceMode('synthetic');
    setFile(null);
    setResult(null);
    setError('');
    createWorker();
  }, [createWorker, setFile, setParameters, setSourceMode]);

  const copySummary = useCallback(async () => {
    if (!result) return;
    try {
      await navigator.clipboard.writeText(summaryText(result));
      setCopied(true);
    } catch {
      setError(locale === 'ja' ? 'クリップボード利用不可' : 'clipboard_unavailable');
      setState('error');
    }
  }, [locale, result]);

  const exportCsv = useCallback(() => {
    if (!result) return;
    const rows = [
      '# pmoke waveform analysis',
      `# algorithm=${result.metadata.algorithm}`,
      `# parity=${result.metadata.parity}`,
      `# sample_rate_hz=${result.source.sampleRateHz}`,
      `# output_rate_hz=${result.metadata.outputRateHz}`,
      `# estimated_enbw_hz=${result.metadata.estimatedEnbwHz}`,
      `# support_s=${result.metadata.supportS}`,
      `# first_input_index=${result.metadata.firstInputIndex}`,
      `# last_input_index=${result.metadata.lastInputIndex}`,
      `# warnings=${result.warnings.join('|') || 'none'}`,
      'time_s,x_v,y_v,in_phase_v,out_of_phase_v,magnitude_v,phase_rad,kerr_rad',
    ];
    for (let index = 0; index < result.export.time.length; index += 1) {
      rows.push([
        result.export.time[index], result.export.x[index], result.export.y[index],
        result.export.inPhase[index], result.export.outOfPhase[index], result.export.magnitude[index],
        result.export.phase[index], result.export.kerr[index],
      ].map((value) => value.toExponential(12)).join(','));
    }
    const url = URL.createObjectURL(new Blob([rows.join('\n')], { type: 'text/csv;charset=utf-8' }));
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = 'pmoke-waveform-analysis.csv';
    anchor.click();
    setTimeout(() => URL.revokeObjectURL(url), 0);
  }, [result]);

  const statusLabel = state === 'loading' ? text.loading : state === 'running' ? text.running
    : state === 'complete' ? text.complete : state === 'error' ? text.error : text.ready;

  return (
    <div className="waveform-analyzer" data-state={state}>
      <header className="analyzer-commandbar">
        <div className="analyzer-source" role="group" aria-label={text.source}>
          <span className="analyzer-kicker">{text.source}</span>
          <button type="button" className={sourceMode === 'synthetic' ? 'active' : ''} disabled={busy} onClick={() => setSourceMode('synthetic')}>
            <Waves size={15} aria-hidden="true" />{text.synthetic}
          </button>
          <button type="button" className={sourceMode === 'csv' ? 'active' : ''} disabled={busy} onClick={() => setSourceMode('csv')}>
            <FileUp size={15} aria-hidden="true" />{text.csv}
          </button>
        </div>
        <div className={`analyzer-status analyzer-status-${state}`} aria-live="polite">
          <span className="status-pulse" aria-hidden="true" />
          <strong>{statusLabel}</strong>
          {build && <small>{build.split(';')[0]}</small>}
        </div>
        <div className="analyzer-actions">
          {state === 'running' ? (
            <button type="button" className="danger" onClick={cancel}><Square size={15} aria-hidden="true" />{text.cancel}</button>
          ) : (
            <button type="button" className="primary" onClick={() => void runAnalysis()} disabled={state === 'loading' || retryAvailable}>
              <Play size={15} fill="currentColor" aria-hidden="true" />{text.run}
            </button>
          )}
          <IconButton label={text.reset} onClick={reset}><RotateCcw size={16} /></IconButton>
        </div>
      </header>

      {sourceMode === 'csv' && (
        <div className="analyzer-upload">
          <label>
            <FileUp size={16} aria-hidden="true" />
            <span>{text.upload}</span>
            <input type="file" accept=".csv,text/csv,text/plain" disabled={busy} onChange={(event) => setFile(event.target.files?.[0] ?? null)} />
          </label>
          <strong>{file?.name ?? text.noFile}</strong>
          <span>{text.csvHint}</span><i>{text.uploadLimit}</i>
        </div>
      )}

      {(state === 'running' || state === 'loading') && (
        <div className="analyzer-progress" aria-label={text.progress}>
          <span style={{ width: `${Math.max(2, progress.fraction * 100)}%` }} />
          <strong>{progress.stage}</strong><small>{Math.round(progress.fraction * 100)}%</small>
        </div>
      )}

      {state === 'error' && (
        <div className="analyzer-error" role="alert">
          <CircleAlert size={18} aria-hidden="true" /><code>{error}</code>
          {retryAvailable && <button type="button" onClick={retryWorker}>{text.retry}</button>}
        </div>
      )}

      <div className="analyzer-plots">
        <SignalPlot title={text.signal} xLabel={text.timeUnit} yLabel={text.inputUnit}
          x={result?.display.input.time} traces={[{ values: result?.display.input.value, color: '#8d9ca3', label: 'INPUT' }]} />
        <SignalPlot title={text.lockin} xLabel={text.timeUnit} yLabel="V"
          x={result?.display.lockin[0]} traces={[
            { values: result?.display.lockin[1], color: '#00c9c2', label: 'X' },
            { values: result?.display.lockin[2], color: '#f04f93', label: 'Y' },
          ]} />
        <SignalPlot title={text.polar} xLabel={text.timeUnit} yLabel="V / rad"
          x={result?.display.lockin[0]} traces={[
            { values: result?.display.lockin[3], color: '#b8f15b', label: '|R|' },
            { values: result?.display.lockin[4], color: '#f2bd4d', label: 'PHASE' },
          ]} />
        <SignalPlot title={text.kerr} xLabel={text.timeUnit} yLabel="rad"
          x={result?.display.lockin[0]} traces={[
            { values: result?.display.lockin[5], color: '#a67cff', label: 'KERR' },
          ]} secondary={{
            x: result?.display.response.frequency,
            values: result?.display.response.magnitude,
            color: '#6b7d85', label: '|H(f)|', xLabel: text.responseUnit,
          }} />
      </div>

      <div className="analyzer-lower">
        <section className="analyzer-controls" aria-labelledby="analysis-control-heading">
          <div className="analyzer-section-title"><span id="analysis-control-heading">{text.controls}</span><small>{text.local}</small></div>
          <div className="filter-contract">
            <span>{text.filter}</span><strong>{text.filterValue}</strong><i><Check size={13} />{text.parity}</i>
          </div>
          <fieldset className="parameter-grid" disabled={busy}>
            <NumberControl label={text.samples} value={parameters.samples} min={64} max={limits?.max_demo_samples ?? 100_000} step={1_000}
              disabled={sourceMode === 'csv'} onChange={(samples) => setParameters((current) => ({ ...current, samples }))} />
            <NumberControl label={text.sampleRate} value={parameters.sampleRateHz} min={1_000} max={10_000_000} step={1_000} suffix="Hz"
              onChange={(sampleRateHz) => setParameters((current) => ({ ...current, sampleRateHz }))} />
            <NumberControl label={text.frequency} value={parameters.referenceFrequencyHz} min={1} max={100_000} step={100} suffix="Hz"
              onChange={(referenceFrequencyHz) => setParameters((current) => ({ ...current, referenceFrequencyHz }))} />
            <NumberControl label={text.amplitude} value={parameters.amplitude} min={0.001} max={10} step={0.1} suffix="V"
              disabled={sourceMode === 'csv'} onChange={(amplitude) => setParameters((current) => ({ ...current, amplitude }))} />
            <NumberControl label={text.phase} value={parameters.signalPhaseRad} min={-3.14} max={3.14} step={0.05} suffix="rad"
              disabled={sourceMode === 'csv'} onChange={(signalPhaseRad) => setParameters((current) => ({ ...current, signalPhaseRad }))} />
            <NumberControl label={text.noise} value={parameters.noiseRms} min={0} max={1} step={0.001} suffix="V"
              disabled={sourceMode === 'csv'} onChange={(noiseRms) => setParameters((current) => ({ ...current, noiseRms }))} />
            <NumberControl label={text.angle} value={parameters.kerrAngleRad} min={-0.1} max={0.1} step={0.001} suffix="rad"
              disabled={sourceMode === 'csv'} onChange={(kerrAngleRad) => setParameters((current) => ({ ...current, kerrAngleRad }))} />
            <NumberControl label={text.window} value={parameters.halfWindowCycles} min={0.25} max={10} step={0.25} suffix="cycle"
              onChange={(halfWindowCycles) => setParameters((current) => ({ ...current, halfWindowCycles }))} />
            <NumberControl label={text.stride} value={parameters.strideSamples} min={1} max={10_000} step={1}
              onChange={(strideSamples) => setParameters((current) => ({ ...current, strideSamples: Math.round(strideSamples) }))} />
            <NumberControl label={text.harmonic} value={parameters.harmonic} min={1} max={6} step={1}
              onChange={(harmonic) => setParameters((current) => ({ ...current, harmonic: Math.round(harmonic) }))} />
            <NumberControl label={text.rotation} value={parameters.rotationRad} min={-3.14} max={3.14} step={0.05} suffix="rad"
              onChange={(rotationRad) => setParameters((current) => ({ ...current, rotationRad }))} />
            <NumberControl label={text.factor} value={parameters.kerrFactor} min={-10} max={10} step={0.1}
              onChange={(kerrFactor) => setParameters((current) => ({ ...current, kerrFactor }))} />
          </fieldset>
          <div className="unsupported-contract"><span>{text.unsupported}</span><strong>{text.unsupportedList}</strong></div>
        </section>

        <section className="analyzer-metrics" aria-labelledby="analysis-result-heading">
          <div className="analyzer-section-title"><span id="analysis-result-heading">{text.result}</span>
            <div><IconButton label={copied ? text.copied : text.copy} onClick={() => void copySummary()} disabled={!result}><Copy size={15} /></IconButton>
              <IconButton label={text.export} onClick={exportCsv} disabled={!result}><Download size={15} /></IconButton></div>
          </div>
          <dl>
            <Metric label={text.samples} value={result ? formatInteger(result.source.samples) : '—'} />
            <Metric label={text.output} value={result ? formatInteger(result.metadata.outputSamples) : '—'} />
            <Metric label={text.inputRate} value={result ? formatRate(result.source.sampleRateHz) : '—'} />
            <Metric label={text.outputRate} value={result ? formatRate(result.metadata.outputRateHz) : '—'} />
            <Metric label={text.elapsed} value={result ? `${result.metadata.elapsedMs.toFixed(1)} ms` : '—'} accent />
            <Metric label={text.enbw} value={result ? formatRate(result.metadata.estimatedEnbwHz) : '—'} />
            <Metric label={text.support} value={result ? formatDuration(result.metadata.supportS) : '—'} />
            <Metric label={text.cutoff} value={text.cutoffValue} />
            <Metric label={text.trim} value={result ? `${formatInteger(result.metadata.firstInputIndex)}…${formatInteger(result.metadata.lastInputIndex)}` : '—'} />
            <Metric label={text.settling} value={text.settlingValue} />
            <Metric label={text.modulation} value={result ? result.metadata.modulationDepth.toFixed(6) : '—'} />
            <Metric label={text.kerrMedian} value={result ? `${median(result.export.kerr).toExponential(4)} rad` : '—'} accent />
          </dl>
          {result?.warnings.length ? <ul className="analyzer-warnings">{result.warnings.map((warning) => <li key={warning}>{warningLabel(locale, warning)}</li>)}</ul> : null}
        </section>
      </div>
    </div>
  );
}

function NumberControl({ label, value, min, max, step, suffix, disabled, onChange }: {
  label: string; value: number; min: number; max: number; step: number; suffix?: string;
  disabled?: boolean; onChange: (value: number) => void;
}) {
  return <label className="number-control"><span>{label}</span><div><input type="number" value={value} min={min} max={max} step={step} disabled={disabled}
    onChange={(event) => { const next = Number(event.target.value); if (Number.isFinite(next)) onChange(next); }} />{suffix && <i>{suffix}</i>}</div></label>;
}

function Metric({ label, value, accent = false }: { label: string; value: string; accent?: boolean }) {
  return <div className={accent ? 'accent' : ''}><dt>{label}</dt><dd>{value}</dd></div>;
}

function IconButton({ label, onClick, disabled, children }: { label: string; onClick: () => void; disabled?: boolean; children: ReactNode }) {
  return <button type="button" className="icon-button" aria-label={label} title={label} onClick={onClick} disabled={disabled}>{children}</button>;
}

type Trace = { values?: Float64Array; color: string; label: string };

function SignalPlot({ title, xLabel, yLabel, x, traces, secondary }: {
  title: string; xLabel: string; yLabel: string; x?: Float64Array; traces: Trace[];
  secondary?: { x?: Float64Array; values?: Float64Array; color: string; label: string; xLabel: string };
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const series = useMemo(() => traces.filter((trace) => trace.values), [traces]);
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const draw = () => drawPlot(canvas, x, series, secondary);
    const resizeObserver = new ResizeObserver(draw);
    const themeObserver = new MutationObserver(draw);
    resizeObserver.observe(canvas);
    themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] });
    draw();
    return () => {
      resizeObserver.disconnect();
      themeObserver.disconnect();
    };
  }, [secondary, series, x]);
  return <figure className="analyzer-plot"><figcaption><strong>{title}</strong><span>{yLabel} / {secondary?.xLabel ?? xLabel}</span></figcaption>
    <canvas ref={canvasRef} role="img" aria-label={title} />
    <div className="plot-legend" aria-hidden="true">{traces.map((trace) => <span key={trace.label}><i style={{ background: trace.color }} />{trace.label}</span>)}
      {secondary && <span><i style={{ background: secondary.color }} />{secondary.label}</span>}</div></figure>;
}

function drawPlot(canvas: HTMLCanvasElement, x: Float64Array | undefined, traces: Trace[], secondary?: { x?: Float64Array; values?: Float64Array; color: string }) {
  const context = canvas.getContext('2d');
  if (!context) return;
  const rect = canvas.getBoundingClientRect();
  const ratio = Math.min(window.devicePixelRatio || 1, 2);
  canvas.width = Math.max(1, Math.round(rect.width * ratio)); canvas.height = Math.max(1, Math.round(rect.height * ratio));
  context.setTransform(ratio, 0, 0, ratio, 0, 0); context.clearRect(0, 0, rect.width, rect.height);
  const dark = document.documentElement.classList.contains('dark');
  const left = 38, right = rect.width - 12, top = 12, bottom = rect.height - 24;
  context.strokeStyle = dark ? 'rgba(145,164,174,.15)' : 'rgba(33,62,64,.14)'; context.lineWidth = 1;
  for (let index = 0; index <= 4; index += 1) { const y = top + (bottom - top) * index / 4; context.beginPath(); context.moveTo(left, y); context.lineTo(right, y); context.stroke(); }
  for (let index = 0; index <= 6; index += 1) { const px = left + (right - left) * index / 6; context.beginPath(); context.moveTo(px, top); context.lineTo(px, bottom); context.stroke(); }
  const available = traces.filter((trace): trace is Trace & { values: Float64Array } => Boolean(trace.values?.length));
  if (!x?.length || !available.length) { context.fillStyle = dark ? '#61727a' : '#839094'; context.font = '11px JetBrains Mono, monospace'; context.fillText('AWAITING DATA', left + 8, top + 20); return; }
  let minY = Infinity, maxY = -Infinity;
  for (const trace of available) for (const value of trace.values) { minY = Math.min(minY, value); maxY = Math.max(maxY, value); }
  if (minY === maxY) { minY -= 1; maxY += 1; }
  for (const trace of available) { context.beginPath(); context.strokeStyle = trace.color; context.lineWidth = 1.5;
    for (let index = 0; index < Math.min(x.length, trace.values.length); index += 1) { const px = left + (right - left) * index / Math.max(1, x.length - 1); const py = bottom - (bottom - top) * (trace.values[index] - minY) / (maxY - minY); if (index === 0) context.moveTo(px, py); else context.lineTo(px, py); } context.stroke(); }
  if (secondary?.x?.length && secondary.values?.length) { const values = secondary.values; let secondaryMax = 0; for (const value of values) secondaryMax = Math.max(secondaryMax, Math.abs(value)); context.beginPath(); context.strokeStyle = secondary.color; context.setLineDash([4, 4]);
    for (let index = 0; index < values.length; index += 1) { const px = left + (right - left) * index / Math.max(1, values.length - 1); const py = bottom - (bottom - top) * values[index] / Math.max(secondaryMax, Number.EPSILON); if (index === 0) context.moveTo(px, py); else context.lineTo(px, py); } context.stroke(); context.setLineDash([]); }
  context.fillStyle = dark ? '#82939b' : '#637579'; context.font = '10px JetBrains Mono, monospace'; context.fillText(maxY.toExponential(1), 2, top + 4); context.fillText(minY.toExponential(1), 2, bottom);
}

function summaryText(result: AnalysisResult) { return [`pmoke waveform analysis`, `source=${singleLine(result.source.name)}`, `samples=${result.source.samples}`, `sample_rate_hz=${result.source.sampleRateHz}`, `algorithm=${result.metadata.algorithm}`, `parity=${result.metadata.parity}`, `runtime_ms=${result.metadata.elapsedMs.toFixed(3)}`, `output_rate_hz=${result.metadata.outputRateHz}`, `estimated_enbw_hz=${result.metadata.estimatedEnbwHz}`, `support_s=${result.metadata.supportS}`, `first_input_index=${result.metadata.firstInputIndex}`, `last_input_index=${result.metadata.lastInputIndex}`, `warnings=${result.warnings.join('|') || 'none'}`, `kerr_median_rad=${median(result.export.kerr)}`].join('\n'); }
function singleLine(value: string) { return value.replace(/[\r\n\t]+/gu, ' ').trim(); }
function median(values: Float64Array) { if (!values.length) return Number.NaN; const copy = Array.from(values).sort((a, b) => a - b); const middle = Math.floor(copy.length / 2); return copy.length % 2 ? copy[middle] : 0.5 * (copy[middle - 1] + copy[middle]); }
function formatInteger(value: number) { return new Intl.NumberFormat('en-US').format(value); }
function formatRate(value: number) { return value >= 1e6 ? `${(value / 1e6).toFixed(3)} MHz` : value >= 1e3 ? `${(value / 1e3).toFixed(3)} kHz` : `${value.toFixed(2)} Hz`; }
function formatDuration(value: number) { return value < 1e-3 ? `${(value * 1e6).toFixed(2)} µs` : `${(value * 1e3).toFixed(2)} ms`; }
function progressLabel(locale: Locale, stage: string) { const text = COPY[locale]; if (stage.startsWith('lockin:')) return `${text.stageLockin} ${stage.slice(7)}`; return ({ prepare: text.stagePrepare, kerr: text.stageKerr, decimate: text.stageDecimate, complete: text.stageComplete } as Record<string, string>)[stage] ?? stage; }
function warningLabel(locale: Locale, warning: string) { const text = COPY[locale]; return ({ long_window: text.warningLong, sparse_output: text.warningSparse, local_input: text.warningLocal } as Record<string, string>)[warning] ?? warning; }
