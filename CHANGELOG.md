# Changelog

## Unreleased

### Changes

- Analysis commands are safely rerunnable as new transactional generations, including when analysis-only config values change.
- Each published analysis stores its own `analysis/config.source.toml` and `analysis/config.resolved.toml`; root config snapshots remain immutable acquisition provenance.
- Analysis manifest schema 2 records generation numbers, config and acquisition checksums, the published stage, and stage-scoped config fingerprints.
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

- Config versions 1–3 remain readable and can be migrated to the latest executable schema when their recorded data is sufficient.
- Legacy `raw_waveform/`, `raw.csv`, legacy analysis CSV names, and `analysis_npy/` remain supported as fallback inputs.
- Config migration remains preview-only by default and requires explicit acceptance for lossy changes.

### Fixes

- PowerShell completion is loaded from a standalone script so its required `using namespace` statements no longer invalidate an existing profile.
- The TUI calls the cross-stage `process` and `auto` workflow group `END-TO-END`; acquisition-only `automeasure` remains under `ACQUISITION`.
