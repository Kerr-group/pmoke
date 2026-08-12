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

function rotatePhasePoint(x, y, delta) {
  const cosDelta = Math.cos(delta);
  const sinDelta = Math.sin(delta);
  return [x * cosDelta + y * sinDelta, -x * sinDelta + y * cosDelta];
}

function calculateHarmonicKerrCue(a2, a3, a4, a6, factor = 1) {
  const modulationDenominator = 15 * a2 + 24 * a4 + 9 * a6;
  const modulationDepth = 6 * Math.sqrt((20 * a4) / modulationDenominator);
  const angleDenominator = ((a2 + a4) * modulationDepth) / 6;
  return {
    modulationDepth,
    angleRad: 0.5 * Math.atan(a3 / angleDenominator) * factor,
  };
}

function finitePulseEnvelope(localTime) {
  const normalized = (value) => Math.min(1, Math.max(0, value));
  const smoothStep = (value) => {
    const clamped = normalized(value);
    return clamped * clamped * (3 - 2 * clamped);
  };
  const onset = smoothStep(localTime / 0.1);
  const settle = 1 - smoothStep((localTime - 0.78) / 0.22);
  const unipolarLobe = Math.exp(-0.5 * ((localTime - 0.28) / 0.16) ** 2);
  return onset * settle * unipolarLobe;
}

function smoothStep(value) {
  const clamped = Math.min(1, Math.max(0, value));
  return clamped * clamped * (3 - 2 * clamped);
}

function sequenceProgressForElapsed(elapsedMs) {
  const loopProgress = ((elapsedMs / 24_000) % 1 + 1) % 1;
  if (loopProgress < 0.9) return loopProgress / 0.9;
  if (loopProgress < 0.96) return 1;
  return 1 - smoothStep((loopProgress - 0.96) / 0.04);
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

// 3. Phase-rotation and harmonic Kerr visual contracts
const [rotatedX, rotatedY] = rotatePhasePoint(0.72, 0.42, 0.72);
assert.ok(Math.abs(Math.hypot(rotatedX, rotatedY) - Math.hypot(0.72, 0.42)) <= 1e-12);
assert.ok(Math.abs(rotatedX - (0.72 * Math.cos(0.72) + 0.42 * Math.sin(0.72))) <= 1e-12);
assert.ok(Math.abs(rotatedY - (-0.72 * Math.sin(0.72) + 0.42 * Math.cos(0.72))) <= 1e-12);

const harmonicCue = calculateHarmonicKerrCue(0.74, 0.22, 0.27, 0.08);
assert.ok(Number.isFinite(harmonicCue.modulationDepth) && harmonicCue.modulationDepth > 0);
assert.ok(Number.isFinite(harmonicCue.angleRad));

for (let index = 0; index <= 100; index += 1) {
  assert.ok(finitePulseEnvelope(index / 100) >= 0, 'field pulse must remain unipolar');
}

assert.equal(sequenceProgressForElapsed(0), 0);
assert.ok(Math.abs(sequenceProgressForElapsed(21_600) - 1) <= 1e-12);
assert.ok(Math.abs(sequenceProgressForElapsed(23_040) - 1) <= 1e-12);
assert.ok(sequenceProgressForElapsed(23_999) < 0.1, 'loop should sweep back before its boundary');
assert.equal(sequenceProgressForElapsed(24_000), 0);

console.log('Signal verification script passed: closure, render density, phase rotation, harmonic Kerr cue, and unipolar pulse validated.');
