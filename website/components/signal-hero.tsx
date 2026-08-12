'use client';

import { useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { basePath } from '@/lib/shared';

const STAGE_DURATION_MS = 4_600;
const STAGE_MOTION_MS = 3_600;
const TIME_START_MS = -10;
const TIME_END_MS = 60;
const PULSE_PEAK_MS = 15.8;
const PULSE_END_MS = 42;
const FIELD_PEAK_T = 0.82;
const LI_X_PEAK_MV = -3.2;
const LI_Y_PEAK_MV = 5.4;
const KERR_PEAK_MRAD = -9.8;

export type SignalSequenceStage = 'field-pulse' | 'lock-in' | 'phase-correction' | 'kerr-angle';

export type SignalHeroLabels = {
  label: string;
  description: string;
  pipeline: string;
  sequence: string;
  fieldPulse: string;
  lockIn: string;
  phaseCorrection: string;
  kerrAngle: string;
  fieldSummary: string;
  lockInSummary: string;
  phaseSummary: string;
  kerrSummary: string;
  currentTime: string;
  timeAxis: string;
  fieldAxis: string;
  lockInAxis: string;
  kerrAxis: string;
  liX: string;
  liY: string;
  rawVector: string;
  correctedVector: string;
  phaseShift: string;
  quadratureZero: string;
  kerrResult: string;
  replayStage: string;
};

type MotionState = 'paused' | 'running' | 'complete' | 'reduced';
type PlaybackMode = 'sequence' | 'stage';
type SignalStatus = 'loading' | 'ready' | 'fallback';

type StageCopy = {
  title: string;
  summary: string;
};

type RenderState = {
  stageIndex: number;
  elapsedMs: number;
  motion: MotionState;
  mode: PlaybackMode;
};

type PlaybackControls = {
  replayStage: (index: number) => void;
};

const STAGES: SignalSequenceStage[] = [
  'field-pulse',
  'lock-in',
  'phase-correction',
  'kerr-angle',
];

const NOOP_PLAYBACK: PlaybackControls = {
  replayStage: () => undefined,
};

export function fieldPulseAtMs(timeMs: number): number {
  if (!Number.isFinite(timeMs) || timeMs <= 0 || timeMs >= PULSE_END_MS) return 0;
  if (timeMs <= PULSE_PEAK_MS) {
    const rise = Math.sin((Math.PI * timeMs) / (2 * PULSE_PEAK_MS));
    return FIELD_PEAK_T * rise ** 1.12;
  }
  const decayProgress = (timeMs - PULSE_PEAK_MS) / (PULSE_END_MS - PULSE_PEAK_MS);
  const decay = Math.cos((Math.PI * decayProgress) / 2);
  return FIELD_PEAK_T * Math.max(0, decay) ** 1.48;
}

export function lockInAtMs(timeMs: number): readonly [number, number] {
  const envelope = fieldPulseAtMs(timeMs) / FIELD_PEAK_T;
  return [LI_X_PEAK_MV * envelope, LI_Y_PEAK_MV * envelope];
}

export function kerrAngleAtMs(timeMs: number): number {
  const envelope = fieldPulseAtMs(timeMs) / FIELD_PEAK_T;
  return KERR_PEAK_MRAD * envelope;
}

/** Apply the same coordinate transform as pmoke-analysis-core::rotate_phase. */
export function rotatePhasePoint(x: number, y: number, deltaRad: number): [number, number] {
  const cosDelta = Math.cos(deltaRad);
  const sinDelta = Math.sin(deltaRad);
  return [x * cosDelta + y * sinDelta, -x * sinDelta + y * cosDelta];
}

export function sequenceStageForElapsed(elapsedMs: number): SignalSequenceStage {
  const totalDuration = STAGES.length * STAGE_DURATION_MS;
  const elapsed = clamp(elapsedMs, 0, totalDuration - 1);
  return STAGES[Math.floor(elapsed / STAGE_DURATION_MS)];
}

export function SignalHero({ labels }: { labels: SignalHeroLabels }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const controlsRef = useRef<PlaybackControls>(NOOP_PLAYBACK);
  const [status, setStatus] = useState<SignalStatus>('loading');
  const [renderState, setRenderState] = useState<RenderState>({
    stageIndex: 0,
    elapsedMs: 0,
    motion: 'paused',
    mode: 'sequence',
  });

  useEffect(() => {
    let terminated = false;
    const worker = new Worker(`${basePath}/workers/signal.worker.js`, { type: 'module' });

    worker.onmessage = (event: MessageEvent<{ type: string }>) => {
      if (terminated) return;
      setStatus(event.data.type === 'ready' ? 'ready' : 'fallback');
    };
    worker.onerror = () => {
      if (!terminated) setStatus('fallback');
    };
    worker.postMessage({ type: 'init', basePath, samples: 720 });

    return () => {
      terminated = true;
      worker.terminate();
    };
  }, []);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)');
    const playback = {
      stageIndex: 0,
      elapsedMs: 0,
      mode: 'sequence' as PlaybackMode,
      complete: false,
    };
    let animationFrame = 0;
    let lastTimestamp: number | null = null;
    let inViewport = false;
    let autoplayEligible = false;
    let userInitiated = false;
    let documentVisible = document.visibilityState === 'visible';
    let reduced = reducedMotion.matches;

    const publish = (motion: MotionState) => {
      setRenderState({
        stageIndex: playback.stageIndex,
        elapsedMs: reduced ? STAGE_MOTION_MS : playback.elapsedMs,
        motion,
        mode: playback.mode,
      });
    };

    const canRun = () =>
      inViewport &&
      (autoplayEligible || userInitiated) &&
      documentVisible &&
      !reduced &&
      !playback.complete;

    const finishStage = () => {
      if (playback.mode === 'sequence' && playback.stageIndex < STAGES.length - 1) {
        playback.stageIndex += 1;
        playback.elapsedMs = 0;
        return;
      }
      playback.elapsedMs = STAGE_DURATION_MS;
      playback.complete = true;
    };

    const frame = (timestamp: number) => {
      if (lastTimestamp !== null) {
        playback.elapsedMs += Math.min(timestamp - lastTimestamp, 64);
        if (playback.elapsedMs >= STAGE_DURATION_MS) finishStage();
      }
      lastTimestamp = timestamp;
      publish(playback.complete ? 'complete' : 'running');

      if (canRun()) {
        animationFrame = window.requestAnimationFrame(frame);
      } else {
        animationFrame = 0;
        lastTimestamp = null;
        publish(playback.complete ? 'complete' : 'paused');
      }
    };

    const sync = () => {
      if (reduced) {
        if (animationFrame) window.cancelAnimationFrame(animationFrame);
        animationFrame = 0;
        lastTimestamp = null;
        publish('reduced');
        return;
      }

      if (canRun()) {
        publish('running');
        if (!animationFrame) animationFrame = window.requestAnimationFrame(frame);
        return;
      }

      if (animationFrame) window.cancelAnimationFrame(animationFrame);
      animationFrame = 0;
      lastTimestamp = null;
      publish(playback.complete ? 'complete' : 'paused');
    };

    controlsRef.current = {
      replayStage: (index: number) => {
        playback.stageIndex = clamp(Math.round(index), 0, STAGES.length - 1);
        playback.elapsedMs = reduced ? STAGE_MOTION_MS : 0;
        playback.mode = 'stage';
        playback.complete = reduced;
        userInitiated = true;
        lastTimestamp = null;
        publish(reduced ? 'reduced' : 'paused');
        sync();
      },
    };

    const bounds = container.getBoundingClientRect();
    const visibleHeight = Math.max(0, Math.min(bounds.bottom, window.innerHeight) - Math.max(bounds.top, 0));
    const visibleRatio = bounds.height > 0 ? visibleHeight / bounds.height : 0;
    inViewport = visibleRatio > 0;
    autoplayEligible = visibleRatio >= 0.7;

    const intersectionObserver = new IntersectionObserver(([entry]) => {
      inViewport = entry.isIntersecting;
      autoplayEligible = entry.isIntersecting && entry.intersectionRatio >= 0.7;
      if (!entry.isIntersecting) userInitiated = false;
      sync();
    }, { threshold: [0, 0.7] });
    intersectionObserver.observe(container);

    const handleVisibility = () => {
      documentVisible = document.visibilityState === 'visible';
      sync();
    };
    const handleReducedMotion = (event: MediaQueryListEvent) => {
      reduced = event.matches;
      sync();
    };

    document.addEventListener('visibilitychange', handleVisibility);
    reducedMotion.addEventListener('change', handleReducedMotion);
    sync();

    return () => {
      if (animationFrame) window.cancelAnimationFrame(animationFrame);
      controlsRef.current = NOOP_PLAYBACK;
      intersectionObserver.disconnect();
      document.removeEventListener('visibilitychange', handleVisibility);
      reducedMotion.removeEventListener('change', handleReducedMotion);
    };
  }, []);

  const stage = STAGES[renderState.stageIndex];
  const rawProgress = clamp(renderState.elapsedMs / STAGE_MOTION_MS, 0, 1);
  const stageProgress = easeInOut(rawProgress);
  const pipelineProgress = clamp(
    ((renderState.stageIndex + rawProgress) / (STAGES.length - 1)) * 100,
    0,
    100,
  );
  const stageCopy: Record<SignalSequenceStage, StageCopy> = {
    'field-pulse': { title: labels.fieldPulse, summary: labels.fieldSummary },
    'lock-in': { title: labels.lockIn, summary: labels.lockInSummary },
    'phase-correction': { title: labels.phaseCorrection, summary: labels.phaseSummary },
    'kerr-angle': { title: labels.kerrAngle, summary: labels.kerrSummary },
  };
  return (
    <div
      ref={containerRef}
      className="signal-stage"
      data-wasm={status}
      data-motion={renderState.motion}
      data-playback-mode={renderState.mode}
      data-sequence-stage={stage}
    >
      <div className="signal-stage-content">
        <article className="signal-process-card" aria-label={labels.label} aria-describedby="signal-description">
          <header className="signal-process-header">
            <div className="signal-process-topline">
              <p className="signal-process-kicker">{labels.pipeline}</p>
            </div>
            <div className="signal-process-flow">
              <div className="signal-process-track" aria-hidden="true">
                <span style={{ width: `${pipelineProgress}%` }} />
              </div>
              <ol className="signal-process-rail" aria-label={labels.sequence}>
                {STAGES.map((step, index) => (
                  <li
                    key={step}
                    data-step={step}
                    data-current={String(step === stage)}
                    data-complete={String(index < renderState.stageIndex || (index === renderState.stageIndex && rawProgress >= 1))}
                  >
                    <button
                      type="button"
                      onClick={() => controlsRef.current.replayStage(index)}
                      aria-current={step === stage ? 'step' : undefined}
                      aria-label={`${labels.replayStage}: ${stageCopy[step].title}`}
                    >
                      <span className="signal-step-index">{String(index + 1).padStart(2, '0')}</span>
                      <span className="signal-step-name">{stageCopy[step].title}</span>
                    </button>
                  </li>
                ))}
              </ol>
            </div>
          </header>

          <div className="signal-process-body">
            <div className="signal-stage-heading">
              <div>
                <span>{String(renderState.stageIndex + 1).padStart(2, '0')} / 04</span>
                <h2>{stageCopy[stage].title}</h2>
              </div>
            </div>
            <p className="signal-stage-summary">{stageCopy[stage].summary}</p>

            <div className="signal-visualization" data-signal-region="visualization">
              <FieldPanel labels={labels} active={stage === 'field-pulse'} progress={stageProgress} />
              <LockInPanel labels={labels} active={stage === 'lock-in'} progress={stageProgress} />
              <PhasePanel labels={labels} active={stage === 'phase-correction'} progress={stageProgress} />
              <KerrPanel labels={labels} active={stage === 'kerr-angle'} progress={stageProgress} />
            </div>
          </div>

          <p id="signal-description" className="signal-description">{labels.description}</p>
        </article>
      </div>
    </div>
  );
}

function FieldPanel({ labels, active, progress }: PanelProps) {
  const path = useMemo(() => makeTimePath(fieldPulseAtMs, 0, 0.9), []);
  const area = useMemo(() => `${path}L100 90L0 90Z`, [path]);
  const currentTime = lerp(0, TIME_END_MS, progress);
  const revealWidth = timeRevealPercent(progress);
  return (
    <section className="signal-panel" data-active={String(active)} aria-hidden={!active}>
      <TimeChart
        title={labels.fieldAxis}
        accessibleTitle={labels.fieldAxis}
        labels={labels}
        yTicks={['0.8', '0.4', '0']}
      >
        <defs>
          <linearGradient id="signal-field-fill" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="var(--cyan)" stopOpacity="0.24" />
            <stop offset="1" stopColor="var(--cyan)" stopOpacity="0" />
          </linearGradient>
          <clipPath id="signal-field-reveal"><rect x="0" y="0" width={revealWidth} height="100" /></clipPath>
        </defs>
        <path className="signal-trace-area" clipPath="url(#signal-field-reveal)" d={area} fill="url(#signal-field-fill)" />
        <path className="signal-trace signal-trace-field" clipPath="url(#signal-field-reveal)" d={path} />
      </TimeChart>
      <div className="signal-panel-readout">
        <span>{labels.currentTime} · {currentTime.toFixed(1)} ms</span>
        <strong>μ₀H<sub>peak</sub> ≈ {FIELD_PEAK_T.toFixed(2)} T</strong>
      </div>
    </section>
  );
}

function LockInPanel({ labels, active, progress }: PanelProps) {
  const fieldPath = useMemo(() => makeTimePath(fieldPulseAtMs, 0, 0.9), []);
  const xPath = useMemo(() => makeTimePath((time) => lockInAtMs(time)[0], -6, 6), []);
  const yPath = useMemo(() => makeTimePath((time) => lockInAtMs(time)[1], -6, 6), []);
  const currentTime = lerp(0, TIME_END_MS, progress);
  const revealWidth = timeRevealPercent(progress);
  const [xValue, yValue] = lockInAtMs(currentTime);
  return (
    <section className="signal-panel" data-active={String(active)} aria-hidden={!active}>
      <div className="signal-chart-legend" aria-hidden="true">
        <span className="is-field">μ₀H</span>
        <span className="is-x">{labels.liX}</span>
        <span className="is-y">{labels.liY}</span>
      </div>
      <TimeChart
        title={labels.lockInAxis}
        accessibleTitle={labels.lockInAxis}
        labels={labels}
        yTicks={['+6', '0', '−6']}
      >
        <defs>
          <clipPath id="signal-lockin-reveal"><rect x="0" y="0" width={revealWidth} height="100" /></clipPath>
        </defs>
        <path className="signal-trace signal-trace-context" d={fieldPath} />
        <path className="signal-trace signal-trace-x" clipPath="url(#signal-lockin-reveal)" d={xPath} />
        <path className="signal-trace signal-trace-y" clipPath="url(#signal-lockin-reveal)" d={yPath} />
      </TimeChart>
      <div className="signal-panel-readout signal-panel-readout-split">
        <span>{currentTime.toFixed(1)} ms</span>
        <strong className="is-x">X {formatSigned(xValue)} mV</strong>
        <strong className="is-y">Y {formatSigned(yValue)} mV</strong>
      </div>
    </section>
  );
}

function PhasePanel({ labels, active, progress }: PanelProps) {
  const rawX = LI_X_PEAK_MV;
  const rawY = LI_Y_PEAK_MV;
  const rawAngle = Math.atan2(rawY, rawX);
  const correctedAngle = rawAngle * (1 - progress);
  const radius = 37;
  const rawPoint = pointOnCircle(rawAngle, radius);
  const correctedPoint = pointOnCircle(correctedAngle, radius);
  const magnitude = Math.hypot(rawX, rawY);
  const appliedPhase = rawAngle * progress;
  const correctedY = magnitude * Math.sin(correctedAngle);
  const rotationArc = phaseArcPath(rawAngle, correctedAngle, 29);

  return (
    <section className="signal-panel signal-panel-phase" data-active={String(active)} aria-hidden={!active}>
      <div className="signal-phase-layout">
        <div className="signal-phase-graphic">
          <svg viewBox="0 0 100 100" role="img" aria-label={labels.phaseSummary}>
            <circle className="signal-phase-field" cx="50" cy="50" r="43" />
            <circle className="signal-phase-ring" cx="50" cy="50" r={radius} />
            <line className="signal-phase-axis" x1="8" y1="50" x2="92" y2="50" />
            <line className="signal-phase-axis" x1="50" y1="8" x2="50" y2="92" />
            <text className="signal-phase-svg-label" x="54" y="6" aria-hidden="true">Y′</text>
            <text className="signal-phase-svg-label" x="94" y="46" aria-hidden="true">X′</text>
            <path className="signal-phase-rotation-arc" d={rotationArc} />
            <line className="signal-phase-raw" x1="50" y1="50" x2={rawPoint.x} y2={rawPoint.y} />
            <circle className="signal-phase-raw-halo" cx={rawPoint.x} cy={rawPoint.y} r="2.7" />
            <circle className="signal-phase-raw-point" cx={rawPoint.x} cy={rawPoint.y} r="1.05" />
            <line className="signal-phase-corrected" x1="50" y1="50" x2={correctedPoint.x} y2={correctedPoint.y} />
            <circle className="signal-phase-corrected-halo" cx={correctedPoint.x} cy={correctedPoint.y} r="3" />
            <circle className="signal-phase-corrected-point" cx={correctedPoint.x} cy={correctedPoint.y} r="1.15" />
            <circle className="signal-phase-origin" cx="50" cy="50" r="0.9" />
          </svg>
          <span className="signal-phase-operator" aria-hidden="true">R(−φ)</span>
        </div>
        <dl className="signal-phase-metrics">
          <div>
            <dt>{labels.rawVector}</dt>
            <dd>X {LI_X_PEAK_MV.toFixed(1)} / Y +{LI_Y_PEAK_MV.toFixed(1)} mV</dd>
          </div>
          <div>
            <dt>{labels.phaseShift}</dt>
            <dd>{formatPhaseDegrees(appliedPhase)}</dd>
          </div>
          <div className="is-accent">
            <dt>{labels.quadratureZero}</dt>
            <dd>LI Y′ {formatSigned(correctedY)} mV</dd>
          </div>
        </dl>
      </div>
      <div className="signal-vector-legend" aria-hidden="true">
        <span className="is-raw">{labels.rawVector}</span>
        <span className="is-corrected">{labels.correctedVector}</span>
      </div>
    </section>
  );
}

function KerrPanel({ labels, active, progress }: PanelProps) {
  const path = useMemo(() => makeTimePath(kerrAngleAtMs, -10, 0), []);
  const area = useMemo(() => `${path}L100 10L0 10Z`, [path]);
  const revealWidth = timeRevealPercent(progress);
  return (
    <section className="signal-panel" data-active={String(active)} aria-hidden={!active}>
      <TimeChart
        title={<>θ<sub>K</sub> (mrad)</>}
        accessibleTitle={labels.kerrAxis}
        labels={labels}
        yTicks={['0', '−5', '−10']}
      >
        <defs>
          <linearGradient id="signal-kerr-fill" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="var(--green)" stopOpacity="0.03" />
            <stop offset="1" stopColor="var(--green)" stopOpacity="0.22" />
          </linearGradient>
          <clipPath id="signal-kerr-reveal"><rect x="0" y="0" width={revealWidth} height="100" /></clipPath>
        </defs>
        <path className="signal-trace-area" clipPath="url(#signal-kerr-reveal)" d={area} fill="url(#signal-kerr-fill)" />
        <path className="signal-trace signal-trace-kerr" clipPath="url(#signal-kerr-reveal)" d={path} />
      </TimeChart>
      <div className="signal-panel-readout">
        <span>{labels.kerrResult}</span>
        <strong>θ<sub>K, peak</sub> ≈ {KERR_PEAK_MRAD.toFixed(1)} mrad</strong>
      </div>
    </section>
  );
}

function TimeChart({ title, accessibleTitle, labels, yTicks, children }: TimeChartProps) {
  return (
    <div className="signal-time-chart">
      <div className="signal-y-axis" aria-hidden="true">
        <strong><span>{title}</span></strong>
        <div>{yTicks.map((tick) => <span key={tick}>{tick}</span>)}</div>
      </div>
      <div className="signal-plot-column">
        <div className="signal-plot-area">
          <svg viewBox="0 0 100 100" preserveAspectRatio="none" role="img" aria-label={accessibleTitle}>
            <line className="signal-grid-line" x1="0" y1="10" x2="100" y2="10" />
            <line className="signal-grid-line" x1="0" y1="50" x2="100" y2="50" />
            <line className="signal-grid-line" x1="0" y1="90" x2="100" y2="90" />
            {children}
          </svg>
        </div>
        <div className="signal-time-axis" aria-hidden="true">
          <div>
            {[-10, 0, 10, 20, 30, 40, 50, 60].map((tick) => (
              <span key={tick} className={[10, 30, 50].includes(tick) ? 'is-optional' : ''}>{tick}</span>
            ))}
          </div>
          <strong>{labels.timeAxis}</strong>
        </div>
      </div>
    </div>
  );
}

type PanelProps = {
  labels: SignalHeroLabels;
  active: boolean;
  progress: number;
};

type TimeChartProps = {
  title: ReactNode;
  accessibleTitle: string;
  labels: SignalHeroLabels;
  yTicks: string[];
  children: ReactNode;
};

type SignalPoint = { x: number; y: number };

function makeTimePath(sample: (timeMs: number) => number, min: number, max: number): string {
  const points = 281;
  let path = '';
  for (let index = 0; index < points; index += 1) {
    const position = index / (points - 1);
    const timeMs = lerp(TIME_START_MS, TIME_END_MS, position);
    const value = clamp(sample(timeMs), min, max);
    const x = position * 100;
    const y = 90 - ((value - min) / (max - min)) * 80;
    path += `${index === 0 ? 'M' : 'L'}${x.toFixed(3)} ${y.toFixed(3)}`;
  }
  return path;
}

function pointOnCircle(angleRad: number, radius: number): SignalPoint {
  return {
    x: 50 + radius * Math.cos(angleRad),
    y: 50 - radius * Math.sin(angleRad),
  };
}

function phaseArcPath(startAngle: number, endAngle: number, radius: number): string {
  const start = pointOnCircle(startAngle, radius);
  const end = pointOnCircle(endAngle, radius);
  const largeArc = Math.abs(startAngle - endAngle) > Math.PI ? 1 : 0;
  return `M${start.x.toFixed(3)} ${start.y.toFixed(3)}A${radius} ${radius} 0 ${largeArc} 1 ${end.x.toFixed(3)} ${end.y.toFixed(3)}`;
}

function timeRevealPercent(progress: number): number {
  const timeMs = lerp(0, TIME_END_MS, clamp(progress, 0, 1));
  return ((timeMs - TIME_START_MS) / (TIME_END_MS - TIME_START_MS)) * 100;
}

function formatSigned(value: number): string {
  if (Math.abs(value) < 0.05) return '0.0';
  return `${value > 0 ? '+' : '−'}${Math.abs(value).toFixed(1)}`;
}

function formatPhaseDegrees(angleRad: number): string {
  const degrees = Math.abs(angleRad * 180 / Math.PI);
  return degrees < 0.05 ? '0.0°' : `−${degrees.toFixed(1)}°`;
}

function easeInOut(value: number): number {
  const normalized = clamp(value, 0, 1);
  return normalized * normalized * (3 - 2 * normalized);
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function lerp(from: number, to: number, amount: number): number {
  return from + (to - from) * amount;
}
