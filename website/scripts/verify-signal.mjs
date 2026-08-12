import assert from 'node:assert/strict';

const STAGE_DURATION_MS = 4_600;
const STAGES = ['field-pulse', 'lock-in', 'phase-correction', 'kerr-angle'];
const TIME_START_MS = -10;
const TIME_END_MS = 60;
const PULSE_PEAK_MS = 15.8;
const PULSE_END_MS = 42;
const FIELD_PEAK_T = 0.82;
const LI_X_PEAK_MV = -3.2;
const LI_Y_PEAK_MV = 5.4;
const KERR_PEAK_MRAD = -9.8;

function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}

function fieldPulseAtMs(timeMs) {
  if (!Number.isFinite(timeMs) || timeMs <= 0 || timeMs >= PULSE_END_MS) return 0;
  if (timeMs <= PULSE_PEAK_MS) {
    const rise = Math.sin((Math.PI * timeMs) / (2 * PULSE_PEAK_MS));
    return FIELD_PEAK_T * rise ** 1.12;
  }
  const decayProgress = (timeMs - PULSE_PEAK_MS) / (PULSE_END_MS - PULSE_PEAK_MS);
  const decay = Math.cos((Math.PI * decayProgress) / 2);
  return FIELD_PEAK_T * Math.max(0, decay) ** 1.48;
}

function lockInAtMs(timeMs) {
  const envelope = fieldPulseAtMs(timeMs) / FIELD_PEAK_T;
  return [LI_X_PEAK_MV * envelope, LI_Y_PEAK_MV * envelope];
}

function kerrAngleAtMs(timeMs) {
  const envelope = fieldPulseAtMs(timeMs) / FIELD_PEAK_T;
  return KERR_PEAK_MRAD * envelope;
}

function rotatePhasePoint(x, y, delta) {
  const cosDelta = Math.cos(delta);
  const sinDelta = Math.sin(delta);
  return [x * cosDelta + y * sinDelta, -x * sinDelta + y * cosDelta];
}

function sequenceStageForElapsed(elapsedMs) {
  const totalDuration = STAGES.length * STAGE_DURATION_MS;
  const elapsed = clamp(elapsedMs, 0, totalDuration - 1);
  return STAGES[Math.floor(elapsed / STAGE_DURATION_MS)];
}

// The public pulse starts at the 0 ms trigger, remains unipolar, peaks once,
// and returns to baseline before the displayed 60 ms time axis ends.
assert.equal(fieldPulseAtMs(-10), 0);
assert.equal(fieldPulseAtMs(0), 0);
assert.ok(fieldPulseAtMs(PULSE_PEAK_MS) > 0);
assert.equal(fieldPulseAtMs(PULSE_END_MS), 0);
assert.equal(fieldPulseAtMs(TIME_END_MS), 0);
let decayPrevious = null;
for (let index = 0; index <= 700; index += 1) {
  const timeMs = TIME_START_MS + ((TIME_END_MS - TIME_START_MS) * index) / 700;
  const value = fieldPulseAtMs(timeMs);
  assert.ok(Number.isFinite(value) && value >= 0, `field pulse must remain unipolar at ${timeMs} ms`);
  if (timeMs >= PULSE_PEAK_MS && timeMs <= PULSE_END_MS) {
    if (decayPrevious !== null) {
      assert.ok(value <= decayPrevious + 1e-9, `field pulse must decay after its peak at ${timeMs} ms`);
    }
    decayPrevious = value;
  }
}

// Lock-in and Kerr traces share the field-pulse time support and expose units.
assert.ok(lockInAtMs(0).every((value) => Math.abs(value) <= 1e-12));
assert.ok(Math.abs(kerrAngleAtMs(0)) <= 1e-12);
assert.ok(Math.abs(lockInAtMs(PULSE_PEAK_MS)[0] - LI_X_PEAK_MV) <= 1e-12);
assert.ok(Math.abs(lockInAtMs(PULSE_PEAK_MS)[1] - LI_Y_PEAK_MV) <= 1e-12);
assert.ok(Math.abs(kerrAngleAtMs(PULSE_PEAK_MS) - KERR_PEAK_MRAD) <= 1e-12);

// Phase correction preserves vector magnitude and removes the quadrature term.
const rawX = LI_X_PEAK_MV;
const rawY = LI_Y_PEAK_MV;
const phase = Math.atan2(rawY, rawX);
const [correctedX, correctedY] = rotatePhasePoint(rawX, rawY, phase);
assert.ok(Math.abs(Math.hypot(correctedX, correctedY) - Math.hypot(rawX, rawY)) <= 1e-12);
assert.ok(Math.abs(correctedY) <= 1e-12);
assert.ok(correctedX > 0);

// The four visible stages advance automatically, while stage replay can
// select any one of them without reintroducing acquisition or harmonic steps.
assert.equal(sequenceStageForElapsed(0), 'field-pulse');
assert.equal(sequenceStageForElapsed(STAGE_DURATION_MS), 'lock-in');
assert.equal(sequenceStageForElapsed(STAGE_DURATION_MS * 2), 'phase-correction');
assert.equal(sequenceStageForElapsed(STAGE_DURATION_MS * 3), 'kerr-angle');
assert.equal(sequenceStageForElapsed(STAGE_DURATION_MS * 4), 'kerr-angle');

console.log('Signal verification passed: 0 ms unipolar pulse, shared time support, phase correction, and four-stage workflow validated.');
