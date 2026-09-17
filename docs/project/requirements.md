# Project Requirements

This document defines the stable product, compatibility, and security
boundaries of `pmoke`. It changes only when a product contract changes: it is
not a roadmap, an Issue tracker, or a chronological progress diary. Durable
goals belong in GitHub Issues, and implementation state belongs in linked pull
requests.

## Product scope

`pmoke` is a Rust 2024 command-line and terminal application for pulsed-field
magneto-optical Kerr effect measurements. It connects supported acquisition
hardware, preserves reproducible run artifacts, and performs deterministic
analysis. The repository also ships a browser-facing WASM analysis/configuration
boundary and a bilingual static documentation site.

The following surfaces are part of the product contract:

- The Rust workspace, locked dependency graph, CLI, terminal UI, configuration
  validation/migration, acquisition workflows, analysis stages, and artifact
  export.
- The feature-gated hardware transports: direct TCP/IP, Linux GPIB, Windows
  USBTMC/VISA, and Prologix TCP/serial where the selected platform and feature
  profile support them.
- The dependency-light analysis and configuration cores shared with the browser
  adapter. Hardware access, native drivers, and the embedded Python bridge do
  not cross the browser/WASM boundary.
- The English/Japanese static site exported below the `/pmoke/` project path,
  including documentation, search, machine-readable resources, configuration
  validation, and waveform analysis.

## Compatibility requirements

- `Cargo.lock` remains intentional and reviewable. Source, dependency,
  feature, license, and platform changes are reviewed together.
- Configuration schema version 6 is canonical. Versions 1–5 remain readable
  when their structure is recognized; a version 5 `[kerr]` section is aliased
  to `[moke]` with values preserved. Historical LPF kinds that cannot be
  represented by the active runtime produce an explicit migration diagnostic;
  they are never silently changed to `boxcar_legacy`. Migration is
  preview-only by default; potentially behavior-changing migration requires
  explicit acceptance. A validated legacy `[timebase]` is preserved when a
  CSV without a recorded time axis still requires it, even when that prevents
  rendering a v6 resolved snapshot.
- The active lock-in LPF is `boxcar_legacy` only. The configuration keeps the
  `kind` discriminator so a future LPF can be added as a separately specified
  schema and runtime contract. The `[[signals]]` readout averages raw channels
  over that same lock-in support window on the lock-in output grid and
  publishes one combined plot (`signal/mean.png`); per-channel signal figures
  are not part of the artifact contract. A channel listed in `[[signals]]`
  must not overlap sensor, reference, or lock-in channels.
- The native analysis workflow executes sensor, shared reference/window
  preparation, signal, lock-in, phase, and MOKE in that order. The preparation
  (reference fit plus the immutable stride/output grid) is computed once after
  the sensor stage and reused by signal and lock-in; lock-in demodulation is
  not executed early, and no stage repeats another stage's work. The numerical
  averaging recipe, support, stride, timestamps, and published values are
  unchanged by this ordering.
- The sensor stage needs no reference channel: it writes the stride-decimated
  `sensor/sensor.csv` (time plus `{label} rate` and `{label} integral` columns)
  and returns full-rate series that the lock-in stage strides onto its own
  grid. A zero reference channel is the unspecified sentinel, rejected by
  reference-gated stages.
- Canonical acquisition and analysis artifacts use the versioned run layout,
  immutable configuration snapshots, checksums, and transactional publication.
  Legacy inputs remain compatibility behavior only where the current changelog
  and documentation explicitly describe them.
- Generated CLI/configuration references and schema output are owned by
  `xtask`; source changes that affect them must regenerate and review the
  paired English/Japanese outputs.
- The machine-resource manifest remains schema 1. Additive fields are allowed
  when consumers ignore unknown fields; a breaking shape change requires a
  schema increment. Exact paths, field names, units, enum values, and checksums
  are public contracts.
- English and Japanese documentation remain paired for user-visible behavior.
  Static routes, workers, assets, canonical URLs, and tests must preserve the
  `/pmoke/` base path.
- Linux/Windows full transport builds, macOS builds without direct GPIB, and
  analysis-only builds are distinct supported profiles. A WSL build is a
  Linux-native validation result, not native Windows MSVC/VISA validation.
- The repository's pinned Nix development shell is the installation authority
  for agent and contributor work on Nix-managed workstations. End-user
  installation outside Nix remains documented separately in `README.md`.

## Security boundaries

- Build and test commands must not contact, trigger, fetch from, screenshot, or
  otherwise operate live instruments unless a separate hardware action is
  explicitly authorized. Native-link success is not evidence of hardware
  reachability.
- Browser tools run within their documented worker/WASM limits. They do not
  provide native driver, filesystem, Python-package, credential, or hardware
  reachability checks; those remain native `pmoke doctor` responsibilities.
- Run artifacts preserve provenance and integrity through isolated directories,
  immutable source/resolved configuration snapshots, checksums, and
  transactional publication. User-controlled paths and labels must remain
  within their intended artifact boundary. Comparison method labels must not
  alias reserved staging directories or each other on supported filesystems;
  validate them before creating output. Calibration metadata comparisons must
  preserve their ordering contract across the full representable integer range
  without overflow or a process panic.
- Public branches, Issues, PRs, logs, screenshots, and generated reports must
  contain only redacted, reproducible evidence. Never commit credentials,
  private endpoints, raw captures, personal data, machine-specific paths, or
  unreviewed logs.
- Dependency and CI changes preserve locked inputs, integrity-checked action
  pins, least-privilege permissions, advisory/license/source checks, and
  secret-scanning expectations. Vulnerability reports follow `SECURITY.md` and
  must never be filed with exploit details in a public Issue.

- Scientific estimator comparisons keep the reference frequency, rotation,
  modulation depth, field factor, evaluation interval, detrending policy,
  paired block length, confidence method, and gate version explicit in the
  request/report. The accepted benefit gate is candidate/baseline SD ratio
  `<= 0.97` with the paired confidence-interval upper endpoint `< 1`; too few
  independent blocks or an unconfirmed retained physical feature/fidelity
  tolerance produces `inconclusive`, not a pass. A completed negative result
  remains a completed report, and residual variance is not assigned a
  physical noise mechanism. Evaluation reports do not change the estimator
  default or authorize publication of private measurement evidence.

## Change acceptance

Every durable public change has one Issue with purpose, scope, non-goals,
acceptance criteria, compatibility impact, and security impact. Implementation
starts in a linked normal PR that is open for review from the beginning. Review
1 covers the Issue, design, API or public contract, compatibility, security,
and rollback decision. Review 2 covers the complete staged diff, tests,
generated outputs, and public-artifact hygiene.

A change is ready for merge only when its acceptance criteria and relevant
validation evidence are complete, known limitations are stated, the current PR
head is up to date, required and relevant checks pass on that head,
conversations are resolved, and no blocking review remains. After merge,
deployment or release verification is completed when applicable before the
Issue is closed. User-visible release changes are recorded in `CHANGELOG.md`;
temporary planning and private deliberation stay out of the repository.
