# Sensor Output + Reference Decoupling: Requirements (v0.2)

Status: Draft v2 (2026-09-13)
Audience: pmoke maintainer and implementers
Related: `docs/project/requirements.md` (stable requirements. This document holds the
working requirements for this change; reflect the delta into the stable requirements
once finalized.)

Two changes: (1) emit sensor rate/integral time series as files, not just console
tables and plots; (2) remove the reference-fit prerequisite so the sensor stage runs
without a reference channel. No code is copied from elsewhere.

---

## 1. Background assets

| Asset | Role | Treatment in this change |
| --- | --- | --- |
| `src/lockin/sensor/mod.rs` (`run`, `run_sensor`, `SensorOutput`) | Sensor computation | Split f_ref-free computation from grid striding; add CSV output |
| `src/lockin/mod.rs` (`run_li`) | Current ref→sensor→lockin orchestrator | Reorder to sensor→ref→lockin; stride sensor series internally |
| `src/commands/sensor.rs`, `src/commands/li.rs` | Standalone commands | `pmoke sensor` drops the reference fit; `pmoke li` keeps it |
| `src/lockin/sensor/` | Misplaced module (sensor is not lock-in) | Move to top-level `src/sensor/`; update 4 import sites + bench import |
| `src/config/validation.rs` (`ValidationTarget::Sensor`) | Target gating | Drop `reference_roles`; keep sensor roles/metadata/input checks |
| `src/lockin/stride.rs` (`li_stride_time`, `li_stride_2d`) | Grid alignment | Reused by li/signal stages on full-rate sensor series |
| `src/lockin/provenance.rs`, `src/config/paths.rs`, `src/commands/export/npy.rs` | Run-dir contract | New `sensor/` CSV artifacts (`sensor` kind), NPY targets |
| `website/` analyzer, `crates/pmoke-web-wasm` | Web surface | Out of scope (demo chain unchanged) |
| `docs/project/requirements.md` | Stable requirements | Reference only. Reflect the delta once finalized |

## 2. Findings (F)

- F1 `run_sensor` uses `f_ref` only for stride decimation to the li grid
  (`li_stride_time`, `li_stride_2d`). Background averaging, series integration,
  and scaling are f_ref-free.
- F2 The sensor stage currently emits no data files: console tables plus raw and
  integral combined plots only.
- F3 `run_sensor` has exactly two callers: `run_li` and the standalone sensor
  wrapper (which ref-fits first). The refactor surface is contained.
- F4 Downstream li/signal/moke chains consume sensor series on the trimmed li
  grid; they can stride full-rate series themselves with the existing helpers.

## 3. Decisions (D)

- D1 `run_sensor` no longer takes `f_ref`. It returns full-rate rate/integral
  series, writes one combined `sensor/sensor.csv`
  (time + `{label} rate ({unit}/s)` + `{label} integral ({unit})` columns), and
  keeps its console tables.
- D2 Sensor CSV and sensor plots share one grid: every `stride_samples`-th sample
  over the full range (no window trim, no reference needed). File size stays
  proportional to the existing stride decimation instead of full raw rate.
- D3 Execution order becomes sensor → reference → lock-in everywhere:
  `run_li` internally runs the sensor part first, then the reference fit, then
  demodulation; standalone `pmoke sensor` runs only the sensor part.
  `pmoke li` still requires a reference (demodulation fundamentally needs `f_ref`).
- D4 `run_li` strides the full-rate sensor series onto the li grid itself with
  `li_stride_2d`; its return shape and all downstream consumers are unchanged.
- D5 `ValidationTarget::Sensor` drops `reference_roles` (keeps oscilloscope,
  sensor roles/metadata, analysis input). Standalone sensor works with no
  reference channel or recording. Load-time validation treats
  `reference_ch = 0` as the unspecified sentinel so a reference-free file
  loads; Li/Reference targets still reject it via `reference_roles`.
- D6 Provenance: `sensor/` CSVs register as `sensor`-kind artifacts with output
  checksums and NPY targets, mirroring the `signal` pattern. `published_through`
  is unaffected (sensor runs first; li/phase/moke dominate). Sensor stage keeps
  its diagnostics-style manifest entries.
- D7 No config schema change (no new keys), no template change, no Web/WASM change.
- D8 `src/lockin/sensor/` moves to top-level `src/sensor/` (mod, pulse_calculator,
  both plot glues, pytools). `lockin::stride` stays: the sensor stride helper
  becomes local (plain every-Nth, no window). `include_str!` relative paths and
  Python module names are unchanged by the move.
- D9 `sensor.csv` length is deterministic: `floor((n - 1) / stride) + 1` rows where
  `n` is the fetched sample count and `stride` is `lockin.stride_samples`
  (template default 100, so ~10k rows per 1M samples). No header beyond the single
  header row; grid and plots share the same decimation.

## 4. Open questions (O)

- None.

## 5. Milestones (M)

- M1 Core split: f_ref-free `run_sensor` (full-rate + `sensor.csv`), `run_li`
  internal reorder with self-striding, sensor target validation update, unit tests.
- M2 Commands/manifest: standalone `pmoke sensor` without reference, analyze
  integration, `sensor` artifacts + NPY wiring, monitor/show touch-ups if needed.
- M3 Generated docs + full matrix (Rust, pytools, fmt/clippy, pnpm check, e2e).
- M4 Reviews, PR, stable-requirements reflection, changelog.
