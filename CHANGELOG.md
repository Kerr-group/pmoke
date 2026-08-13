# Changelog

## Unreleased

### Changes

- The bilingual homepage now uses a compact, responsive four-stage pulsed-field
  MOKE workflow with stable transitions, aligned signal axes, and clearer
  numerical lock-in, phase-alignment, and Kerr-angle terminology.

## v0.4.0 — 2026-08-12

### Breaking changes

- The default pmoke build now enables direct TCP/IP, direct GPIB/VISA, and
  Prologix TCP/serial transports. macOS builds that exclude GPIB must use
  `--no-default-features --features hw-core,hw-prologix-tcp,hw-prologix-serial`.
- The legacy `hw` Cargo feature has been removed. Use `hw-gpib` for a
  direct-GPIB build or the default feature set for the complete Linux/Windows
  build.
- Configuration schema v5 is now canonical. The active lock-in LPF is
  `boxcar_legacy`; historical FIR/IIR kinds and their algorithm-specific
  fields are migration-only and require an explicit compatibility decision.

### Changes

- The bilingual website now presents pmoke as a reproducible pulsed-field
  MOKE workflow from field-pulse capture through phase-aware lock-in analysis
  and Kerr-angle extraction.
- The monitor's primary output is now an Activity view with explicit live,
  paused, history, and unseen-event states plus logical warning/error counts.
- Monitor child commands use structured JSONL events internally; direct CLI
  output retains the concise human-readable renderer.
- Carriage-return progress updates are coalesced instead of filling the Activity
  history, and elapsed event times are shown when the terminal is wide enough.
- Activity events now use clear, compact status labels and structured tree
  fields; `PMOKE_MOTION=full|reduced|off` controls calm live motion without
  animated arrival sweeps.
- Structured progress identities update in place and transition to completion
  without increasing event or unread counts.
- Timeline states use static `DONE/RUN/NEXT/FAIL/STOP` labels; only the current
  state's color pulses, and failed or skipped progress is explicitly terminated.
- Analysis commands are safely rerunnable as new transactional generations, including when analysis-only config values change.
- Each published analysis stores its own `analysis/config.source.toml` and `analysis/config.resolved.toml`; root config snapshots remain immutable acquisition provenance.
- Analysis manifest schema 3 records generation numbers, config and acquisition checksums, the published stage, and stage-scoped config fingerprints.
- `phase` and `kerr` reject stale upstream results with an explicit command to rerun, while standalone `reference` and `sensor` create diagnostic-only manifests when needed.
- Canonical NPY export is idempotent and replaces only generated NPY artifacts transactionally.

### Fixes

- Diagnostic and NPY generations now keep `run.toml` synchronized with the published analysis generation; diagnostic configs are stored separately without replacing numerical-analysis provenance.
- Analysis attempts that fail while reading waveform input are recorded, and both source and resolved analysis configs are checksum-protected.
- Reanalysis continues with a warning when only an acquisition config snapshot checksum is stale; RAW channel sizes and checksums remain mandatory, while `raw verify` stays strict.
- Standalone `reference` and `sensor` diagnostic plots no longer require `li` to have created an analysis manifest first.

## v0.3.0 — 2026-07-13

### Breaking changes

- New acquisitions use the canonical `acquisition/` layout and analyses use `analysis/`.
- Waveform CSV output is fixed at `acquisition/waveforms/waveform.csv`.
- `fetch --out` has been removed; use `export csv --output FILE` for a custom CSV destination.
- Lock-in, phase-rotated, Kerr, NPY, plot, and debug artifacts now live under `analysis/`.
- Canonical plots are fixed under `analysis/plots/`; `plot.output_dir` is deprecated, accepted only for config compatibility, and ignored.
- Standalone screenshot capture adds a screenshot only to an existing completed canonical acquisition.
- Run-mutating commands are serialized and publish acquisition or analysis directories transactionally.

### Compatibility

- Config versions 1–4 remain readable and can be migrated to the latest executable schema when their recorded data is sufficient. Legacy LPF kinds that cannot be represented by the active runtime produce a migration diagnostic.
- Legacy `raw_waveform/`, `raw.csv`, legacy analysis CSV names, and `analysis_npy/` remain supported as fallback inputs.
- Config migration remains preview-only by default and requires explicit acceptance for lossy changes.

### Fixes

- PowerShell completion is loaded from a standalone script so its required `using namespace` statements no longer invalidate an existing profile.
- The TUI calls the cross-stage `process` and `auto` workflow group `END-TO-END`; acquisition-only `automeasure` remains under `ACQUISITION`.
