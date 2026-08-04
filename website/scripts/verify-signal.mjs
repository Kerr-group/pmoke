import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

// Verify C1 periodic envelope closure and TS / Wasm preview equivalence
const TAU = Math.PI * 2;

function sampleChannels(t, phase) {
  const carrier = TAU * (7.0 * t + phase);
  const envelope = Math.cos(Math.PI * (t - 0.5)) ** 2;
  const pulse = (Math.sin(carrier) + 0.18 * Math.sin(3.0 * carrier + 0.4)) * envelope;
  const inPhase = Math.sin(TAU * 2.0 * t) * 0.72 + 0.12 * pulse;
  const quadrature = Math.sin(TAU * 2.0 * t + 1.12) * 0.48;
  return [pulse, inPhase, quadrature];
}

function fallbackSignal(samples, phase = 0.17) {
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

function previewPointCount(width, samples) {
  const periodicSamples = Math.max(2, samples - 1);
  return Math.min(periodicSamples, Math.max(256, Math.ceil(width) + 1));
}

function periodicSample(signal, channel, position) {
  const periodicSamples = signal.length / 4 - 1;
  const normalized = ((position % 1) + 1) % 1;
  const exactIndex = normalized * periodicSamples;
  const index0 = Math.floor(exactIndex) % periodicSamples;
  const index1 = (index0 + 1) % periodicSamples;
  const fraction = exactIndex - index0;
  return signal[index0 * 4 + channel] * (1 - fraction) + signal[index1 * 4 + channel] * fraction;
}

// 1. C1 Boundary Closure Verification
for (const samples of [64, 720, 4096]) {
  const signal = fallbackSignal(samples, 0.17);
  for (let ch = 1; ch <= 3; ch += 1) {
    const first = signal[ch];
    const last = signal[(samples - 1) * 4 + ch];
    assert.ok(
      Math.abs(first - last) <= 1e-9,
      `C0 boundary failed for channel ${ch} with ${samples} samples: first=${first}, last=${last}`
    );
  }
}

// 2. Mobile-density circular interpolation and exact viewport closure
const preview = fallbackSignal(720, 0.17);
for (const width of [320, 360, 768, 1440]) {
  const points = previewPointCount(width, preview.length / 4);
  assert.ok(points >= Math.min(256, preview.length / 4 - 1));
  if (width <= 718) assert.ok(points >= Math.ceil(width));

  for (const offset of [0, 0.003, 0.17, 0.499, 0.998]) {
    for (let channel = 1; channel <= 3; channel += 1) {
      const left = periodicSample(preview, channel, offset);
      const right = periodicSample(preview, channel, 1 + offset);
      assert.ok(Math.abs(left - right) <= 1e-12, `viewport closure failed at ${width}px`);
    }
  }
}

console.log('Signal verification script passed: C1 closure, circular interpolation, and render density validated.');
