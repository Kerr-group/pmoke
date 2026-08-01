'use client';

import { useEffect, useRef, useState } from 'react';
import { basePath } from '@/lib/shared';

const CHANNELS = [
  { offset: 1, color: '#e7edf0', width: 1.15 },
  { offset: 2, color: '#16d9d1', width: 1.8 },
  { offset: 3, color: '#ed4f9a', width: 1.45 },
] as const;
const FALLBACK_SIGNAL = fallbackSignal(360);

export function SignalHero({ label }: { label: string }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const dataRef = useRef<Float64Array | undefined>(undefined);
  const [status, setStatus] = useState<'loading' | 'ready' | 'fallback'>('loading');

  useEffect(() => {
    const worker = new Worker(`${basePath}/workers/signal.worker.js`, { type: 'module' });
    worker.onmessage = (event: MessageEvent<{ type: string; data?: ArrayBuffer }>) => {
      if (event.data.type === 'ready' && event.data.data) {
        dataRef.current = new Float64Array(event.data.data);
        setStatus('ready');
      } else if (event.data.type === 'error') {
        setStatus('fallback');
      }
    };
    worker.onerror = () => setStatus('fallback');
    worker.postMessage({ type: 'init', basePath, samples: 720 });
    return () => worker.terminate();
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const context = canvas.getContext('2d');
    if (!context) return;
    let frame = 0;
    let visible = true;
    let animation = 0;
    const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    const observer = new IntersectionObserver(([entry]) => { visible = entry.isIntersecting; });
    observer.observe(canvas);

    const draw = () => {
      if (visible) {
        const rect = canvas.getBoundingClientRect();
        const ratio = Math.min(window.devicePixelRatio || 1, 2);
        const width = Math.max(1, Math.round(rect.width * ratio));
        const height = Math.max(1, Math.round(rect.height * ratio));
        if (canvas.width !== width || canvas.height !== height) {
          canvas.width = width;
          canvas.height = height;
        }
        context.setTransform(ratio, 0, 0, ratio, 0, 0);
        context.clearRect(0, 0, rect.width, rect.height);
        drawGrid(context, rect.width, rect.height);
        const values = dataRef.current ?? FALLBACK_SIGNAL;
        const phase = reducedMotion ? 0 : frame * 0.0018;
        drawSignals(context, values, rect.width, rect.height, phase);
        frame += 1;
      }
      animation = requestAnimationFrame(draw);
    };
    animation = requestAnimationFrame(draw);
    return () => { cancelAnimationFrame(animation); observer.disconnect(); };
  }, []);

  return (
    <div className="signal-stage" data-wasm={status}>
      <canvas ref={canvasRef} aria-label={label} role="img" />
      <div className="signal-hud" aria-hidden="true">
        <span><i className="dot dot-cyan" />LOCK-IN X</span>
        <span><i className="dot dot-magenta" />LOCK-IN Y</span>
        <span className="wasm-state">{status === 'ready' ? 'WASM ONLINE' : status.toUpperCase()}</span>
      </div>
    </div>
  );
}

function drawGrid(context: CanvasRenderingContext2D, width: number, height: number) {
  context.strokeStyle = 'rgba(145, 164, 174, 0.13)';
  context.lineWidth = 1;
  for (let x = 0; x <= width; x += 48) { context.beginPath(); context.moveTo(x, 0); context.lineTo(x, height); context.stroke(); }
  for (let y = 0; y <= height; y += 40) { context.beginPath(); context.moveTo(0, y); context.lineTo(width, y); context.stroke(); }
}

function drawSignals(context: CanvasRenderingContext2D, values: Float64Array, width: number, height: number, phase: number) {
  const count = values.length / 4;
  for (const channel of CHANNELS) {
    context.beginPath();
    context.strokeStyle = channel.color;
    context.lineWidth = channel.width;
    context.globalAlpha = channel.offset === 1 ? 0.36 : 0.92;
    for (let index = 0; index < count; index += 1) {
      const t = values[index * 4];
      const value = values[index * 4 + channel.offset];
      const x = t * width;
      const lane = channel.offset === 1 ? 0.34 : channel.offset === 2 ? 0.58 : 0.72;
      const drift = Math.sin(t * 8 + phase) * 3;
      const y = height * lane - value * height * 0.19 + drift;
      if (index === 0) context.moveTo(x, y); else context.lineTo(x, y);
    }
    context.stroke();
  }
  context.globalAlpha = 1;
}

function fallbackSignal(samples: number): Float64Array {
  const output = new Float64Array(samples * 4);
  for (let i = 0; i < samples; i += 1) {
    const t = i / (samples - 1);
    output.set([t, Math.sin(t * 42) * 0.4, Math.sin(t * 8) * 0.7, Math.cos(t * 8) * 0.48], i * 4);
  }
  return output;
}
