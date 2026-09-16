# Signal Readout ([[signals]]): Requirements (v0.2)

Status: Draft v2 (2026-09-13)
Audience: pmoke maintainer and implementers
Related: `docs/project/requirements.md` (stable requirements. This document holds the
working requirements for this change; reflect the delta into the stable requirements
once finalized.)

Add a raw-signal readout: boxcar-averaged scope channels over the same window the
lock-in uses, on the same output time grid, so signal means align sample-for-sample
with lock-in outputs. Multiple signal entries are assumed from the start. No code is
copied from elsewhere.

---

## 1. Background assets

| Asset | Role | Treatment in this change |
| --- | --- | --- |
| `crates/pmoke-analysis-core/src/lockin.rs` (`Geometry`, `legacy_boxcar_weights`) | Window/weight definition | Expose a `boxcar_mean` reusing the same geometry and trapezoidal weights |
| `src/lockin/mod.rs` (`run_li`), `src/commands/analyze.rs`, `src/commands/li.rs` | Analysis pipeline | New `signal` stage after `li`; new `pmoke signal` command re-running `run_li` like `li.rs` |
| `src/config/schema.rs`, `crates/pmoke-config-core/src/model.rs`, `xtask/src/config_schema.rs` | Config contract owners | New `[[signals]]` entries; schema stays at v6 (optional section) |
| `src/utils/channels.rs` (`build_channel_list`) | Fetch channel list | Include signal channels; cross-role duplicates stay an error |
| `src/lockin/provenance.rs`, `src/config/paths.rs`, `src/commands/export/npy.rs` | Run-dir contract | New stage dir, artifacts, manifest entries, NPY targets |
| `src/lockin/sensor/pytools/sensor_raw_plot.py`, `sensor_plot_dir` paths | Plot precedent | New `signal_plot.py` mirroring the sensor per-channel + combined pattern |
| `website/` analyzer + specs, `crates/pmoke-web-wasm` | Web surface | Time-trace display, CSV export columns, validator sample |
| `docs/project/requirements.md` | Stable requirements | Reference only. Reflect the delta once finalized |

## 2. Decisions (D)

- D1 New optional `[[signals]]` entries, shaped like `[[sensors]]`:
  `channel` (u8, required), `label` (string, required), `unit` (string, required).
  No `scale`/`factor`: the readout is a mean of the raw waveform, no conversion.
  Example: `channel = 4, label = "DC", unit = "V"`.
- D2 Averaging window is exactly the lock-in support: `±half_window_s` around each
  stride center with the same trapezoidal weights and edge interpolation, where
  `half_window_s = half_window_cycles / f_ref`. Output grid = lock-in stride centers.
- D3 Channel overlap is forbidden: a channel listed in `[[signals]]` must not appear
  in `[lockin].channels`, `[[sensors]]`, or `[reference]`. Violation is a validation
  error, consistent with the existing cross-role duplicate policy. Fetch acquires
  each channel once.
- D4 Stage placement: in `pmoke analyze`, the `signal` stage runs immediately after
  `run_li` returns and reuses its trimmed stride grid (`t_stride`) plus `f_ref`.
  The standalone `pmoke signal` command re-runs `run_li` from fetched data (same
  pattern as `pmoke li`) and then computes the means. Signal results feed nothing
  downstream (phase/moke do not consume them).
  **Amended 2026-09-16 (Architecture decision, superseded):** the native analyze
  workflow executes sensor → reference preparation → signal → lock-in → phase →
  MOKE. The reference fit and the immutable analysis-window/output-grid
  preparation run once after the sensor stage, before signal and lock-in, and are
  reused by both; lock-in demodulation is not executed early, and the sensor and
  reference work is not repeated. Standalone `pmoke sensor` remains
  reference-independent; its full-rate series are aligned to the prepared
  downstream grid without recomputation. The standalone `pmoke signal` command
  reuses the same preparation and order while keeping its existing artifacts.
- D5 Invalidation: re-running `li` invalidates `signal` outputs; re-running `signal`
  invalidates only `signal` plus `export_npy`. No further downstream propagation.
- D6 Scope includes CSV output, time-trace plot, NPY export, manifest/provenance,
  config template + generated references, and Web/WASM display from the start.
- D7 Plot design for multiple signals: one panel per signal entry (units differ per
  entry, so no overlay). Mosaic grows with entry count (`A`, `AB`, `ABC`, …) at
  6 inches per panel, mirroring the sensor plots. Outputs: per-channel
  `signal/ch{N}_mean.png` plus combined `signal/mean.png`. Axis labels come from
  each entry's `label`/`unit`. Combined angle-vs-signal loop plots are out of scope
  (deferred to a later analysis step).
  **Amended 2026-09-16 (Architecture decision, superseded):** the combined figure
  `signal/mean.png` is the only signal plot. Per-channel
  `signal/ch{N}_mean.png` figures are never rendered or written, including when a
  single signal entry is configured; the run emits exactly one signal plot
  completion line carrying the measured elapsed time. The panels, mosaic growth,
  axis labels, and "no combined angle-vs-signal loop plot" contract are
  unchanged.
- D8 CSV: a single combined file (`signal/signal.csv`) mirroring `moke/moke.csv`:
  time, sensor rate/integral columns, then one mean column per entry in
  `[[signals]]` order with headers `Ch{N} {label} mean ({unit})`. A combined
  file keeps one artifact kind (`signal`) so the manifest column-set guard,
  NPY export, and web CSV handling stay uniform. Per-entry files would give
  each column set a different header and trip the guard.

## 3. Open questions (O)

- None. O1 (plot timing) resolved as stage time-trace per D7.

## 4. Milestones (M)

- M1 Config: `[[signals]]` parse/validate (range 1..=8, intra-list duplicates,
  cross-role collision as error), template, core model, unit tests.
- M2 Core + stage: `boxcar_mean` in analysis-core with fixtures, `signal` stage
  (`pmoke signal` + analyze integration), combined CSV + manifest + provenance,
  fetch list.
- M3 Plot (`signal_plot.py`, per-channel + combined) + NPY + Web/WASM + generated
  docs; pnpm check and analyzer e2e.
- M4 Reviews, PR, stable-requirements reflection, changelog.
