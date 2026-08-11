# ADR 0002: Shared bounded analysis core for the waveform tool

- Status: Accepted
- Date: 2026-08-02

## Context

The browser waveform tool must not maintain simplified JavaScript copies of pmoke
analysis algorithms. The current native pipeline combines Rust lock-in processing,
Rust phase rotation, and Python-backed Kerr calculation and plotting. Browser input
also needs strict resource limits, cancellation, and static GitHub Pages delivery.

## Decision

1. `pmoke-analysis-core` owns dependency-light deterministic kernels shared by
   native pmoke and `pmoke-web-wasm`.
2. M4 parity includes legacy boxcar lock-in, phase rotation, and harmonics Kerr.
   Native pmoke calls these kernels directly. Python remains responsible only for
   harmonics Kerr plotting.
   The boxcar kernel uses a compensated rolling window, so working memory is
   proportional to the filter support and output length rather than input length.
3. Historical FIR and synchronous IIR filters are migration-only and are not
   active runtime algorithms. Phase fitting and standard Kerr are excluded from
   browser parity and are labeled as unsupported in the tool and exports.
4. Browser computation runs in a dedicated Worker. The UI cancels by terminating
   the Worker and rejects stale generations. Input controls and selected files are
   preserved across cancellation, timeout, and recoverable failures.
5. Synthetic input is capped at 100,000 samples. CSV input is capped at 16 MiB and
   1,000,000 samples. Six-harmonic output is capped at 1,500,000 points.
6. Worker messages transfer `Float64Array` buffers. Display traces are decimated to
   at most 1,200 points without changing analysis or exported arrays.
7. Generated reference fixtures are CC0-1.0. pmoke source and Wasm artifacts remain
   Apache-2.0.

## Consequences

- Browser parity claims are narrow, testable, and tied to the native call path.
- The browser does not reproduce every analysis path or Python fit.
- Cancellation is immediate because terminating a Worker interrupts synchronous
  Wasm without requiring callbacks inside numerical loops.
- Supporting another browser algorithm requires moving its native implementation
  into the shared core and adding native/Wasm golden fixtures first.
