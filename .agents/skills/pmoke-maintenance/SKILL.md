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

### Nix-managed environment

The local development workstation is Nix-managed. Inspect a repository
`flake.nix`, `flake.lock`, `devenv.nix`, or `.envrc` first and use `nix develop`
when available. Otherwise use a temporary `nix shell`/`nix run` from a pinned
flake input for missing tools. If only a registry package is available, record
its evaluated version and do not claim repository-level reproducibility. Do not
use `cargo install`, `rustup target add`, global npm/pnpm, `pip install`,
Homebrew, or system package managers for local setup unless the user explicitly
authorizes an exception. Do not update Nix inputs merely to obtain a tool;
report an unavailable package as an environment block.

The repository shell provides Python 3.12 with the Nix-packaged NumPy runtime
required by the Rust/PyO3 tests, sets `PYO3_PYTHON` to that interpreter, and
exposes its site-packages to embedded Python. The pinned nixpkgs revision does
not currently provide a complete buildable environment for `scipy`, `lmfit`,
`matplotlib`, and `gsplot` on every supported host. Do not install those
packages with pip; report checks that require them as an environment
limitation.

### WSL boundary

The unified Nix host repository may own a standalone Home Manager profile for
WSL, but it is not a dependency of this public repository. Do not import its
private checkout, host registry, local paths, or machine-specific modules.
Bootstrap Nix inside WSL using the official Nix/WSL guidance, then use this
repository's `nix develop` shell as the project environment. WSL is a
Linux-native lane and must not be reported as native Windows MSVC/VISA
validation.

Resolve the WSL architecture from the host/profile instead of hard-coding it.
The current pmoke shell exposes `x86_64-linux` and `aarch64-darwin`; adding a
new system requires an explicit shell and validation change. Windows
integration can be an opt-in profile capability, while GUI, systemd, and GPIB
USB/IP are not ordinary build prerequisites. A GPIB preflight, when separately
authorized, is read-only: never bind or attach USB/IP devices, load modules,
change udev, create `/dev/gpib0`, or contact instruments during source checks.

### Optional real-machine platform lane

When the user explicitly authorizes platform validation, use local SSH aliases
for the Windows and Linux targets. Do not place their addresses, usernames,
SSH key names, private key paths, or machine-specific directories in tracked
files. Confirm the target and exact commit, then use a clean checkout or
test-owned temporary worktree. On a Nix-provisioned Linux host or WSL
distribution, run the safe build/test lane from the repository root:

```bash
nix develop
cargo build --locked --workspace --all-targets --no-default-features
cargo test --locked --workspace --lib --bins --tests --examples --no-default-features
```

For native Windows SSH sessions, initialize the installed MSVC environment
through Visual Studio's `vswhere.exe` and `VsDevCmd.bat -arch=x64` before
checking `link.exe` and `cl.exe`. Do not hard-code a versioned Visual Studio
path in tracked files. The repository flake is for Linux/macOS systems; use
WSL2 or NixOS-WSL for the Nix lane on Windows, and bootstrap Nix there using
the [official Nix installation instructions](https://nix.dev/manual/nix/stable/installation/)
rather than Scoop or another Windows package manager.

For the website on the same target, run `pnpm install --frozen-lockfile` and
`pnpm build` inside `website/` while the package manager comes from the Nix
shell. Native Windows/VISA validation must follow the CI feature matrix and
requires its pre-provisioned SDKs. A missing Nix/WSL or native dependency is a
reported limitation; do not bypass the Nix-only installation policy. Never
run live instrument trigger, fetch, screenshot, auto-measure, or probe
commands as part of this lane without separate explicit authorization.

On WSL, website and Rust checks use the Linux-native Nix shell. Do not enable
GUI/systemd/USB-IP integration merely to run `build`, `check`, or `test`; use a
Nix-provided browser when visual or browser validation is required and report
its absence as an environment limitation.

Keep machine-test results private until redacted. A public handoff should
include only the target role, commit, feature profile, toolchain source,
commands, pass/fail result, and environment limitation; omit endpoints,
credentials, raw captures, and unreviewed logs.

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
pinned Nix-provided wasm-pack version from `website/README.md`.

### Website lane

From `website/`:

```bash
pnpm install --frozen-lockfile
pnpm check
pnpm test:licenses
pnpm build
browser_bin="$(command -v google-chrome || command -v chromium || true)"
test -n "$browser_bin" || {
  echo "A Nix-provided Chrome/Chromium executable is required" >&2
  exit 1
}
PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH="$browser_bin" pnpm test:e2e
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
