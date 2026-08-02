let wasm;
let limits;

self.onmessage = async (event) => {
  const message = event.data;
  if (message?.type === 'init') {
    await initialize(message.basePath ?? '');
    return;
  }
  if (message?.type !== 'run') return;
  const generation = message.generation;
  const startedAt = performance.now();
  try {
    if (!wasm) throw new Error('worker_not_ready: analysis core is not initialized');
    progress(generation, 0.04, 'prepare');
    const source = prepareSource(message.source, message.parameters);
    const signal = source.signal;
    const parameters = message.parameters;
    const estimatedPoints = Math.ceil(signal.length / parameters.strideSamples) * 6;
    if (estimatedPoints > limits.max_total_harmonic_points) {
      throw new Error(
        `output_too_large: increase stride; estimated harmonic points ${estimatedPoints} exceed ${limits.max_total_harmonic_points}`,
      );
    }

    // Leave one paint opportunity for progress and cancellation before synchronous Wasm work.
    await new Promise((resolve) => setTimeout(resolve, 24));

    const harmonics = [];
    let metadata;
    for (let harmonic = 1; harmonic <= 6; harmonic += 1) {
      progress(generation, 0.08 + harmonic * 0.105, `lockin:${harmonic}/6`);
      const packed = wasm.analyze_boxcar_legacy_interleaved(
        signal,
        source.startTimeS,
        source.sampleRateHz,
        parameters.referenceFrequencyHz,
        parameters.referencePhaseRad,
        parameters.halfWindowCycles,
        parameters.strideSamples,
        harmonic,
      );
      const decoded = decodeLockin(packed, limits.lockin_header_values);
      metadata ??= decoded.metadata;
      const rotatedPacked = wasm.rotate_phase_interleaved(
        decoded.x,
        decoded.y,
        parameters.rotationRad,
      );
      const rotated = deinterleavePair(rotatedPacked);
      harmonics.push({ ...decoded, inPhase: rotated.first, outOfPhase: rotated.second });
    }

    progress(generation, 0.78, 'kerr');
    const kerrPacked = wasm.calculate_harmonics_kerr_packed(
      harmonics[1].inPhase,
      harmonics[2].inPhase,
      harmonics[3].inPhase,
      harmonics[5].inPhase,
      parameters.kerrFactor,
    );
    const modulationDepth = kerrPacked[0];
    const kerr = kerrPacked.slice(1);
    const selected = harmonics[parameters.harmonic - 1];
    const magnitude = new Float64Array(selected.x.length);
    const phase = new Float64Array(selected.x.length);
    for (let index = 0; index < selected.x.length; index += 1) {
      magnitude[index] = Math.hypot(selected.x[index], selected.y[index]);
      phase[index] = Math.atan2(selected.y[index], selected.x[index]);
    }
    const responsePacked = wasm.boxcar_response_interleaved(
      metadata.halfWindowS,
      Math.min(3 * parameters.referenceFrequencyHz, 0.5 * source.sampleRateHz),
      256,
    );
    const response = deinterleavePair(responsePacked);

    progress(generation, 0.9, 'decimate');
    const display = {
      input: decimateInput(signal, source.startTimeS, source.sampleRateHz, 1_200),
      lockin: decimateAligned(
        [selected.time, selected.x, selected.y, magnitude, phase, kerr],
        1_200,
      ),
      response: { frequency: response.first, magnitude: response.second },
    };
    const warnings = [];
    if (parameters.halfWindowCycles > 4) {
      warnings.push('long_window');
    }
    if (metadata.outputRateHz < 20 * parameters.referenceFrequencyHz) {
      warnings.push('sparse_output');
    }
    if (source.kind === 'upload') {
      warnings.push('local_input');
    }

    const result = {
      source: {
        kind: source.kind,
        name: source.name,
        samples: signal.length,
        startTimeS: source.startTimeS,
        sampleRateHz: source.sampleRateHz,
      },
      parameters,
      metadata: {
        ...metadata,
        selectedHarmonic: parameters.harmonic,
        modulationDepth,
        elapsedMs: performance.now() - startedAt,
        algorithm: 'boxcar_legacy',
        parity: 'native-exact',
      },
      warnings,
      display,
      export: {
        time: selected.time,
        x: selected.x,
        y: selected.y,
        inPhase: selected.inPhase,
        outOfPhase: selected.outOfPhase,
        magnitude,
        phase,
        kerr,
      },
    };
    const transfer = collectBuffers(result);
    progress(generation, 1, 'complete');
    self.postMessage({ type: 'result', generation, result }, transfer);
  } catch (error) {
    self.postMessage({ type: 'error', generation, message: normalizeError(error) });
  }
};

async function initialize(basePath) {
  try {
    wasm = await import(`${basePath}/wasm/pmoke_web_wasm.js`);
    await wasm.default();
    limits = JSON.parse(wasm.analysis_limits_json());
    self.postMessage({ type: 'ready', limits, build: wasm.build_info() });
  } catch (error) {
    self.postMessage({ type: 'error', generation: 0, message: normalizeError(error) });
  }
}

function prepareSource(source, parameters) {
  if (source.type === 'synthetic') {
    return {
      kind: 'synthetic',
      name: 'Synthetic MOKE signal',
      startTimeS: 0,
      sampleRateHz: parameters.sampleRateHz,
      signal: wasm.generate_analysis_demo(
        parameters.samples,
        parameters.sampleRateHz,
        parameters.referenceFrequencyHz,
        parameters.amplitude,
        parameters.signalPhaseRad,
        parameters.noiseRms,
        parameters.kerrAngleRad,
        parameters.seed,
      ),
    };
  }
  if (source.type !== 'csv' || !(source.buffer instanceof ArrayBuffer)) {
    throw new Error('invalid_source: expected a synthetic source or transferred CSV buffer');
  }
  if (source.buffer.byteLength > limits.max_upload_bytes) {
    throw new Error(`input_too_large: CSV exceeds ${limits.max_upload_bytes} bytes`);
  }
  return parseCsv(source.buffer, source.name ?? 'waveform.csv', parameters.sampleRateHz);
}

function parseCsv(buffer, name, fallbackSampleRateHz) {
  const text = new TextDecoder('utf-8', { fatal: true }).decode(buffer);
  const time = [];
  const values = [];
  let cursor = 0;
  let sawNumericRow = false;
  let skippedHeader = false;
  while (cursor <= text.length) {
    const newline = text.indexOf('\n', cursor);
    const end = newline === -1 ? text.length : newline;
    const line = text.slice(cursor, end).trim();
    cursor = newline === -1 ? text.length + 1 : newline + 1;
    if (!line || line.startsWith('#')) continue;
    const cells = line.split(',').map((cell) => cell.trim());
    if (cells.length > 2) {
      throw new Error(`invalid_csv: expected one or two columns near sample ${values.length + 1}`);
    }
    if (cells.some((cell) => cell.length === 0)) {
      throw new Error(`invalid_csv: empty value near sample ${values.length + 1}`);
    }
    const numbers = cells.map(Number);
    if (numbers.some((value) => !Number.isFinite(value))) {
      if (!sawNumericRow && !skippedHeader) {
        skippedHeader = true;
        continue;
      }
      throw new Error(`invalid_csv: non-finite value near sample ${values.length + 1}`);
    }
    sawNumericRow = true;
    if (numbers.length === 1) {
      values.push(numbers[0]);
    } else {
      time.push(numbers[0]);
      values.push(numbers[1]);
    }
    if (values.length > limits.max_upload_samples) {
      throw new Error(`input_too_large: CSV exceeds ${limits.max_upload_samples} samples`);
    }
  }
  if (values.length < 64) throw new Error('signal_too_short: CSV requires at least 64 samples');
  let sampleRateHz = fallbackSampleRateHz;
  let startTimeS = 0;
  if (time.length > 0) {
    if (time.length !== values.length) {
      throw new Error('invalid_csv: every data row must use the same column count');
    }
    startTimeS = time[0];
    const dt = time[1] - time[0];
    if (!Number.isFinite(dt) || dt <= 0) {
      throw new Error('invalid_time_axis: CSV time must increase');
    }
    const tolerance = Math.max(Math.abs(dt) * 1e-6, Number.EPSILON * 32);
    for (let index = 2; index < time.length; index += 1) {
      if (Math.abs(time[index] - time[index - 1] - dt) > tolerance) {
        throw new Error(`non_uniform_time_axis: time step changes near sample ${index + 1}`);
      }
    }
    sampleRateHz = 1 / dt;
  }
  if (!Number.isFinite(sampleRateHz) || sampleRateHz <= 0) {
    throw new Error('invalid_sample_rate: provide a positive sample rate for one-column CSV');
  }
  return {
    kind: 'upload',
    name,
    startTimeS,
    sampleRateHz,
    signal: Float64Array.from(values),
  };
}

function decodeLockin(packed, headerValues) {
  const outputSamples = packed[0];
  if (!Number.isSafeInteger(outputSamples) || packed.length !== headerValues + outputSamples * 3) {
    throw new Error('invalid_wasm_output: lock-in buffer length mismatch');
  }
  const time = new Float64Array(outputSamples);
  const x = new Float64Array(outputSamples);
  const y = new Float64Array(outputSamples);
  for (let index = 0; index < outputSamples; index += 1) {
    const offset = headerValues + index * 3;
    time[index] = packed[offset];
    x[index] = packed[offset + 1];
    y[index] = packed[offset + 2];
  }
  return {
    time,
    x,
    y,
    metadata: {
      outputSamples,
      sampleRateHz: packed[1],
      outputRateHz: packed[2],
      halfWindowS: packed[3],
      supportS: packed[4],
      estimatedEnbwHz: packed[5],
      firstInputIndex: packed[6],
      lastInputIndex: packed[7],
    },
  };
}

function deinterleavePair(packed) {
  const length = Math.floor(packed.length / 2);
  const first = new Float64Array(length);
  const second = new Float64Array(length);
  for (let index = 0; index < length; index += 1) {
    first[index] = packed[index * 2];
    second[index] = packed[index * 2 + 1];
  }
  return { first, second };
}

function decimateInput(signal, startTimeS, sampleRateHz, maxPoints) {
  const stride = Math.max(1, Math.ceil(signal.length / maxPoints));
  const length = Math.ceil(signal.length / stride);
  const time = new Float64Array(length);
  const value = new Float64Array(length);
  let output = 0;
  for (let index = 0; index < signal.length; index += stride) {
    time[output] = startTimeS + index / sampleRateHz;
    value[output] = signal[index];
    output += 1;
  }
  return { time, value };
}

function decimateAligned(arrays, maxPoints) {
  const length = arrays[0].length;
  const stride = Math.max(1, Math.ceil(length / maxPoints));
  return arrays.map((array) => {
    const output = new Float64Array(Math.ceil(length / stride));
    let cursor = 0;
    for (let index = 0; index < length; index += stride) {
      output[cursor] = array[index];
      cursor += 1;
    }
    return output;
  });
}

function collectBuffers(result) {
  const buffers = new Set();
  const visit = (value) => {
    if (ArrayBuffer.isView(value)) buffers.add(value.buffer);
    else if (value && typeof value === 'object') Object.values(value).forEach(visit);
  };
  visit(result);
  return [...buffers];
}

function progress(generation, fraction, stage) {
  self.postMessage({ type: 'progress', generation, fraction, stage });
}

function normalizeError(error) {
  return error instanceof Error ? error.message : String(error);
}
