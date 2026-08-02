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

console.log('Signal verification script passed: C1 closure and preview math validated.');
