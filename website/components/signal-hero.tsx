'use client';

import { useTheme } from 'next-themes';
import { useEffect, useRef, useState } from 'react';
import { basePath } from '@/lib/shared';

const TAU = Math.PI * 2;
const LOOP_DURATION_MS = 24_000;
const PREVIEW_SAMPLES = 720;
const INITIAL_PHASE = 0.17;

const CHANNELS = [
  { offset: 1, darkColor: '#e7edf0', lightColor: '#44575a', width: 1.15 },
  { offset: 2, darkColor: '#16d9d1', lightColor: '#087b7b', width: 1.8 },
  { offset: 3, darkColor: '#ed4f9a', lightColor: '#bd286f', width: 1.45 },
] as const;

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

export function SignalHero({ label }: { label: string }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const dataRef = useRef<Float64Array>(FALLBACK_SIGNAL);
  const [status, setStatus] = useState<'loading' | 'ready' | 'fallback'>('loading');
  const [motionState, setMotionState] = useState<'running' | 'paused' | 'reduced'>('paused');
  const { resolvedTheme } = useTheme();

  useEffect(() => {
    let terminated = false;
    const worker = new Worker(`${basePath}/workers/signal.worker.js`, { type: 'module' });
    worker.onmessage = (event: MessageEvent<{ type: string; data?: ArrayBuffer }>) => {
      if (terminated) return;
      if (event.data.type === 'ready' && event.data.data) {
        dataRef.current = new Float64Array(event.data.data);
        setStatus('ready');
      } else if (event.data.type === 'error') {
        setStatus('fallback');
      }
    };
    worker.onerror = () => {
      if (!terminated) setStatus('fallback');
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
    let startTime: number | null = null;
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

    const drawFrame = (sweepOffset: number) => {
      if (cachedWidth === 0 || cachedHeight === 0) return;
      updateCanvasBackingStore();

      context.setTransform(currentDpr, 0, 0, currentDpr, 0, 0);
      context.clearRect(0, 0, cachedWidth, cachedHeight);

      const isDark =
        resolvedTheme === 'dark' ||
        (!resolvedTheme && document.documentElement.classList.contains('dark'));

      drawGrid(context, cachedWidth, cachedHeight, isDark);
      drawSignals(context, dataRef.current, cachedWidth, cachedHeight, sweepOffset, isDark);
    };

    const renderLoop = (timestamp: number) => {
      if (startTime === null) startTime = timestamp;
      if (lastTimestamp !== null) {
        accumulatedTime += timestamp - lastTimestamp;
      }
      lastTimestamp = timestamp;

      const sweepOffset = (accumulatedTime % LOOP_DURATION_MS) / LOOP_DURATION_MS;
      drawFrame(sweepOffset);

      if (isIntersecting && isDocumentVisible && !isReducedMotion) {
        animationFrameId = requestAnimationFrame(renderLoop);
      } else {
        animationFrameId = 0;
        lastTimestamp = null;
      }
    };

    const updateMotionState = () => {
      if (isReducedMotion) {
        setMotionState('reduced');
      } else if (isIntersecting && isDocumentVisible) {
        setMotionState('running');
      } else {
        setMotionState('paused');
      }
    };

    const syncLifecycle = () => {
      updateMotionState();

      if (animationFrameId !== 0) {
        cancelAnimationFrame(animationFrameId);
        animationFrameId = 0;
        lastTimestamp = null;
      }

      if (isReducedMotion) {
        drawFrame(0);
      } else if (isIntersecting && isDocumentVisible) {
        animationFrameId = requestAnimationFrame(renderLoop);
      } else {
        const sweepOffset = (accumulatedTime % LOOP_DURATION_MS) / LOOP_DURATION_MS;
        drawFrame(sweepOffset);
      }
    };

    const resizeObserver = new ResizeObserver((entries) => {
      for (const entry of entries) {
        if (entry.target === container) {
          cachedWidth = entry.contentRect.width;
          cachedHeight = entry.contentRect.height;
          syncLifecycle();
        }
      }
    });
    resizeObserver.observe(container);

    const intersectionObserver = new IntersectionObserver(([entry]) => {
      isIntersecting = entry.isIntersecting;
      syncLifecycle();
    });
    intersectionObserver.observe(container);

    const handleVisibilityChange = () => {
      isDocumentVisible = document.visibilityState === 'visible';
      syncLifecycle();
    };
    document.addEventListener('visibilitychange', handleVisibilityChange);

    const reducedMotionQuery = window.matchMedia('(prefers-reduced-motion: reduce)');
    const handleReducedMotionChange = (event: MediaQueryListEvent) => {
      isReducedMotion = event.matches;
      syncLifecycle();
    };
    reducedMotionQuery.addEventListener('change', handleReducedMotionChange);

    const handleWindowResize = () => {
      const newDpr = getTargetDpr();
      if (newDpr !== currentDpr) {
        currentDpr = newDpr;
      }
      syncLifecycle();
    };
    window.addEventListener('resize', handleWindowResize);

    syncLifecycle();

    return () => {
      if (animationFrameId !== 0) cancelAnimationFrame(animationFrameId);
      resizeObserver.disconnect();
      intersectionObserver.disconnect();
      document.removeEventListener('visibilitychange', handleVisibilityChange);
      reducedMotionQuery.removeEventListener('change', handleReducedMotionChange);
      window.removeEventListener('resize', handleWindowResize);
    };
  }, [resolvedTheme, status]);

  return (
    <div
      ref={containerRef}
      className="signal-stage"
      data-wasm={status}
      data-motion={motionState}
    >
      <canvas ref={canvasRef} aria-label={label} role="img" />
      <div className="signal-hud" aria-hidden="true">
        <span><i className="dot dot-cyan" />LOCK-IN X</span>
        <span><i className="dot dot-magenta" />LOCK-IN Y</span>
        <span className="wasm-state">{status === 'ready' ? 'WASM ONLINE' : status.toUpperCase()}</span>
      </div>
    </div>
  );
}

function drawGrid(context: CanvasRenderingContext2D, width: number, height: number, dark: boolean) {
  context.strokeStyle = dark ? 'rgba(145, 164, 174, 0.13)' : 'rgba(40, 67, 68, 0.14)';
  context.lineWidth = 1;
  for (let x = 0; x <= width; x += 48) { context.beginPath(); context.moveTo(x, 0); context.lineTo(x, height); context.stroke(); }
  for (let y = 0; y <= height; y += 40) { context.beginPath(); context.moveTo(0, y); context.lineTo(width, y); context.stroke(); }
}

function drawSignals(
  context: CanvasRenderingContext2D,
  values: Float64Array,
  width: number,
  height: number,
  sweepOffset: number,
  dark: boolean,
) {
  const count = values.length / 4;
  if (count <= 1) return;

  const pointsCount = Math.max(128, Math.min(count, Math.ceil(width / 2)));
  const step = 1 / (pointsCount - 1);

  for (const channel of CHANNELS) {
    context.beginPath();
    context.strokeStyle = dark ? channel.darkColor : channel.lightColor;
    context.lineWidth = channel.width;
    context.globalAlpha = channel.offset === 1 ? 0.36 : 0.92;

    for (let i = 0; i < pointsCount; i += 1) {
      const xRatio = i * step;
      const x = xRatio * width;

      let samplePos = (xRatio + sweepOffset) % 1.0;
      if (samplePos < 0) samplePos += 1.0;

      const exactIndex = samplePos * (count - 1);
      const idx0 = Math.floor(exactIndex);
      const idx1 = Math.min(count - 1, idx0 + 1);
      const frac = exactIndex - idx0;

      const val0 = values[idx0 * 4 + channel.offset];
      const val1 = values[idx1 * 4 + channel.offset];
      const val = val0 * (1 - frac) + val1 * frac;

      const lane = channel.offset === 1 ? 0.34 : channel.offset === 2 ? 0.58 : 0.72;
      const y = height * lane - val * height * 0.19;

      if (i === 0) context.moveTo(x, y); else context.lineTo(x, y);
    }
    context.stroke();
  }
  context.globalAlpha = 1;
}
