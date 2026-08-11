---
name: pmoke-maintenance
description: Review and maintain this pmoke repository across its Rust workspace, hardware transport features, embedded Python, WASM/browser boundary, generated documentation, bilingual Next.js site, benchmarks, and CI. Use when changing or validating source, tests, Cargo manifests or lockfiles, xtask-generated references, website code/content, Python tools, workflows, or repository guidance. Do not use for unrelated generic work or live instrument operations.
---

# Pmoke maintenance

Use this skill for repository changes that cross pmoke's Rust, hardware,
analysis, Python, WASM, documentation, website, benchmark, or CI boundaries.
Read the root `AGENTS.md` first; it is the repository policy and takes
precedence over this workflow.

## Repository map

- `src/`: CLI, TUI, configuration, run artifacts, acquisition, analysis,
  plotting, and the embedded Python bridge.
- `crates/instruments/`, `crates/gpib-rs/`, `crates/prologix-rs/`: instrument
  models and transport implementations.
- `crates/pmoke-analysis-core/`, `crates/pmoke-config-core/`,
  `crates/pmoke-web-wasm/`: deterministic shared cores and browser adapter.
- `xtask/`: owner of generated CLI/configuration references and schema files.
- `website/`: bilingual static Next.js/Fumadocs site, workers, search, AI
  exports, and browser quality gates.
- `scripts/`: Python benchmark and comparison utilities.
- `.github/workflows/`: CI, security, docs release, deployment, lockfile, and
  performance contracts.

Treat `Cargo.toml`/`Cargo.lock`, source definitions, `xtask`, and the website's
tracked generated outputs as a dependency chain. Do not hand-edit generated
references. Keep `website/content/docs/en` and `website/content/docs/ja`
paired, and keep all public routes below `/pmoke/`.

## Operating procedure

### 1. Triage

Run:

```bash
git status --short --branch
git remote -v
git log -5 --oneline --decorate
```

Preserve unrelated work. Identify the changed crate/site area, public contract,
feature profiles, generated outputs, native dependencies, and whether the task
could contact hardware or write external state.

### 2. Inspect before editing

Read the owning implementation, all direct consumers, relevant tests, Cargo
features, CI workflow, and generated-file producer. For a public CLI or config
change, inspect `src/cli.rs`, `src/docs.rs`, `src/config/`, `xtask/`, generated
references, and both locale trees. For shared analysis or browser behavior,
inspect all three shared crates plus the worker and browser tests.

### 3. Implement minimally

Use `apply_patch` for source edits. Keep public errors and machine-readable
schemas stable unless the request explicitly changes them. Update tests at the
same boundary as behavior. When the source of a generated file changes, run:

```bash
cargo xtask docs-export
```

Review the generated diff and commit it only when it is the expected output.

Never add credentials, raw captures, local configs, experiment artifacts,
private URLs, or machine-specific paths. Use dummy transports and temporary
directories for tests. Do not run trigger/fetch/screenshot/auto commands or
`doctor --probe-fetch` against live equipment without explicit authorization.

## Validation lanes

Select the smallest complete set of lanes, then run the broad CI-equivalent
checks when the change is cross-cutting.

### Rust, feature, and Python lane

```bash
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets --all-features
cargo test --locked --workspace --lib --bins --tests --examples --all-features
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
PYTHONDONTWRITEBYTECODE=1 python -m unittest discover -s src/kerr/pytools -p 'test_*.py'
PYTHONDONTWRITEBYTECODE=1 python -m unittest discover -s scripts -p 'test_*.py'
```

The all-feature test lane needs native GPIB/VISA libraries on the target
platform. For supported macOS builds, also use the no-GPIB bundle:

```bash
cargo check --locked -p pmoke --all-targets --no-default-features \
  --features hw-core,hw-prologix-tcp,hw-prologix-serial
cargo test --locked -p pmoke --all-targets --no-default-features
cargo clippy --locked -p pmoke --all-targets --no-default-features -- -D warnings
```

For transport/core changes, cover the no-default profiles too:

```bash
cargo test --locked -p pmoke-analysis-core -p pmoke-config-core -p pmoke-web-wasm
cargo test --locked -p instruments --all-targets --no-default-features
cargo test --locked -p prologix-rs --all-targets --no-default-features
```

Use the exact feature matrix in `.github/workflows/ci.yml` for GPIB, Prologix
TCP/serial, Windows VISA, macOS, and analysis-only changes. A `cargo check`
pass is not a native link or live hardware pass.

### Generated docs and WASM lane

For CLI, configuration, schema, or public Rust type changes:

```bash
cargo test --locked -p xtask
cargo xtask docs-export
git diff --exit-code -- \
  website/generated website/public/config.schema.json \
  website/content/docs/en/cli website/content/docs/ja/cli \
  website/content/docs/en/configuration/reference.mdx \
  website/content/docs/ja/configuration/reference.mdx
```

For shared browser behavior, add `wasm32-unknown-unknown` checks for
`pmoke-analysis-core` and `pmoke-web-wasm`, then build the site WASM with the
pinned wasm-pack version from `website/README.md`.

### Website lane

From `website/`:

```bash
pnpm install --frozen-lockfile
pnpm check
pnpm test:licenses
pnpm build
pnpm exec playwright install --with-deps chromium
pnpm test:e2e
```

Use `pnpm start` to inspect the exported `/pmoke/` tree. For release-sensitive
changes, run the cross-browser release project and Lighthouse gate as well.
Preserve no-JavaScript readability, keyboard/accessibility behavior, search
fallbacks, worker failure recovery, bounded input limits, AI-resource
contracts, and English/Japanese parity.

### Security and benchmark lane

For dependency or workflow changes, run available local equivalents of
`cargo deny check --all-features advisories licenses sources`, `pnpm audit`,
`pnpm test:licenses`, and a tracked-source secret scan. Keep action hashes,
dependency locks, least-privilege permissions, and deployment evidence intact.

For benchmark changes:

```bash
cargo bench --locked --no-default-features --bench performance -- --smoke
```

The scheduled full benchmark compares results and emits warnings; it is not a
correctness gate.

## Review 1: correctness and risk

Review the complete affected path, not only the diff:

- Verify ownership, public behavior, schema/artifact versions, migration,
  generated output, and rollback.
- Inspect hardware/FFI unsafe blocks, C-string and buffer bounds, timeouts,
  error mapping, cleanup, platform linking, and feature-disabled behavior.
- Check run-directory isolation, atomic staging/publish, checksums, immutable
  snapshots, lock/concurrency behavior, and safe failure recovery.
- Check numerical finite-input validation, empty/short/boundary cases, units,
  indexing, deterministic fixtures, and tolerances. Keep WASM limits and worker
  cancellation bounded.
- Check embedded Python source/tests, interpreter and array ownership behavior,
  `PYO3_PYTHON`, and plotting side effects.
- Check static website export, `/pmoke/` paths, locale pairing, accessibility,
  no-JavaScript behavior, search/AI contracts, and browser release coverage.
- Check secrets, raw data, dependency/license/source policy, action pins, CI
  paths, permissions, and generated-file ownership.

If Review 1 finds a problem, fix it and repeat Review 1 from the beginning.

## Review 2: staged handoff

Stage only intended files, then run:

```bash
git diff --cached --check
git diff --cached --stat
git diff --cached
git status --short --branch
```

Confirm valid frontmatter (`name` and `description` only), valid
`agents/openai.yaml`, English guidance, no build products or secrets, expected
generated files, and a clean explanation for environment-dependent failures.
If anything changes after Review 2, repeat both reviews.

## Safe stopping conditions

Pause for user direction when the requested behavior, schema, artifact format,
feature/platform contract, locale pairing, or generated-file owner is
ambiguous; when validation needs live hardware, credentials, deployment, or
destructive cleanup; when a dependency/license/advisory/CI permission change is
out of scope; or when a missing native library/browser/Python runtime would
make a result look like a false pass.

For example, `cargo check --locked --workspace --all-targets --all-features`
can type-check the workspace while a macOS GPIB link still fails. Report those
as separate results.
