# Changelog

## Unreleased — v0.3.0

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
