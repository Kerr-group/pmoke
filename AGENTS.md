# AGENTS.md

This file defines repository-wide instructions for `pmoke`. Apply it to the
whole repository unless a deeper `AGENTS.md` or `AGENTS.override.md` adds more
specific guidance. System, developer, and user instructions always take
precedence over this file.

## Repository map and sources of truth

`pmoke` is a Rust 2024 workspace with hardware transports, numerical analysis,
an embedded Python bridge, a WebAssembly boundary, and a bilingual static
documentation site.

- `Cargo.toml` and `Cargo.lock` define the Rust workspace and its locked
  dependency graph. Keep the lockfile intentional and review dependency,
  license, source, and feature changes together.
- `src/` contains the CLI, terminal UI, configuration loading and migration,
  run-directory/provenance management, acquisition and analysis workflows,
  plotting, and the Python bridge.
- `crates/instruments/`, `crates/gpib-rs/`, and `crates/prologix-rs/` contain
  instrument models and GPIB, Prologix TCP, and Prologix serial transports.
- `crates/pmoke-analysis-core/` is the dependency-light deterministic analysis
  core shared with WebAssembly. `crates/pmoke-config-core/` is the
  platform-independent configuration and validation core.
- `crates/pmoke-web-wasm/` is the browser-facing Rust/WASM adapter. Keep
  platform-specific hardware and Python dependencies out of this boundary.
- `xtask/` owns generated CLI references, configuration references, schema
  output, and English/Japanese generated reference pages. Run
  `cargo xtask docs-export`; do not hand-edit generated reference output.
- `website/content/docs/{en,ja}/` contains paired MDX documentation. Keep
  locale structure, public behavior, and examples aligned.
- `website/` is a Next.js/Fumadocs static export served below the GitHub Pages
  project path `/pmoke/`. Its workers, search indexes, AI-facing exports,
  license checks, and browser tests are part of the product surface.
- `scripts/` contains Python benchmark and comparison utilities. Python code
  embedded by Rust is stored beside the owning Rust module and included with
  `include_str!`; update both the Rust boundary and its Python tests when
  changing that behavior.
- `.github/workflows/` is an executable release and validation contract. Keep
  action pins, least-privilege permissions, path filters, feature coverage,
  generated-file checks, and deployment evidence intact.

Tracked generated files include `website/generated/`,
`website/public/config.schema.json`, and the generated CLI/configuration MDX
references. Build-only output such as `website/public/wasm/`, `website/.next/`,
and `website/out/` is intentionally untracked. Never commit run directories,
raw captures, plots, local configuration, Python caches, credentials, tokens,
instrument logs, or personal data.

## Authority and safety gates

Treat the following as potentially stateful or externally visible:

- commands that connect to GPIB, VISA, TCP/IP, or serial instruments;
- `pmoke` commands that trigger, fetch, screenshot, auto-measure, or otherwise
  change instrument state;
- `doctor --probe-fetch`, which explicitly permits active hardware checks;
- writes outside a test-owned temporary directory, deployment actions, and
  release or dependency updates;
- changes to configuration schema versions, artifact formats, feature
  ownership, CI permissions, or security policy.

Do not run live instrument operations or destructive cleanup merely to validate
source code. Use dummy transports, loopback fixtures, unit tests, and isolated
temporary directories. Confirm the exact target and user intent before any
hardware access, external write, broad deletion, force push, or release action.
Do not use `git reset --hard`, broad `rm -rf`, force push, or branch deletion to
resolve an unclear state. Preserve unrelated work in a dirty tree.

Do not add secrets, API keys, VISA/GPIB credentials, raw experimental data,
private URLs, machine-specific paths, or generated artifacts containing such
data. Keep external downloads and CI actions pinned and integrity-checked when
editing workflows.

## Working procedure

1. Start with `git status --short --branch`, `git remote -v`, and
   `git log -5 --oneline --decorate`. Record the current branch and preserve
   unrelated changes.
2. Read the relevant source, consumers, tests, feature definitions, generated
   file owners, and workflow gates before editing. For cross-cutting changes,
   inspect both Rust and website consumers instead of inferring behavior from a
   single module.
3. Make a small plan. State the affected crate, platform/feature profiles,
   generated outputs, external systems, and validation commands.
4. Edit source files with `apply_patch`. Keep generated files synchronized by
   running their owner (`cargo xtask docs-export`) and reviewing the result;
   never conceal generated drift with manual edits.
5. Run the narrowest relevant checks first, then the broader validation matrix
   below. Distinguish a local missing system dependency from a passing test.
6. Perform Review 1 on the working tree. If it finds a problem, fix it and
   restart Review 1.
7. Stage only the intended files and perform Review 2 on the staged diff. If
   anything changes after Review 2, repeat both reviews.
8. Use a concise conventional commit when the user requested a commit. Do not
   push, merge, deploy, or delete the branch unless the user explicitly asks
   for that handoff.

## Toolchain and validation matrix

Use the repository-pinned tools where possible: Rust stable with edition 2024,
Python 3.12 in CI, Node.js `22.23.1` from `website/.node-version`, pnpm
`11.18.0`, `wasm32-unknown-unknown`, and wasm-pack `0.15.0`. Use `--locked` for
normal Rust verification.

### Rust and Python baseline

```bash
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets --all-features
cargo test --locked --workspace --lib --bins --tests --examples --all-features
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings

PYTHONDONTWRITEBYTECODE=1 python -m unittest discover \
  -s src/kerr/pytools -p 'test_*.py'
PYTHONDONTWRITEBYTECODE=1 python -m unittest discover \
  -s scripts -p 'test_*.py'
```

The full-feature test suite needs the platform's native libraries. Linux CI
installs the linux-gpib userspace library; Windows VISA checks require the
VISA SDK. On macOS, validate the supported bundle without direct GPIB:

```bash
cargo check --locked -p pmoke --all-targets --no-default-features \
  --features hw-core,hw-prologix-tcp,hw-prologix-serial
cargo test --locked -p pmoke --all-targets --no-default-features
cargo clippy --locked -p pmoke --all-targets --no-default-features -- -D warnings
```

Also cover analysis-only and transport profiles when their code changes:

```bash
cargo check --locked -p pmoke --all-targets --no-default-features
cargo test --locked -p pmoke-analysis-core -p pmoke-config-core -p pmoke-web-wasm
cargo test --locked -p instruments --all-targets --no-default-features
cargo test --locked -p prologix-rs --all-targets --no-default-features
```

For GPIB or Prologix changes, use the feature matrix in `ci.yml`, including
Linux, macOS, and Windows profiles as applicable. A successful `cargo check`
does not prove that a native GPIB/VISA link or a connected instrument works.

### Generated references and WebAssembly

When CLI, configuration, schema, public command behavior, or relevant Rust
types change:

```bash
cargo test --locked -p xtask
cargo xtask docs-export
git diff --exit-code -- \
  website/generated \
  website/public/config.schema.json \
  website/content/docs/en/cli \
  website/content/docs/ja/cli \
  website/content/docs/en/configuration/reference.mdx \
  website/content/docs/ja/configuration/reference.mdx
```

Review and commit generated changes when they are the expected result of a
source change. For the site/WASM path, install the pinned target and wasm-pack,
then run the relevant `cargo check` commands for `pmoke-analysis-core` and
`pmoke-web-wasm` against `wasm32-unknown-unknown`.

### Website

From `website/`, install with `pnpm install --frozen-lockfile`. Then run the
checks relevant to the change; `pnpm check` is the normal source gate and
includes lint, Japanese editorial checks, content/readme verification, type
checking, search, signal, AI-resource, and export checks.

```bash
pnpm check
pnpm test:licenses
pnpm build
pnpm exec playwright install --with-deps chromium
pnpm test:e2e
```

The CI release gate additionally runs cross-browser release tests and
Lighthouse. Preserve `/pmoke/` in links, routes, workers, static assets,
canonical URLs, and tests. Use `pnpm start` to inspect the exported tree before
claiming a production-path change is valid.

### Security and performance

For dependency or workflow changes, run the available local equivalents of the
CI gates: `cargo deny check --all-features advisories licenses sources`,
`pnpm audit`, `pnpm test:licenses`, and a tracked-source secret scan. Do not
replace an unavailable gate with an unsupported claim of success.

For benchmark changes, run the deterministic smoke test:

```bash
cargo bench --locked --no-default-features --bench performance -- --smoke
```

The scheduled full benchmark records and compares performance; it is
informational and must not be described as a correctness proof.

## Code Review Rules

Review every relevant consumer and test, not only the edited lines.

### Review 1: design and behavior

- Verify the requirement, ownership boundary, public behavior, migration path,
  and rollback story. Keep CLI/config/schema versions and generated references
  consistent.
- For hardware and FFI code, inspect unsafe blocks, C-string and buffer
  lengths, timeout/error mapping, cleanup, platform-specific linking, and
  feature-disabled behavior. Do not test against real equipment without
  explicit authorization.
- For run directories and provenance, preserve isolation, checksums, immutable
  snapshots, atomic staging/publish behavior, lock/concurrency guarantees, and
  failure recovery. Ensure a user-supplied path cannot escape its intended
  root.
- For numerical analysis, check finite-input validation, empty/short/boundary
  cases, indexing, deterministic seeds, units, tolerances, and the shared-core
  fixture. Keep browser limits and worker cancellation behavior bounded.
- For Python integration, update the embedded source and its test together;
  verify `PYO3_PYTHON`, array ownership/copy behavior, interpreter errors, and
  plotting side effects.
- For website changes, preserve English/Japanese parity, static export, `/pmoke/`
  base paths, no-JavaScript readability, keyboard/accessibility behavior,
  worker/WASM failure recovery, search fallbacks, AI-resource contracts, and
  release metadata.
- For CI and dependencies, preserve locked inputs, action pins, permissions,
  advisory/license/source checks, feature slices, generated-file checks, and
  deployment evidence. Check that performance warnings remain informational.
- Confirm no secret, raw capture, local artifact, credential, or unreviewed
  external source entered the change.

### Review 2: handoff integrity

After staging, run and inspect:

```bash
git diff --cached --check
git diff --cached --stat
git diff --cached
git status --short --branch
```

Confirm the staged diff contains only the requested English guidance/skill (and
expected generated output), has valid skill frontmatter and metadata, does not
include ignored build products, and matches the branch/commit request. Recheck
lockfile, source/license policy, generated ownership, test evidence, and any
environment-dependent failure explanations.

## Stop conditions

Pause and ask for direction when:

- a schema, artifact, CLI, feature, platform, locale, or generated-file change
  has more than one plausible contract;
- a test requires live hardware, credentials, deployment, external writes, or
  a destructive operation;
- a dependency/lockfile, license, advisory, CI permission, or release policy
  change is not explicitly in scope;
- generated output disagrees with its source and the intended owner is unclear;
- a native library, Python interpreter, browser, or network dependency is
  missing and the result would otherwise be reported as a pass;
- a dirty tree, merge conflict, branch protection rule, or permission issue
  would require deleting or overwriting someone else's work.

For example, `cargo check --locked --workspace --all-targets --all-features`
can prove that Rust targets type-check while still failing to prove a macOS
GPIB link or a live oscilloscope query; report those as separate gates.
