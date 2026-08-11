'use client';

import { useEffect, useRef, useState } from 'react';
import { basePath } from '@/lib/shared';

const TAU = Math.PI * 2;
const LOOP_DURATION_MS = 24_000;
const PREVIEW_SAMPLES = 720;
const INITIAL_PHASE = 0.17;
const MIN_RENDER_POINTS = 256;

type SignalStatus = 'loading' | 'ready' | 'fallback';
type MotionState = 'running' | 'paused' | 'reduced';
type SequenceStage = 'acquisition' | 'field-pulse' | 'waveforms' | 'lock-in' | 'kerr-angle';

export type SignalHeroLabels = {
  label: string;
  description: string;
  sequence: string;
  fieldPulse: string;
  acquisitionWindow: string;
  reference: string;
  kerrResponse: string;
  lockInX: string;
  lockInY: string;
  kerrAngle: string;
  pause: string;
  resume: string;
  reducedMotion: string;
  staticFallback: string;
  wasmLoading: string;
  wasmReady: string;
  wasmFallback: string;
};

const DARK_COLORS = {
  reference: '#e7edf0',
  cyan: '#16d9d1',
  magenta: '#ed4f9a',
  green: '#74e1a4',
  amber: '#f4bd6b',
  grid: 'rgba(145, 164, 174, 0.34)',
};

const LIGHT_COLORS = {
  reference: '#44575a',
  cyan: '#087b7b',
  magenta: '#bd286f',
  green: '#137946',
  amber: '#a05d00',
  grid: 'rgba(40, 67, 68, 0.36)',
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

const FALLBACK_SIGNAL = fallbackSignal(PREVIEW_SAMPLES);

export function previewPointCount(width: number, samples: number): number {
  const periodicSamples = Math.max(2, samples - 1);
  return Math.min(periodicSamples, Math.max(MIN_RENDER_POINTS, Math.ceil(width) + 1));
}

export function SignalHero({ labels }: { labels: SignalHeroLabels }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
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
    const canvas = canvasRef.current;
    const container = containerRef.current;
    if (!canvas || !container) return;
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

    const getTargetDpr = () => Math.min(window.devicePixelRatio || 1, 2);
    let currentDpr = getTargetDpr();

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
      statusRef.current !== 'fallback';

    const drawFrame = (sweepOffset: number, complete = false) => {
      if (cachedWidth === 0 || cachedHeight === 0) return;
      updateCanvasBackingStore();

      context.setTransform(currentDpr, 0, 0, currentDpr, 0, 0);
      context.clearRect(0, 0, cachedWidth, cachedHeight);

      const isDark = document.documentElement.classList.contains('dark');
      const sequenceProgress = complete ? 1 : normalize(sweepOffset);
      const sequenceStage = sequenceStageForProgress(sequenceProgress);
      if (container.dataset.sequenceStage !== sequenceStage) {
        container.dataset.sequenceStage = sequenceStage;
      }

      const renderedPoints = drawSignals(
        context,
        dataRef.current,
        cachedWidth,
        cachedHeight,
        sequenceProgress,
        isDark,
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

      const sweepOffset = (accumulatedTime % LOOP_DURATION_MS) / LOOP_DURATION_MS;
      drawFrame(sweepOffset);

      if (canAnimate()) {
        animationFrameId = requestAnimationFrame(renderLoop);
      } else {
        animationFrameId = 0;
        lastTimestamp = null;
      }
    };

    const drawLatest = () => {
      const complete = isReducedMotion || statusRef.current === 'fallback';
      const sweepOffset = (accumulatedTime % LOOP_DURATION_MS) / LOOP_DURATION_MS;
      drawFrame(sweepOffset, complete);
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
        if (entry.target === container) {
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
    resizeObserver.observe(container);

    const initialRect = container.getBoundingClientRect();
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
  }, []);

  const statusLabel =
    status === 'ready' ? labels.wasmReady : status === 'fallback' ? labels.wasmFallback : labels.wasmLoading;
  const controlLabel = reducedMotion
    ? labels.reducedMotion
    : status === 'fallback'
      ? labels.staticFallback
      : userPaused
        ? labels.resume
        : labels.pause;
  const controlDisabled = reducedMotion || status === 'fallback';

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
      data-sequence-stage="acquisition"
    >
      <canvas
        ref={canvasRef}
        aria-label={labels.label}
        aria-describedby="signal-description"
        role="img"
      />
      <ol className="signal-sequence" aria-label={labels.sequence}>
        <li data-step="field-pulse">{labels.fieldPulse}</li>
        <li data-step="acquisition-window">{labels.acquisitionWindow}</li>
        <li data-step="reference">{labels.reference}</li>
        <li data-step="kerr-response">{labels.kerrResponse}</li>
        <li data-step="lock-in-x">{labels.lockInX}</li>
        <li data-step="lock-in-y">{labels.lockInY}</li>
        <li data-step="kerr-angle">{labels.kerrAngle}</li>
      </ol>
      <p id="signal-description" className="signal-description">{labels.description}</p>
      <div className="signal-hud">
        <span className="signal-status" role="status" aria-live="polite">
          <i className={`dot ${status === 'fallback' ? 'dot-amber' : 'dot-cyan'}`} aria-hidden="true" />
          {statusLabel}
        </span>
      </div>
      <div className="signal-controls">
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

function sequenceStageForProgress(progress: number): SequenceStage {
  if (progress < 0.22) return 'acquisition';
  if (progress < 0.44) return 'field-pulse';
  if (progress < 0.68) return 'waveforms';
  if (progress < 0.84) return 'lock-in';
  return 'kerr-angle';
}

function finitePulseEnvelope(localTime: number): number {
  // The site illustration follows the measured pulse morphology: a fast
  // positive lobe, a smaller negative undershoot, and a return to baseline.
  // It is deliberately normalized; no private capture or instrument scale is
  // embedded in the public hero graphic.
  const onset = smoothStep(localTime / 0.1);
  const settle = 1 - smoothStep((localTime - 0.78) / 0.22);
  const positiveLobe = Math.exp(-0.5 * ((localTime - 0.28) / 0.16) ** 2);
  const negativeUndershoot = 0.24 * Math.exp(-0.5 * ((localTime - 0.62) / 0.12) ** 2);
  return onset * settle * (positiveLobe - negativeUndershoot);
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
): number {
  const count = values.length / 4;
  if (count <= 2) return 0;

  const palette = dark ? DARK_COLORS : LIGHT_COLORS;
  const pointsCount = previewPointCount(width, count);
  const compactLayout = width < 860;
  const plotLeft = compactLayout ? width * 0.08 : width * 0.56;
  const plotRight = compactLayout ? width * 0.92 : width * 0.95;
  const plotWidth = plotRight - plotLeft;
  const acquisitionStart = plotLeft;
  const acquisitionEnd = plotRight;
  const traceStart = acquisitionStart + plotWidth * 0.08;
  const traceEnd = acquisitionEnd - plotWidth * 0.06;
  const acquisitionReveal = reveal(sequenceProgress, 0, 0.18);
  const pulseReveal = reveal(sequenceProgress, 0, 0.36);
  const waveformReveal = reveal(sequenceProgress, 0.22, 0.62);
  const lockInReveal = reveal(sequenceProgress, 0.48, 0.82);
  const angleReveal = reveal(sequenceProgress, 0.68, 0.94);

  drawAcquisitionWindow(context, width, height, acquisitionStart, acquisitionEnd, acquisitionReveal, palette);
  drawFieldPulse(context, height, traceStart, traceEnd, pulseReveal, compactLayout, palette);

  drawTrace(
    context,
    values,
    2,
    traceStart,
    traceEnd,
    height * (compactLayout ? 0.32 : 0.49),
    height * (compactLayout ? 0.04 : 0.075),
    pointsCount,
    sequenceProgress * 0.12,
    waveformReveal,
    palette.reference,
  );
  drawTrace(
    context,
    values,
    3,
    traceStart,
    traceEnd,
    height * (compactLayout ? 0.365 : 0.65),
    height * (compactLayout ? 0.035 : 0.062),
    pointsCount,
    sequenceProgress * 0.12 + 0.08,
    waveformReveal,
    palette.magenta,
  );
  drawLockInCue(context, width, height, lockInReveal, palette);
  drawKerrAngleCue(context, width, height, angleReveal, palette);

  return pointsCount;
}

function drawAcquisitionWindow(
  context: CanvasRenderingContext2D,
  width: number,
  height: number,
  startX: number,
  endX: number,
  active: number,
  palette: typeof DARK_COLORS,
): void {
  const compactLayout = width < 860;
  const top = height * (compactLayout ? 0.3 : 0.36);
  const bottom = height * (compactLayout ? 0.4 : 0.72);
  context.save();
  context.lineWidth = 1;
  context.strokeStyle = palette.grid;
  context.globalAlpha = 0.18 + active * 0.72;
  context.setLineDash([7, 7]);
  context.strokeRect(startX, top, endX - startX, bottom - top);
  context.setLineDash([]);
  context.globalAlpha = 0.04 + active * 0.1;
  context.fillStyle = palette.cyan;
  context.fillRect(startX, top, (endX - startX) * active, bottom - top);
  context.globalAlpha = 0.35 + active * 0.65;
  context.beginPath();
  context.moveTo(startX, top - height * 0.04);
  context.lineTo(startX, bottom + height * 0.04);
  context.moveTo(endX, top - height * 0.04);
  context.lineTo(endX, bottom + height * 0.04);
  const gateX = startX + (endX - startX) * active;
  context.strokeStyle = palette.cyan;
  context.globalAlpha = 0.18 + active * 0.72;
  context.moveTo(gateX, top);
  context.lineTo(gateX, bottom);
  context.stroke();
  context.restore();
}

function drawFieldPulse(
  context: CanvasRenderingContext2D,
  height: number,
  startX: number,
  endX: number,
  active: number,
  compactLayout: boolean,
  palette: typeof DARK_COLORS,
): void {
  const baseline = height * 0.24;
  const amplitude = height * (compactLayout ? 0.065 : 0.14);
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
    for (let i = 0; i < points; i += 1) {
      const local = i / (points - 1);
      if (local > revealTo) break;
      const x = startX + (endX - startX) * local;
      const y = baseline - finitePulseEnvelope(local) * amplitude;
      if (i === 0) context.moveTo(x, y); else context.lineTo(x, y);
      lastX = x;
    }
    context.stroke();
    if (fill && revealTo > 0) {
      context.globalAlpha = alpha * 0.32;
      context.lineTo(lastX, baseline);
      context.lineTo(startX, baseline);
      context.closePath();
      context.fillStyle = palette.cyan;
      context.fill();

      const markerProgress = clamp(revealTo, 0, 1);
      const markerX = startX + (endX - startX) * markerProgress;
      const markerY = baseline - finitePulseEnvelope(markerProgress) * amplitude;
      context.globalAlpha = Math.min(1, alpha + 0.14);
      context.fillStyle = palette.cyan;
      context.beginPath();
      context.arc(markerX, markerY, 3.5, 0, TAU);
      context.fill();
    }
    context.restore();
  };

  drawPulse(1, 0.22, false);
  drawPulse(Math.max(active, 0.01), 0.8, true);
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
    for (let i = 0; i < visiblePoints; i += 1) {
      const ratio = i / (points - 1);
      const x = startX + ratio * (endX - startX);
      const value = periodicSample(values, channel, ratio + offset);
      const y = baseline - value * amplitude;
      if (i === 0) context.moveTo(x, y); else context.lineTo(x, y);
    }
    context.stroke();
    context.restore();
  };

  drawPath(1, 0.18, 1);
  if (active > 0) drawPath(active, 0.88, 1.5);
}

function drawLockInCue(
  context: CanvasRenderingContext2D,
  width: number,
  height: number,
  active: number,
  palette: typeof DARK_COLORS,
): void {
  const originX = width * 0.86;
  const originY = height * 0.58;
  const axisWidth = Math.min(width * 0.08, 76);
  const axisHeight = Math.min(height * 0.12, 72);
  const vectorX = originX + axisWidth * 0.76;
  const vectorY = originY - axisHeight * 0.72;

  context.save();
  context.lineWidth = 1;
  context.strokeStyle = palette.grid;
  context.globalAlpha = 0.2 + active * 0.6;
  context.beginPath();
  context.moveTo(originX - axisWidth * 0.14, originY);
  context.lineTo(originX + axisWidth, originY);
  context.moveTo(originX, originY + axisHeight * 0.18);
  context.lineTo(originX, originY - axisHeight);
  context.stroke();

  context.strokeStyle = palette.cyan;
  context.lineWidth = 2;
  context.globalAlpha = 0.2 + active * 0.78;
  context.beginPath();
  context.moveTo(originX, originY);
  context.lineTo(vectorX, originY);
  context.stroke();

  context.strokeStyle = palette.magenta;
  context.beginPath();
  context.moveTo(originX, originY);
  context.lineTo(vectorX, vectorY);
  context.stroke();

  context.fillStyle = palette.magenta;
  context.beginPath();
  context.arc(vectorX, vectorY, 3.5, 0, TAU);
  context.fill();
  context.restore();
}

function drawKerrAngleCue(
  context: CanvasRenderingContext2D,
  width: number,
  height: number,
  active: number,
  palette: typeof DARK_COLORS,
): void {
  const originX = width * 0.86;
  const originY = height * 0.58;
  const radius = Math.min(width * 0.055, 50);

  context.save();
  context.strokeStyle = palette.green;
  context.lineWidth = 2;
  context.globalAlpha = 0.2 + active * 0.78;
  context.beginPath();
  context.arc(originX, originY, radius, -Math.PI * 0.78, -Math.PI * 0.28);
  context.stroke();
  context.restore();
}
