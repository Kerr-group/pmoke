# Changelog

## v0.5.0 — 2026-09-20

### Breaking changes

- Lock-in quadratures use the peak-amplitude convention (`Xk = b_k`,
  `Yk = a_k` for `s = DC + sum_k [a_k*cos(k*phi) + b_k*sin(k*phi)]`) instead
  of the legacy half-amplitude (`/2`). XY columns double, XY covariance
  quadruples, and `Vm` now reports the true carrier amplitude (previously
  half); `angle`, modulation depth, and all ratio-based quantities are
  unchanged. Migration: rerun analyses with 0.5.0 and never mix pre-0.5.0
  lock-in artifacts with new ones by shape alone — old artifacts keep the
  old values.
- `pmoke analyze` executes its stages in the real order — sensor,
  reference/window preparation, signal, lock-in, phase, MOKE — instead of
  demodulating before the signal readout. Migration: rerun published
  analyses with 0.5.0; do not compare stage outputs across the reorder.
- The signal readout publishes only the combined `signal/mean.png` figure.
  Migration: update any scripts or docs that consume per-channel
  `signal/ch{N}_mean.png` figures (no longer generated, including for a
  single entry).
- Configuration schema v7 is canonical: `[lockin]` gains an explicit
  `estimator` table (`boxcar_legacy` default, joint-harmonic GLS opt-in)
  and an explicit `window` table. Migration: v6 files remain readable and
  migrate with values preserved (preview with `pmoke config migrate`,
  accept explicitly); v5 `[kerr]` files migrate to `[moke]` the same way.

### Changes

- New recorded-only `pmoke noise` workflow (Issue #246). `pmoke noise diagnose`
  plans and publishes a mechanism-agnostic diagnosis of a recorded RAW/CSV
  source (acquisition QC, phase-binned residual variance, residual
  autocorrelation, nuisance observability, identifiability) without requiring a
  physical noise theory. `pmoke noise compare` freezes mode-specific calibration
  artifacts with exact SHA-256 digests from an explicit role plan and
  demodulates a labeled boxcar baseline plus the requested joint-GLS candidates
  on one shared grid, reporting paired residual-scatter evidence, the distinct
  evaluate-lockin mean-block-SD statistic, stratified paired bootstrap, complete
  method x channel x region accounting, and separate result gates. An opt-in
  frozen regime bank with a deterministic center schedule plus
  `pmoke noise replay` (digest/semantic verification and phase -> MOKE -> NPY
  replay) cover prepared conditions. The workflow never performs acquisition,
  never mutates configuration or sources, reports failed/unavailable/unqualified
  legs explicitly, never promotes a default, and a numeric SD improvement cannot
  override unverified fidelity or applicability controls.

- `pmoke analyze` now executes its stages in the real order — sensor,
  reference/window preparation, signal, lock-in, phase, MOKE — instead of
  demodulating before the signal readout. The reference fit and the shared
  output grid are prepared once and reused by the signal readout and the
  lock-in stage. The signal readout publishes only the combined
  `signal/mean.png` figure (per-channel `signal/ch{N}_mean.png` figures are no
  longer generated, including for a single entry) and reports exactly one
  signal plot completion line with the measured elapsed time.

- Lock-in quadratures now use the peak-amplitude convention (`Xk = b_k`,
  `Yk = a_k` for `s = DC + sum_k [a_k*cos(k*phi) + b_k*sin(k*phi)]`) instead
  of the legacy half-amplitude (`/2`). XY columns double, XY covariance
  quadruples, and `Vm` now reports the true carrier amplitude (previously
  half); `angle`, modulation depth, and all ratio-based quantities are
  unchanged. The D8 formulas are untouched — only the `LI*_in` scale
  changed. Artifacts written before this change keep the old values and
  must not be mixed with new ones by shape alone.

- Native joint GLS now prepares one immutable geometry/resource plan per run,
  executes output windows in bounded chunks on the configured worker pool, and
  preserves deterministic output order. `covariance_output=none` validates
  solver covariance per window without retaining unused 12x12 matrices.

- The browser waveform worker now exposes and executes the bounded
  `joint_harmonic_gls` route alongside the legacy boxcar route. Joint requests
  enforce window/model/output budgets before solving, preserve request
  generations across worker restart, and report estimator capabilities; the
  analyzer UI keeps boxcar as the default and adds an explicit estimator
  control.

- Staged publication now records a durable destination-side journal and
  content digest before replacement. Restart recovery distinguishes an
  unpublished/cancelled attempt, a completed generation with uncertain
  durability, and an old-generation restore; pre-commit cancellation leaves
  the published destination untouched.

- Added the native-only `evaluate-lockin` M6 protocol report. It freezes the
  declared reference/rotation/depth/field context, interval, detrending,
  paired-block bootstrap, source fingerprint, and gate version; reports the
  accepted SD-ratio gate, inconclusive insufficient-block cases, and explicit
  unverified known-noise/dynamic/private-control states without promoting a
  default or attributing residual variance to a physical mechanism.

- Comparison method names cannot alias internal staging directories or each
  other on case-insensitive filesystems. Calibration tail metadata is checked
  without overflowing at the integer boundary.

- `pmoke sensor` no longer requires a reference channel: it emits
  `sensor/sensor.csv` (stride-decimated rate/integral series shared with the
  sensor plots) with NPY export, and a zero reference channel is accepted as
  the unspecified sentinel (reference-gated stages still reject it). The
  sensor module moves from `src/lockin/sensor` to top-level `src/sensor`.

- New `pmoke signal` command and `[[signals]]` readout: boxcar-averaged
  scope channels over the lock-in window, emitted as `signal/signal.csv`
  with time-trace plots and NPY export.

- The `[lockin]` setting `signal_channels` is renamed to `channels`
  (schema stays at version 6; existing files using the old key keep working).

- The Kerr analysis concept is renamed to MOKE: configuration schema version 6
  uses the `[moke]` section (version 5 `[kerr]` files migrate with values
  preserved), the command is `pmoke moke` (`pmoke kerr` remains as a deprecated
  alias), and analysis outputs use `moke_results.csv` with `angle` and monitor
  voltage (`Vm`) columns.

- Joint-harmonic GLS gains a pre-pulse auto-calibration source (Issue #258).
  `lockin.estimator.calibration_source` selects `artifact` (default, one
  immutable calibration artifact per lock-in channel) or `prepulse`, which
  derives each channel noise model once per LI run from
  `pulse.background_before` through the calibrate-equivalent pipeline
  (training-block and SCS adequacy gates) and records the derivation digest
  on the LI provenance. No artifact files are created in prepulse mode, and
  existing configurations behave identically.

- `pmoke calibrate build` accepts a direct recorded-data build (Issues
  #269/#270). `--run DIR --channel N` (repeatable; defaults to the run
  signal channels) replaces the TOML `--request` path (retained, mutually
  exclusive) with manifest-backed defaults — same-run reference fit
  (canonically ch3, overridable), seed 0, `block_len` n//40, skip-first-2pct
  with 70/15/15 roles — writing per-channel artifacts under a CWD-anchored
  `calibration/` directory. Reports record the requested and effective
  intervals, exclusions, warnings, and the fully resolved request; trailing
  exclusions warn instead of staying silent. RAW and CSV builds of the same
  source agree byte-for-byte on the canonical sample digest, the TOML
  request gains optional planning floors (schema v1 unchanged), and prepulse
  provenance records the requested window plus the effective range.

- Stage-minimal configuration files are accepted (Issue #264). Absent
  scope/data/pulse/reference/lockin/phase/moke sections fill in with inert
  documented defaults at parse (no schema bump; unknown fields still
  rejected), and missing required items fail with named diagnostics that
  explain how to add them. Sensor and lock-in runs declare per-command
  required sets, and the recorded-data gate no longer requires
  `[instruments.*]`; full-configuration load output is unchanged, and
  `version`/roles/channels stay required.

- Calibration applicability uses an uncertainty-aware reference-frequency
  tolerance (Issue #274). The gate widens from the fixed 1e-9 default
  through max(1e-9, 3*sqrt(u_build^2 + u_apply^2)): the reference fit
  records its uncertainty (fit stderr plus split-segment probe) as u_build
  at artifact build (direct and prepulse paths) and lock-in inference
  supplies u_apply, and the effective tolerance plus the tol-basis recipe
  are recorded on the applicability report and artifact. `pmoke calibrate
  build` accepts an explicit `--frequency-rel-tol` direct override
  (recorded and marked overridden). Behavior without recorded
  uncertainties is unchanged.

- The monitor TUI gains a keybinding-registry foundation and a read-only
  run browser (Issues #275/#276). A global plus per-pane/modal registry
  drives the `?` help overlay and the per-focus footer, with numbered
  pane focus, inspector-local tabs, safe quit-while-running confirmation,
  and a `PMOKE_MOUSE=off` mouse-capture opt-out. A budgeted
  run-directory scan backs the RUNS browser section with `/` filter and
  inspector preview, unified `[/]` history navigation, `Enter` pin, and
  panelized `show`/`raw verify`/`doctor` views without any new execution
  path or writes.

- Staged `pmoke noise compare` publication writes `staging-manifest.json`
  atomically (sibling temp file plus fsync plus rename), so an
  interrupted publish leaves it absent-or-intact instead of torn. Retry
  reclaims an unreadable manifest as stale staging with a warning while
  parseable-but-mismatched manifests stay hard errors.

### CI

- The all-profiles test matrix is split with sccache enabled to cut wall
  time (#259), heavy non-golden noise fixtures are shrunk with identical
  assertions and code paths (#268), and every sccache-enabled job now
  reports sccache stats plus per-step wall-clock timing with zero behavioral
  change (#272).

## v0.4.1 — 2026-08-21

### Changes

- The bilingual homepage now uses a compact, responsive four-stage pulsed-field
  MOKE workflow with stable transitions, aligned signal axes, and clearer
  numerical lock-in, phase-alignment, and Kerr-angle terminology.

### Fixes

- macOS builds now discover the active SDK library path automatically for the
  linker.
- Durable writes to bare relative destinations such as `config.toml` and
  `out.csv` now synchronize the current directory correctly.
- Reference-frequency initialization ignores DC offsets and rejects constant or
  otherwise invalid reference inputs instead of producing a fabricated carrier.
- Explicit CSV time axes are validated for finite, positive, and uniform
  sampling before analysis uses a global interval.
- CSV headers now use standards-compliant escaping, and preflight validation no
  longer truncates an existing destination on failure.
- Ambiguous legacy RAW channel metadata now fails closed instead of silently
  falling back to or overwriting `ch1`.

### Security and CI

- Static export URL checks, CI downloads, and dependency-audit handling were
  hardened without changing the user-facing analysis contract.

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
