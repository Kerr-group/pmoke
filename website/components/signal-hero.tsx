'use client';

import { useEffect, useRef, useState } from 'react';
import { basePath } from '@/lib/shared';

const TAU = Math.PI * 2;
const LOOP_DURATION_MS = 24_000;
const LOOP_HOLD_START = 0.9;
const LOOP_RETURN_START = 0.96;
const PREVIEW_SAMPLES = 720;
const INITIAL_PHASE = 0.17;
const MIN_RENDER_POINTS = 256;

export type SignalSequenceStage = 'field-pulse' | 'waveforms' | 'lock-in' | 'rotate-phase' | 'kerr-angle';

export type SignalHeroLabels = {
  label: string;
  description: string;
  pipeline: string;
  sequence: string;
  fieldPulse: string;
  triggeredWindow: string;
  referenceResponse: string;
  lockIn: string;
  rotatePhase: string;
  kerrAngle: string;
  timeDomain: string;
  phaseSpace: string;
  harmonicExtraction: string;
  perHarmonic: string;
  pause: string;
  resume: string;
  reducedMotion: string;
  staticFallback: string;
  wasmLoading: string;
  wasmReady: string;
  wasmFallback: string;
};

type SignalStatus = 'loading' | 'ready' | 'fallback';
type MotionState = 'running' | 'paused' | 'reduced';
type Palette = typeof DARK_COLORS;

type SignalRect = {
  left: number;
  top: number;
  right: number;
  bottom: number;
  width: number;
  height: number;
};

type SignalLayoutMode = 'phone' | 'compact' | 'wide';

type SignalLayout = {
  safe: SignalRect;
  plot: SignalRect;
  phase: SignalRect;
  output: SignalRect;
  mode: SignalLayoutMode;
  stacked: boolean;
};

type Rect = SignalRect;

export type HarmonicKerrCue = {
  a2: number;
  a3: number;
  a4: number;
  a6: number;
  modulationDepth: number;
  angleRad: number;
};

const DARK_COLORS = {
  reference: '#e7edf0',
  cyan: '#16d9d1',
  magenta: '#ed4f9a',
  green: '#74e1a4',
  amber: '#f4bd6b',
  grid: 'rgba(145, 164, 174, 0.34)',
  panel: 'rgba(16, 21, 23, 0.72)',
  panelEdge: 'rgba(145, 164, 174, 0.42)',
  muted: '#8fa09f',
};

const LIGHT_COLORS = {
  reference: '#44575a',
  cyan: '#087b7b',
  magenta: '#bd286f',
  green: '#137946',
  amber: '#a05d00',
  grid: 'rgba(40, 67, 68, 0.36)',
  panel: 'rgba(238, 243, 241, 0.78)',
  panelEdge: 'rgba(68, 87, 90, 0.48)',
  muted: '#637575',
};

// A normalized public illustration. It follows the same dependency graph as
// calculate_harmonics_kerr without embedding a private measurement scale.
const HARMONIC_CUE_INPUT = {
  a2: 0.74,
  a3: 0.22,
  a4: 0.27,
  a6: 0.08,
  factor: 1,
};

export function sampleChannels(t: number, phase: number): [number, number, number] {
  const carrier = TAU * (7.0 * t + phase);
  const envelope = Math.cos(Math.PI * (t - 0.5)) ** 2;
  const pulse = (Math.sin(carrier) + 0.18 * Math.sin(3.0 * carrier + 0.4)) * envelope;
  const inPhase = Math.sin(TAU * 2.0 * t) * 0.72 + 0.12 * pulse;
  const quadrature = Math.sin(TAU * 2.0 * t + 1.12) * 0.48;
  return [pulse, inPhase, quadrature];
}

export function fallbackSignal(samples: number, phase = INITIAL_PHASE): Float64Array {
  const output = new Float64Array(samples * 4);
  for (let i = 0; i < samples; i += 1) {
    const t = i / (samples - 1);
    const [pulse, inPhase, quadrature] = sampleChannels(t, phase);
    output[i * 4] = t;
    output[i * 4 + 1] = pulse;
    output[i * 4 + 2] = inPhase;
    output[i * 4 + 3] = quadrature;
  }
  return output;
}

export function previewPointCount(width: number, samples: number): number {
  const periodicSamples = Math.max(2, samples - 1);
  return Math.min(periodicSamples, Math.max(MIN_RENDER_POINTS, Math.ceil(width) + 1));
}

/** Apply the same coordinate transform as pmoke-analysis-core::rotate_phase. */
export function rotatePhasePoint(x: number, y: number, deltaRad: number): [number, number] {
  const cosDelta = Math.cos(deltaRad);
  const sinDelta = Math.sin(deltaRad);
  return [x * cosDelta + y * sinDelta, -x * sinDelta + y * cosDelta];
}

/** Keep the homepage's harmonic cue tied to the shared Kerr dependency graph. */
export function calculateHarmonicKerrCue(
  a2: number,
  a3: number,
  a4: number,
  a6: number,
  factor = 1,
): HarmonicKerrCue {
  const modulationDenominator = 15 * a2 + 24 * a4 + 9 * a6;
  const radicand = (20 * a4) / modulationDenominator;
  const modulationDepth = 6 * Math.sqrt(radicand);
  const angleDenominator = ((a2 + a4) * modulationDepth) / 6;
  const angleRad = 0.5 * Math.atan(a3 / angleDenominator) * factor;
  if (![modulationDepth, angleRad].every(Number.isFinite)) {
    return { a2, a3, a4, a6, modulationDepth: 0, angleRad: 0 };
  }
  return { a2, a3, a4, a6, modulationDepth, angleRad };
}

export function finitePulseEnvelope(localTime: number): number {
  // The public illustration uses a normalized unipolar field pulse: a fast
  // positive excursion, an asymmetric return to baseline, and no negative
  // undershoot. No private capture or instrument scale is embedded here.
  const onset = smoothStep(localTime / 0.1);
  const settle = 1 - smoothStep((localTime - 0.78) / 0.22);
  const unipolarLobe = Math.exp(-0.5 * ((localTime - 0.28) / 0.16) ** 2);
  return onset * settle * unipolarLobe;
}

const FALLBACK_SIGNAL = fallbackSignal(PREVIEW_SAMPLES);
const HARMONIC_KERR_CUE = calculateHarmonicKerrCue(
  HARMONIC_CUE_INPUT.a2,
  HARMONIC_CUE_INPUT.a3,
  HARMONIC_CUE_INPUT.a4,
  HARMONIC_CUE_INPUT.a6,
  HARMONIC_CUE_INPUT.factor,
);

export function sequenceStageForProgress(progress: number): SignalSequenceStage {
  if (progress < 0.18) return 'field-pulse';
  if (progress < 0.4) return 'waveforms';
  if (progress < 0.6) return 'lock-in';
  if (progress < 0.8) return 'rotate-phase';
  return 'kerr-angle';
}

/** Complete the result, then sweep the same composition back to its opening state. */
export function sequenceProgressForElapsed(elapsedMs: number): number {
  const loopProgress = normalize(elapsedMs / LOOP_DURATION_MS);
  if (loopProgress < LOOP_HOLD_START) return loopProgress / LOOP_HOLD_START;
  if (loopProgress < LOOP_RETURN_START) return 1;
  return 1 - smoothStep((loopProgress - LOOP_RETURN_START) / (1 - LOOP_RETURN_START));
}

export function SignalHero({ labels }: { labels: SignalHeroLabels }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const visualizationRef = useRef<HTMLDivElement>(null);
  const dataRef = useRef<Float64Array>(FALLBACK_SIGNAL);
  const statusRef = useRef<SignalStatus>('loading');
  const userPausedRef = useRef(false);
  const drawLatestRef = useRef<() => void>(() => undefined);
  const syncLifecycleRef = useRef<() => void>(() => undefined);
  const [status, setStatus] = useState<SignalStatus>('loading');
  const [userPaused, setUserPaused] = useState(false);
  const [reducedMotion, setReducedMotion] = useState(false);
  const [motionState, setMotionState] = useState<MotionState>('paused');

  useEffect(() => {
    let terminated = false;
    const worker = new Worker(`${basePath}/workers/signal.worker.js`, { type: 'module' });
    const updateStatus = (nextStatus: SignalStatus) => {
      statusRef.current = nextStatus;
      setStatus(nextStatus);
      syncLifecycleRef.current();
    };

    worker.onmessage = (event: MessageEvent<{ type: string; data?: ArrayBuffer }>) => {
      if (terminated) return;
      if (event.data.type === 'ready' && event.data.data) {
        dataRef.current = new Float64Array(event.data.data);
        drawLatestRef.current();
        updateStatus('ready');
      } else if (event.data.type === 'error') {
        updateStatus('fallback');
      }
    };
    worker.onerror = () => {
      if (!terminated) updateStatus('fallback');
    };
    worker.postMessage({ type: 'init', basePath, samples: PREVIEW_SAMPLES });
    return () => {
      terminated = true;
      worker.terminate();
    };
  }, []);

  useEffect(() => {
    const container = containerRef.current;
    const visualization = visualizationRef.current;
    const canvas = canvasRef.current;
    if (!canvas || !container || !visualization) return;
    const context = canvas.getContext('2d');
    if (!context) return;

    let isIntersecting = false;
    let isDocumentVisible = document.visibilityState === 'visible';
    let isReducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    let cachedWidth = 0;
    let cachedHeight = 0;

    let animationFrameId = 0;
    let accumulatedTime = 0;
    let lastTimestamp: number | null = null;
    let renderedFrame = 0;

    const stageLabels: Record<SignalSequenceStage, string> = {
      'field-pulse': labels.fieldPulse,
      waveforms: labels.referenceResponse,
      'lock-in': labels.lockIn,
      'rotate-phase': labels.rotatePhase,
      'kerr-angle': labels.kerrAngle,
    };
    const sequenceItems = Array.from(
      container.querySelectorAll<HTMLElement>('.signal-sequence [data-step]'),
    );
    const currentStageLabel = container.querySelector<HTMLElement>('.signal-current-stage');

    const syncSequenceStage = (nextStage: SignalSequenceStage) => {
      container.dataset.sequenceStage = nextStage;
      for (const item of sequenceItems) {
        const isCurrent = item.dataset.step === nextStage;
        item.dataset.current = isCurrent ? 'true' : 'false';
        if (isCurrent) {
          item.setAttribute('aria-current', 'step');
        } else {
          item.removeAttribute('aria-current');
        }
      }
      if (currentStageLabel && currentStageLabel.textContent !== stageLabels[nextStage]) {
        currentStageLabel.textContent = stageLabels[nextStage];
      }
    };

    const getTargetDpr = () => Math.min(window.devicePixelRatio || 1, 2);
    let currentDpr = getTargetDpr();
    let layoutMetadataKey = '';

    const updateLayoutMetadata = () => {
      const nextKey = `${cachedWidth}:${cachedHeight}:${window.innerWidth}`;
      if (nextKey === layoutMetadataKey) return;
      layoutMetadataKey = nextKey;
      const layout = getSignalLayout(cachedWidth, cachedHeight, window.innerWidth);
      canvas.dataset.signalLayoutMode = layout.mode;
      canvas.dataset.signalLayoutStacked = String(layout.stacked);
      canvas.dataset.signalLayoutRects = JSON.stringify({
        safe: layout.safe,
        plot: layout.plot,
        phase: layout.phase,
        output: layout.output,
      });
    };

    const updateCanvasBackingStore = () => {
      const targetWidth = Math.max(1, Math.round(cachedWidth * currentDpr));
      const targetHeight = Math.max(1, Math.round(cachedHeight * currentDpr));
      if (canvas.width !== targetWidth || canvas.height !== targetHeight) {
        canvas.width = targetWidth;
        canvas.height = targetHeight;
      }
    };

    const canAnimate = () =>
      isIntersecting &&
      isDocumentVisible &&
      !isReducedMotion &&
      !userPausedRef.current &&
      statusRef.current === 'ready';

    const drawFrame = (sweepOffset: number, complete = false) => {
      if (cachedWidth === 0 || cachedHeight === 0) return;
      updateCanvasBackingStore();
      updateLayoutMetadata();

      context.setTransform(currentDpr, 0, 0, currentDpr, 0, 0);
      context.clearRect(0, 0, cachedWidth, cachedHeight);
      canvas.dataset.renderFrame = String(++renderedFrame);

      const isDark = document.documentElement.classList.contains('dark');
      const sequenceProgress = complete ? 1 : normalize(sweepOffset);
      const sequenceStage = sequenceStageForProgress(sequenceProgress);
      if (container.dataset.sequenceStage !== sequenceStage) {
        syncSequenceStage(sequenceStage);
      }

      const renderedPoints = drawSignals(
        context,
        dataRef.current,
        cachedWidth,
        cachedHeight,
        sequenceProgress,
        isDark,
        labels,
        window.innerWidth,
      );
      if (canvas.dataset.renderPoints !== String(renderedPoints)) {
        canvas.dataset.renderPoints = String(renderedPoints);
      }
      canvas.dataset.sequenceStage = sequenceStage;
    };

    const renderLoop = (timestamp: number) => {
      if (lastTimestamp !== null) {
        accumulatedTime += timestamp - lastTimestamp;
      }
      lastTimestamp = timestamp;

      drawFrame(sequenceProgressForElapsed(accumulatedTime));

      if (canAnimate()) {
        animationFrameId = requestAnimationFrame(renderLoop);
      } else {
        animationFrameId = 0;
        lastTimestamp = null;
      }
    };

    const drawLatest = () => {
      const complete = isReducedMotion || statusRef.current === 'fallback';
      drawFrame(sequenceProgressForElapsed(accumulatedTime), complete);
    };
    drawLatestRef.current = drawLatest;
    canvas.dataset.renderGeneration = String(Number(canvas.dataset.renderGeneration ?? '0') + 1);

    const updateMotionState = () => {
      if (isReducedMotion) {
        setMotionState('reduced');
      } else if (canAnimate()) {
        setMotionState('running');
      } else {
        setMotionState('paused');
      }
    };

    const syncLifecycle = () => {
      updateMotionState();

      if (canAnimate()) {
        if (animationFrameId === 0) {
          lastTimestamp = null;
          animationFrameId = requestAnimationFrame(renderLoop);
        }
        return;
      }

      if (animationFrameId !== 0) {
        cancelAnimationFrame(animationFrameId);
        animationFrameId = 0;
      }
      lastTimestamp = null;
      drawLatest();
    };
    syncLifecycleRef.current = syncLifecycle;

    const resizeObserver = new ResizeObserver((entries) => {
      for (const entry of entries) {
        if (entry.target === visualization) {
          const nextWidth = entry.contentRect.width;
          const nextHeight = entry.contentRect.height;
          if (nextWidth !== cachedWidth || nextHeight !== cachedHeight) {
            cachedWidth = nextWidth;
            cachedHeight = nextHeight;
            drawLatest();
          }
        }
      }
    });
    resizeObserver.observe(visualization);

    const initialRect = visualization.getBoundingClientRect();
    cachedWidth = initialRect.width;
    cachedHeight = initialRect.height;
    isIntersecting =
      initialRect.bottom > 0 &&
      initialRect.right > 0 &&
      initialRect.top < window.innerHeight &&
      initialRect.left < window.innerWidth;
    drawLatest();

    const intersectionObserver = new IntersectionObserver(([entry]) => {
      if (entry.isIntersecting !== isIntersecting) {
        isIntersecting = entry.isIntersecting;
        syncLifecycle();
      }
    });
    intersectionObserver.observe(container);

    const handleVisibilityChange = () => {
      isDocumentVisible = document.visibilityState === 'visible';
      syncLifecycle();
    };
    document.addEventListener('visibilitychange', handleVisibilityChange);

    const themeObserver = new MutationObserver(drawLatest);
    themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] });

    const reducedMotionQuery = window.matchMedia('(prefers-reduced-motion: reduce)');
    const handleReducedMotionChange = (event: MediaQueryListEvent) => {
      isReducedMotion = event.matches;
      setReducedMotion(isReducedMotion);
      syncLifecycle();
    };
    setReducedMotion(isReducedMotion);
    reducedMotionQuery.addEventListener('change', handleReducedMotionChange);

    const handleWindowResize = () => {
      const newDpr = getTargetDpr();
      if (newDpr !== currentDpr) {
        currentDpr = newDpr;
        drawLatest();
      }
    };
    window.addEventListener('resize', handleWindowResize);

    syncLifecycle();

    return () => {
      if (animationFrameId !== 0) cancelAnimationFrame(animationFrameId);
      resizeObserver.disconnect();
      intersectionObserver.disconnect();
      themeObserver.disconnect();
      document.removeEventListener('visibilitychange', handleVisibilityChange);
      reducedMotionQuery.removeEventListener('change', handleReducedMotionChange);
      window.removeEventListener('resize', handleWindowResize);
      if (drawLatestRef.current === drawLatest) drawLatestRef.current = () => undefined;
      if (syncLifecycleRef.current === syncLifecycle) syncLifecycleRef.current = () => undefined;
    };
  }, [labels]);

  const statusLabel =
    status === 'ready' ? labels.wasmReady : status === 'fallback' ? labels.wasmFallback : labels.wasmLoading;
  const controlLabel = status === 'loading'
    ? labels.wasmLoading
    : reducedMotion
      ? labels.reducedMotion
      : status === 'fallback'
        ? labels.staticFallback
        : userPaused
          ? labels.resume
          : labels.pause;
  const controlDisabled = status !== 'ready' || reducedMotion;

  const toggleUserPause = () => {
    const nextPaused = !userPausedRef.current;
    userPausedRef.current = nextPaused;
    setUserPaused(nextPaused);
    syncLifecycleRef.current();
  };

  return (
    <div
      ref={containerRef}
      className="signal-stage"
      data-wasm={status}
      data-motion={motionState}
      data-user-paused={userPaused ? 'true' : 'false'}
      data-sequence-stage="field-pulse"
    >
      <div className="signal-stage-content">
        <div className="signal-sequence-shell" data-signal-region="process-rail">
          <p className="signal-sequence-title">{labels.pipeline}</p>
          <ol className="signal-sequence" aria-label={labels.sequence}>
            <li data-step="field-pulse" data-current="true" aria-current="step">
              <span className="signal-step-marker" aria-hidden="true">01</span>
              <span className="signal-step-label">{labels.fieldPulse}</span>
            </li>
            <li data-step="waveforms" data-current="false">
              <span className="signal-step-marker" aria-hidden="true">02</span>
              <span className="signal-step-label">{labels.referenceResponse}</span>
            </li>
            <li data-step="lock-in" data-current="false">
              <span className="signal-step-marker" aria-hidden="true">03</span>
              <span className="signal-step-label">{labels.lockIn}</span>
            </li>
            <li data-step="rotate-phase" data-current="false">
              <span className="signal-step-marker" aria-hidden="true">04</span>
              <span className="signal-step-label">{labels.rotatePhase}</span>
            </li>
            <li data-step="kerr-angle" data-current="false">
              <span className="signal-step-marker" aria-hidden="true">05</span>
              <span className="signal-step-label">{labels.kerrAngle}</span>
            </li>
          </ol>
        </div>
        <p className="signal-current-stage" data-signal-region="current-stage" aria-hidden="true">{labels.fieldPulse}</p>
        <div className="signal-controls" data-signal-region="control">
          <button
            type="button"
            className="signal-control"
            aria-label={controlLabel}
            aria-pressed={userPaused}
            disabled={controlDisabled}
            onClick={toggleUserPause}
          >
            <span aria-hidden="true">{reducedMotion ? '▣' : userPaused ? '▶' : 'Ⅱ'}</span>
            {controlLabel}
          </button>
        </div>
        <div className="signal-visualization" data-signal-region="visualization" ref={visualizationRef}>
          <canvas
            ref={canvasRef}
            aria-label={labels.label}
            aria-describedby="signal-description"
            role="img"
          />
          <p id="signal-description" className="signal-description">{labels.description}</p>
        </div>
        <div className="signal-hud" data-signal-region="status">
          <span className="signal-status" role="status" aria-live="polite">
            <i className={`dot ${status === 'fallback' ? 'dot-amber' : 'dot-cyan'}`} aria-hidden="true" />
            {statusLabel}
          </span>
        </div>
      </div>
    </div>
  );
}

function normalize(value: number): number {
  return ((value % 1) + 1) % 1;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function smoothStep(value: number): number {
  const normalized = clamp(value, 0, 1);
  return normalized * normalized * (3 - 2 * normalized);
}

function reveal(progress: number, start: number, end: number): number {
  return smoothStep((progress - start) / (end - start));
}

function lerp(from: number, to: number, amount: number): number {
  return from + (to - from) * amount;
}

function makeRect(left: number, top: number, right: number, bottom: number): Rect {
  return { left, top, right, bottom, width: right - left, height: bottom - top };
}

function getSignalLayout(width: number, height: number, viewportWidth = width): SignalLayout {
  const gap = 16;
  const phone = viewportWidth <= 720;
  const wide = viewportWidth >= 960 && width >= 680;

  if (phone) {
    const side = clamp(width * 0.055, 14, 24);
    const top = 16;
    const bottom = 16;
    const minimumHeight = 168 + 144 + 160 + gap * 2;
    const extra = Math.max(0, height - top - bottom - minimumHeight);
    const plotHeight = 168 + extra * 0.36;
    const phaseHeight = 144 + extra * 0.28;
    const plotTop = top;
    const phaseTop = plotTop + plotHeight + gap;
    const outputTop = phaseTop + phaseHeight + gap;
    return {
      safe: makeRect(side, top, width - side, height - bottom),
      plot: makeRect(side, plotTop, width - side, plotTop + plotHeight),
      phase: makeRect(side, phaseTop, width - side, phaseTop + phaseHeight),
      output: makeRect(side, outputTop, width - side, height - bottom),
      mode: 'phone',
      stacked: true,
    };
  }

  if (!wide) {
    const side = clamp(width * 0.045, 20, 32);
    const lowerTop = clamp(height * 0.5, 205, Math.max(205, height - 200));
    const bottom = Math.max(lowerTop + 150, height - side);
    const availableWidth = width - side * 2;
    const sideBySide = width >= 560 && height >= 400;

    if (sideBySide) {
      const panelWidth = (availableWidth - gap) / 2;
      return {
        safe: makeRect(side, side, width - side, bottom),
        plot: makeRect(side, side, width - side, lowerTop - gap),
        phase: makeRect(side, lowerTop, side + panelWidth, bottom),
        output: makeRect(side + panelWidth + gap, lowerTop, width - side, bottom),
        mode: 'compact',
        stacked: false,
      };
    }

    const panelGap = gap;
    const usableHeight = Math.max(180, height - side * 2 - panelGap * 2);
    const plotHeight = Math.max(150, usableHeight * 0.42);
    const phaseHeight = Math.max(128, usableHeight * 0.27);
    const plotTop = side;
    const phaseTop = plotTop + plotHeight + panelGap;
    const outputTop = phaseTop + phaseHeight + panelGap;
    return {
      safe: makeRect(side, side, width - side, height - side),
      plot: makeRect(side, plotTop, width - side, plotTop + plotHeight),
      phase: makeRect(side, phaseTop, width - side, phaseTop + phaseHeight),
      output: makeRect(side, outputTop, width - side, height - side),
      mode: 'compact',
      stacked: true,
    };
  }

  const side = clamp(width * 0.035, 24, 36);
  const top = Math.max(24, height * 0.08);
  const bottom = height - Math.max(24, height * 0.08);
  const availableWidth = width - side * 2;
  const plotWidth = availableWidth * 0.54;
  const panelWidth = (availableWidth - plotWidth - gap * 2) / 2;
  const plotRight = side + plotWidth;
  const phaseLeft = plotRight + gap;
  return {
    safe: makeRect(side, top, width - side, bottom),
    plot: makeRect(side, top, plotRight, bottom),
    phase: makeRect(phaseLeft, top + height * 0.03, phaseLeft + panelWidth, bottom),
    output: makeRect(phaseLeft + panelWidth + gap, top + height * 0.03, width - side, bottom),
    mode: 'wide',
    stacked: false,
  };
}

function periodicSample(values: Float64Array, channel: number, position: number): number {
  const periodicSamples = values.length / 4 - 1;
  const exactIndex = normalize(position) * periodicSamples;
  const index0 = Math.floor(exactIndex) % periodicSamples;
  const index1 = (index0 + 1) % periodicSamples;
  const fraction = exactIndex - index0;
  return values[index0 * 4 + channel] * (1 - fraction) + values[index1 * 4 + channel] * fraction;
}

function drawSignals(
  context: CanvasRenderingContext2D,
  values: Float64Array,
  width: number,
  height: number,
  sequenceProgress: number,
  dark: boolean,
  labels: SignalHeroLabels,
  viewportWidth: number,
): number {
  const count = values.length / 4;
  if (count <= 2) return 0;

  const palette = dark ? DARK_COLORS : LIGHT_COLORS;
  const pointsCount = previewPointCount(width, count);
  const layout = getSignalLayout(width, height, viewportWidth);
  const pulseReveal = reveal(sequenceProgress, 0, 0.3);
  const windowReveal = reveal(sequenceProgress, 0.04, 0.3);
  const waveformReveal = reveal(sequenceProgress, 0.14, 0.52);
  const lockInReveal = reveal(sequenceProgress, 0.34, 0.68);
  const rotateReveal = reveal(sequenceProgress, 0.5, 0.82);
  const kerrReveal = reveal(sequenceProgress, 0.68, 0.96);

  if (layout.stacked) {
    drawFlowConnector(
      context,
      layout.plot.right * 0.5 + layout.plot.left * 0.5,
      layout.plot.bottom,
      layout.phase.left + layout.phase.width * 0.5,
      layout.phase.top,
      palette,
    );
    drawFlowConnector(
      context,
      layout.phase.right,
      layout.phase.top + layout.phase.height * 0.5,
      layout.output.left,
      layout.output.top + layout.output.height * 0.5,
      palette,
    );
  } else {
    drawFlowConnector(
      context,
      layout.plot.right,
      layout.plot.top + layout.plot.height * 0.5,
      layout.phase.left,
      layout.phase.top + layout.phase.height * 0.5,
      palette,
    );
    drawFlowConnector(
      context,
      layout.phase.right,
      layout.phase.top + layout.phase.height * 0.5,
      layout.output.left,
      layout.output.top + layout.output.height * 0.5,
      palette,
    );
  }

  drawTimeDomain(
    context,
    layout.plot,
    values,
    pointsCount,
    pulseReveal,
    windowReveal,
    waveformReveal,
    labels,
    palette,
  );
  drawPhasePlane(context, layout.phase, rotateReveal, lockInReveal, labels, palette);
  drawHarmonicExtraction(context, layout.output, kerrReveal, labels, palette);
  drawPipelineCursor(context, layout, sequenceProgress, palette);

  return pointsCount;
}

function drawTimeDomain(
  context: CanvasRenderingContext2D,
  rect: Rect,
  values: Float64Array,
  points: number,
  pulseActive: number,
  windowActive: number,
  waveformActive: number,
  labels: SignalHeroLabels,
  palette: Palette,
): void {
  drawPanel(context, rect, palette, 1);
  drawFittedText(context, labels.timeDomain, rect.left + 10, rect.top + 17, rect.width * 0.42, 10, palette.muted);
  drawFittedText(
    context,
    labels.triggeredWindow,
    rect.right - 10,
    rect.top + 17,
    rect.width * 0.5,
    9,
    palette.cyan,
    'right',
  );

  const chartLeft = rect.left + rect.width * 0.06;
  const chartRight = rect.right - rect.width * 0.04;
  const pulseBaseline = rect.top + rect.height * 0.3;
  const chartTop = rect.top + rect.height * 0.39;
  const chartBottom = rect.bottom - rect.height * 0.1;
  drawChartGrid(context, chartLeft, chartRight, chartTop, chartBottom, palette);

  const windowStart = chartLeft + (chartRight - chartLeft) * 0.2;
  const windowEnd = chartLeft + (chartRight - chartLeft) * 0.46;
  drawTriggeredWindow(context, windowStart, windowEnd, chartTop, chartBottom, windowActive, palette);
  drawFieldPulse(context, chartLeft, chartRight, pulseBaseline, rect.height * 0.16, pulseActive, palette);

  drawTrace(
    context,
    values,
    2,
    chartLeft,
    chartRight,
    chartTop + (chartBottom - chartTop) * 0.28,
    (chartBottom - chartTop) * 0.17,
    points,
    0.01,
    waveformActive,
    palette.reference,
  );
  drawTrace(
    context,
    values,
    3,
    chartLeft,
    chartRight,
    chartTop + (chartBottom - chartTop) * 0.73,
    (chartBottom - chartTop) * 0.15,
    points,
    0.09,
    waveformActive,
    palette.magenta,
  );

  drawLegendMark(context, chartLeft, chartBottom + 12, palette.reference, 'R', palette);
  drawLegendMark(context, chartLeft + 26, chartBottom + 12, palette.magenta, 'K', palette);
}

function drawPanel(context: CanvasRenderingContext2D, rect: Rect, palette: Palette, alpha: number): void {
  context.save();
  roundedRect(context, rect.left, rect.top, rect.width, rect.height, 8);
  context.fillStyle = palette.panel;
  context.globalAlpha = alpha;
  context.fill();
  context.strokeStyle = palette.panelEdge;
  context.lineWidth = 1;
  context.globalAlpha = alpha * 0.9;
  context.stroke();
  context.restore();
}

function drawChartGrid(
  context: CanvasRenderingContext2D,
  left: number,
  right: number,
  top: number,
  bottom: number,
  palette: Palette,
): void {
  context.save();
  context.strokeStyle = palette.grid;
  context.lineWidth = 1;
  context.globalAlpha = 0.36;
  context.beginPath();
  for (let index = 0; index <= 4; index += 1) {
    const y = top + (bottom - top) * index / 4;
    context.moveTo(left, y);
    context.lineTo(right, y);
  }
  for (let index = 0; index <= 6; index += 1) {
    const x = left + (right - left) * index / 6;
    context.moveTo(x, top);
    context.lineTo(x, bottom);
  }
  context.stroke();
  context.restore();
}

function drawTriggeredWindow(
  context: CanvasRenderingContext2D,
  startX: number,
  endX: number,
  top: number,
  bottom: number,
  active: number,
  palette: Palette,
): void {
  context.save();
  context.fillStyle = palette.cyan;
  context.globalAlpha = 0.025 + active * 0.07;
  context.fillRect(startX, top, endX - startX, bottom - top);
  context.strokeStyle = palette.cyan;
  context.lineWidth = 1;
  context.globalAlpha = 0.2 + active * 0.48;
  context.setLineDash([3, 5]);
  context.beginPath();
  context.moveTo(startX, top - 5);
  context.lineTo(startX, top + 7);
  context.moveTo(startX, top - 5);
  context.lineTo(endX, top - 5);
  context.moveTo(endX, top - 5);
  context.lineTo(endX, top + 7);
  context.stroke();
  context.setLineDash([]);
  const gateX = startX + (endX - startX) * clamp(active, 0, 1);
  context.globalAlpha = 0.25 + active * 0.7;
  context.beginPath();
  context.moveTo(gateX, top - 2);
  context.lineTo(gateX, bottom + 4);
  context.stroke();
  context.restore();
}

function drawFieldPulse(
  context: CanvasRenderingContext2D,
  startX: number,
  endX: number,
  baseline: number,
  amplitude: number,
  active: number,
  palette: Palette,
): void {
  const points = 144;
  const drawPulse = (revealTo: number, alpha: number, fill: boolean) => {
    context.save();
    context.strokeStyle = palette.cyan;
    context.lineWidth = 2;
    context.lineJoin = 'round';
    context.lineCap = 'round';
    context.globalAlpha = alpha;
    context.beginPath();
    let lastX = startX;
    for (let index = 0; index < points; index += 1) {
      const local = index / (points - 1);
      if (local > revealTo) break;
      const x = startX + (endX - startX) * local;
      const y = baseline - finitePulseEnvelope(local) * amplitude;
      if (index === 0) context.moveTo(x, y); else context.lineTo(x, y);
      lastX = x;
    }
    context.stroke();
    if (fill && revealTo > 0) {
      context.globalAlpha = alpha * 0.28;
      context.lineTo(lastX, baseline);
      context.lineTo(startX, baseline);
      context.closePath();
      context.fillStyle = palette.cyan;
      context.fill();

      const markerProgress = clamp(revealTo, 0, 1);
      const markerX = startX + (endX - startX) * markerProgress;
      const markerY = baseline - finitePulseEnvelope(markerProgress) * amplitude;
      const cursorFade = 1 - smoothStep((revealTo - 0.82) / 0.18);
      context.globalAlpha = Math.max(0.16, Math.min(1, (alpha + 0.14) * cursorFade));
      context.fillStyle = palette.cyan;
      context.beginPath();
      context.arc(markerX, markerY, 3.5, 0, TAU);
      context.fill();
    }
    context.restore();
  };

  drawPulse(1, 0.2, false);
  drawPulse(Math.max(active, 0.01), 0.86, true);
}

function drawTrace(
  context: CanvasRenderingContext2D,
  values: Float64Array,
  channel: number,
  startX: number,
  endX: number,
  baseline: number,
  amplitude: number,
  points: number,
  offset: number,
  active: number,
  color: string,
): void {
  const drawPath = (revealTo: number, alpha: number, lineWidth: number) => {
    context.save();
    context.strokeStyle = color;
    context.lineWidth = lineWidth;
    context.lineJoin = 'round';
    context.lineCap = 'round';
    context.globalAlpha = alpha;
    context.beginPath();
    const visiblePoints = Math.max(2, Math.ceil((points - 1) * revealTo) + 1);
    for (let index = 0; index < visiblePoints; index += 1) {
      const ratio = index / (points - 1);
      const x = startX + ratio * (endX - startX);
      const value = periodicSample(values, channel, ratio + offset);
      const y = baseline - value * amplitude;
      if (index === 0) context.moveTo(x, y); else context.lineTo(x, y);
    }
    context.stroke();
    context.restore();
  };

  drawPath(1, 0.16, 1);
  if (active > 0) drawPath(active, 0.92, 1.5);
}

function drawLegendMark(
  context: CanvasRenderingContext2D,
  x: number,
  y: number,
  color: string,
  label: string,
  palette: Palette,
): void {
  context.save();
  context.strokeStyle = color;
  context.lineWidth = 2;
  context.globalAlpha = 0.9;
  context.beginPath();
  context.moveTo(x, y - 3);
  context.lineTo(x + 12, y - 3);
  context.stroke();
  drawFittedText(context, label, x + 16, y, 16, 9, palette.muted);
  context.restore();
}

function drawPhasePlane(
  context: CanvasRenderingContext2D,
  rect: Rect,
  active: number,
  lockInActive: number,
  labels: SignalHeroLabels,
  palette: Palette,
): void {
  drawPanel(context, rect, palette, 1);
  drawFittedText(context, labels.phaseSpace, rect.left + 10, rect.top + 17, rect.width * 0.56, 10, palette.muted);
  drawFittedText(context, labels.perHarmonic, rect.right - 8, rect.bottom - 8, rect.width * 0.8, 9, palette.cyan, 'right');

  const originX = rect.left + rect.width * 0.5;
  const originY = rect.top + rect.height * 0.58;
  const radius = Math.min(rect.width * 0.31, rect.height * 0.29);
  const pre: [number, number] = [0.72, 0.42];
  const delta = 0.72;
  const post = rotatePhasePoint(pre[0], pre[1], delta);
  const current: [number, number] = [lerp(pre[0], post[0], active), lerp(pre[1], post[1], active)];
  const scale = radius / Math.max(Math.hypot(...pre), Math.hypot(...post));

  context.save();
  context.strokeStyle = palette.grid;
  context.lineWidth = 1;
  context.globalAlpha = 0.38;
  context.beginPath();
  context.arc(originX, originY, radius, 0, TAU);
  context.moveTo(originX - radius, originY);
  context.lineTo(originX + radius, originY);
  context.moveTo(originX, originY - radius);
  context.lineTo(originX, originY + radius);
  context.stroke();

  drawPhaseVector(context, originX, originY, pre, scale, palette.reference, 0.26, [4, 4]);
  drawPhaseVector(context, originX, originY, post, scale, palette.magenta, 0.2, [3, 5]);
  drawPhaseVector(context, originX, originY, current, scale, palette.magenta, 0.3 + active * 0.7, []);

  const preAngle = Math.atan2(pre[1], pre[0]);
  const currentAngle = Math.atan2(current[1], current[0]);
  context.strokeStyle = palette.green;
  context.lineWidth = 2;
  context.globalAlpha = 0.2 + active * 0.78;
  context.beginPath();
  context.arc(originX, originY, radius * 0.56, -preAngle, -currentAngle, false);
  context.stroke();

  drawFittedText(context, 'X', originX + radius + 4, originY + 4, 16, 9, palette.muted);
  drawFittedText(context, 'Y', originX + 4, originY - radius - 4, 16, 9, palette.muted);
  drawFittedText(context, 'Δφₙ', originX + radius * 0.18, originY - radius * 0.36, rect.width * 0.35, 9, palette.green);
  context.restore();

  if (lockInActive > 0) {
    drawSignalDot(
      context,
      originX + current[0] * scale,
      originY - current[1] * scale,
      palette.magenta,
      0.45 + lockInActive * 0.55,
    );
  }
}

function drawPhaseVector(
  context: CanvasRenderingContext2D,
  originX: number,
  originY: number,
  point: [number, number],
  scale: number,
  color: string,
  alpha: number,
  dash: number[],
): void {
  context.save();
  context.strokeStyle = color;
  context.lineWidth = 2;
  context.globalAlpha = alpha;
  context.setLineDash(dash);
  context.beginPath();
  context.moveTo(originX, originY);
  context.lineTo(originX + point[0] * scale, originY - point[1] * scale);
  context.stroke();
  context.setLineDash([]);
  context.restore();
}

function drawHarmonicExtraction(
  context: CanvasRenderingContext2D,
  rect: Rect,
  active: number,
  labels: SignalHeroLabels,
  palette: Palette,
): void {
  drawPanel(context, rect, palette, 1);
  drawFittedText(context, labels.harmonicExtraction, rect.left + 8, rect.top + 17, rect.width - 16, 9, palette.muted);

  const rows = [
    ['A₂', HARMONIC_KERR_CUE.a2, palette.cyan],
    ['A₃', HARMONIC_KERR_CUE.a3, palette.magenta],
    ['A₄', HARMONIC_KERR_CUE.a4, palette.green],
    ['A₆', HARMONIC_KERR_CUE.a6, palette.amber],
  ] as const;
  const barMax = Math.max(...rows.map(([, value]) => value));
  const rowTop = rect.top + rect.height * 0.23;
  const rowHeight = rect.height * 0.11;
  const barLeft = rect.left + 22;
  const barRight = rect.right - 8;

  rows.forEach(([name, value, color], index) => {
    const y = rowTop + index * rowHeight;
    drawFittedText(context, name, rect.left + 8, y + 8, 14, 9, palette.muted);
    context.save();
    context.fillStyle = color;
    context.globalAlpha = 0.13;
    context.fillRect(barLeft, y, barRight - barLeft, 5);
    context.globalAlpha = 0.22 + active * 0.72;
    context.fillRect(barLeft, y, (barRight - barLeft) * (value / barMax) * active, 5);
    context.restore();
  });

  const outputTop = rect.top + rect.height * 0.72;
  const outputCenterX = rect.left + rect.width * 0.5;
  const outputCenterY = rect.bottom - rect.height * 0.1;
  const compact = rect.height < 150;
  const modulationLabel = compact ? 'A₂ A₄ A₆ → MOD DEPTH' : 'A₂ + A₄ + A₆ → MODULATION DEPTH';
  const kerrLabel = compact ? 'A₂ A₃ A₄ + MOD → θK' : 'A₂ + A₃ + A₄ + MOD DEPTH → θK';
  drawFittedText(context, modulationLabel, rect.left + 8, outputTop - 5, rect.width - 16, 9, palette.cyan);
  drawFittedText(context, kerrLabel, rect.left + 8, outputTop + 10, rect.width - 16, 9, palette.green);
  context.save();
  context.strokeStyle = palette.green;
  context.lineWidth = 2;
  context.globalAlpha = 0.25 + active * 0.75;
  context.beginPath();
  context.arc(outputCenterX, outputCenterY, Math.min(rect.width, rect.height) * 0.12, -Math.PI * 0.85, -Math.PI * 0.85 + HARMONIC_KERR_CUE.angleRad * active, false);
  context.stroke();
  drawFittedText(context, 'θK', outputCenterX + 7, outputCenterY + 4, rect.width * 0.32, 11, palette.green);
  context.restore();

  context.save();
  context.strokeStyle = palette.grid;
  context.lineWidth = 1;
  context.globalAlpha = 0.35 + active * 0.35;
  context.setLineDash([2, 4]);
  context.beginPath();
  context.moveTo(barRight, rowTop + rowHeight * 1.5);
  context.lineTo(outputCenterX, outputTop);
  context.stroke();
  context.setLineDash([]);
  context.restore();
}

function drawFlowConnector(
  context: CanvasRenderingContext2D,
  fromX: number,
  fromY: number,
  toX: number,
  toY: number,
  palette: Palette,
): void {
  context.save();
  context.strokeStyle = palette.cyan;
  context.lineWidth = 1;
  context.globalAlpha = 0.28;
  context.setLineDash([3, 5]);
  context.beginPath();
  context.moveTo(fromX, fromY);
  context.lineTo(toX, toY);
  context.stroke();
  context.setLineDash([]);
  context.restore();
}

function drawPipelineCursor(
  context: CanvasRenderingContext2D,
  layout: SignalLayout,
  progress: number,
  palette: Palette,
): void {
  if (progress < 0.4 || progress >= 0.96) return;

  let fromX: number;
  let fromY: number;
  let toX: number;
  let toY: number;
  let active: number;
  if (progress < 0.68) {
    fromX = layout.stacked ? (layout.plot.left + layout.plot.right) * 0.5 : layout.plot.right;
    fromY = layout.stacked ? layout.plot.bottom : layout.plot.top + layout.plot.height * 0.5;
    toX = layout.phase.left;
    toY = layout.stacked ? layout.phase.top : layout.phase.top + layout.phase.height * 0.5;
    active = (progress - 0.4) / 0.28;
  } else {
    fromX = layout.phase.right;
    fromY = layout.phase.top + layout.phase.height * 0.5;
    toX = layout.output.left;
    toY = layout.output.top + layout.output.height * 0.5;
    active = (progress - 0.68) / 0.28;
  }
  const position = smoothStep(clamp(active, 0, 1));
  drawSignalDot(context, lerp(fromX, toX, position), lerp(fromY, toY, position), palette.cyan, 0.4 + clamp(active, 0, 1) * 0.6);
}

function drawSignalDot(context: CanvasRenderingContext2D, x: number, y: number, color: string, alpha: number): void {
  context.save();
  context.fillStyle = color;
  context.globalAlpha = alpha;
  context.beginPath();
  context.arc(x, y, 3, 0, TAU);
  context.fill();
  context.restore();
}

function roundedRect(context: CanvasRenderingContext2D, x: number, y: number, width: number, height: number, radius: number): void {
  const r = Math.min(radius, width / 2, height / 2);
  context.beginPath();
  context.moveTo(x + r, y);
  context.arcTo(x + width, y, x + width, y + height, r);
  context.arcTo(x + width, y + height, x, y + height, r);
  context.arcTo(x, y + height, x, y, r);
  context.arcTo(x, y, x + width, y, r);
  context.closePath();
}

function drawFittedText(
  context: CanvasRenderingContext2D,
  text: string,
  x: number,
  y: number,
  maxWidth: number,
  size: number,
  color: string,
  align: CanvasTextAlign = 'left',
): void {
  context.save();
  context.fillStyle = color;
  context.globalAlpha = 0.9;
  context.textAlign = align;
  context.textBaseline = 'alphabetic';
  context.font = `650 ${size}px ui-monospace, SFMono-Regular, Menlo, monospace`;
  context.fillText(text, x, y, Math.max(12, maxWidth));
  context.restore();
}
