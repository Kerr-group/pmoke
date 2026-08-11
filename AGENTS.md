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
- `flake.nix` and `flake.lock` define the reproducible local Nix development
  shell for Rust, Node, pnpm, WASM, and linker tooling. Keep Nix input updates
  intentional and review their platform coverage.
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

## Skill routing

- Use `$pmoke-maintenance` for cross-cutting Rust, hardware transport, Python,
  WASM, generated documentation, website, benchmark, CI, and repository-policy
  work.
- Use `$pmoke-website` for website routes, components, styles, MDX, browser
  workers, site builds, browser tests, and visual UI review. Use both skills
  when a website change crosses the Rust/WASM or generated-documentation
  boundary.

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

## Nix-managed local environment

The primary development workstation for this repository is managed by Nix.
For agent-run local work, use Nix as the installation authority:

- Inspect repository-local `flake.nix`, `flake.lock`, `devenv.nix`, and `.envrc`
  first. Enter a repository development shell with `nix develop` when one is
  provided.
- If the repository has no development shell, prefer an existing managed Nix
  flake. For a one-off tool, use a temporary `nix shell` or `nix run` from a
  pinned flake input; if only a registry package is available, record its
  evaluated version and do not claim repository-level reproducibility.
- Do not use `cargo install`, `rustup target add`, `pip install`/`pipx`, global
  `npm`/`pnpm` installs, Homebrew, apt, or similar host package managers unless
  the user explicitly authorizes an exception. `pnpm install --frozen-lockfile`
  is an allowed project-local dependency restore; do not use `pnpm add` unless a
  dependency change is in scope.
- Do not run `nix flake update` or rewrite a lockfile merely to obtain a tool.
  If a required tool is absent from the available Nix inputs, report the
  missing package and keep the validation blocked rather than bypassing Nix.
- Use Nix-provided browsers and native libraries for local validation. Record
  the package source/version and any environment limitation in the handoff.
- The repository shell provides Python 3.12 with the Nix-packaged NumPy, SciPy,
  lmfit, matplotlib, and pinned PyPI `gsplot` runtimes for the Rust/PyO3 and
  analysis tests; it sets `PYO3_PYTHON` to that interpreter and exposes its
  site-packages to embedded Python. The shell applies a narrow override for
  one known SciPy precision-test failure with the pinned NumPy/SciPy pair while
  retaining the rest of the dependency checks. Linux shells additionally
  provide Chromium, `pkg-config`, and the Nix-packaged linux-gpib userspace
  library for the GPIB build lane. Do not install these dependencies with pip
  or another host package manager.

### WSL and the unified Nix host configuration

The separate unified Nix configuration repository is the host-environment
owner. It may provide a standalone Home Manager profile for WSL, but `pmoke`
must not import a private checkout, local filesystem path, host registry, or
machine-specific module from that repository. This public repository owns its
own pinned `flake.nix`/`flake.lock` and project development shell.

- Bootstrap Nix inside WSL using the official Nix and WSL guidance. Do not
  install Nix, Rust, Node.js, pnpm, Python, or native libraries through Scoop,
  winget, or another Windows package manager.
- In WSL, enter this repository's `nix develop` shell before building or
  testing. WSL is a Linux-native validation environment; do not describe a
  WSL build as a native Windows MSVC/VISA build.
- Resolve the WSL architecture from the host/profile and the Nix system. Do
  not hard-code `x86_64-linux` into WSL-specific scripts or claim that a
  different architecture is supported without adding and validating its shell.
  The current pmoke flake exposes `x86_64-linux` and `aarch64-darwin` shells.
- Windows clipboard/browser integration may be an opt-in host capability, but
  GUI integrations, systemd user services, and GPIB USB/IP support are not
  prerequisites for ordinary pmoke builds or tests. Do not enable them as a
  side effect of validation.
- If an explicitly authorized WSL GPIB preflight is needed, it must be
  read-only. Do not run `usbipd` bind/attach, load kernel modules, change udev,
  create `/dev/gpib0`, or access instruments as part of a build/test lane.
  Use dummy transports, loopback fixtures, and isolated temporary directories
  unless a separate hardware operation has been approved.
- WSL profile activation, `/etc/wsl.conf`, Windows integration, and Windows
  package state remain outside pmoke's ownership. A missing WSL capability is
  an environment limitation to report, not a reason to bypass the Nix policy.

CI may use its own installation mechanism; preserve those workflow steps unless
changing CI is explicitly requested.

## Optional real-machine validation

Use the maintained Windows and Linux machines as optional platform-validation
targets when the operator has explicitly authorized access. Keep their
connection details private: never commit IP addresses, hostnames, usernames,
SSH key names, private key paths, or machine-specific filesystem paths. Define
local SSH aliases in `~/.ssh/config` or another ignored local file instead:

```ssh-config
Host pmoke-windows
  HostName <windows-address>
  User <windows-user>
  IdentityFile ~/.ssh/<windows-key>

Host pmoke-linux
  HostName <linux-address>
  User <linux-user>
  IdentityFile ~/.ssh/<linux-key>
```

For native Windows builds, an SSH-launched PowerShell session may not inherit
the Visual Studio Developer Command Prompt environment. Initialize the
installed MSVC toolchain through `vswhere.exe` and `VsDevCmd.bat -arch=x64`
using a local profile or helper, then verify the tools before building:

```powershell
Get-Command link.exe, cl.exe
```

Do not hard-code a versioned Visual Studio directory into the repository. The
helper should discover the installation through the Visual Studio installer and
import its environment. Rust may fall back to its bundled `rust-lld.exe`, but
native Windows FFI/VISA validation must still expose and verify `link.exe` and
`cl.exe`.

Connect only through the aliases and validate the exact revision before
building:

```bash
# Choose the target appropriate to the validation lane.
ssh pmoke-linux
# or: ssh pmoke-windows

# Run these commands after connecting to the target.
git status --short --branch
git rev-parse HEAD
```

Use a clean checkout, detached worktree, or test-owned temporary directory on
the target. On a Nix-provisioned Linux host or WSL distribution, enter the
repository shell and run the platform-safe build and test lane:

```bash
nix develop
cargo build --locked --workspace --all-targets --no-default-features
cargo test --locked --workspace --lib --bins --tests --examples --no-default-features
cd website
pnpm install --frozen-lockfile
pnpm build
```

Native Windows builds additionally require the Windows feature matrix and its
VISA SDK/native dependencies from CI. The repository flake targets Linux and
macOS; it is not a native Windows package manager environment. For Nix-backed
Windows development, use WSL2 or NixOS-WSL and install Nix inside that Linux
environment using the [official Nix installation instructions](https://nix.dev/manual/nix/stable/installation/)
and [WSL guidance](https://wiki.nixos.org/wiki/WSL). Do not install the Nix
package manager through Scoop, winget, or another Windows package manager.
The one-time Nix bootstrap is an environment prerequisite; after it is
available, install project tools and enter the repository shell with Nix. Do
not install Rust, Node, pnpm, Python, or native libraries through Scoop,
winget, rustup, Homebrew, or another host package manager. A missing Nix/WSL or
native SDK is an environment limitation to report, not a reason to bypass the
repository policy. Build and test commands must not trigger, fetch from,
screenshot, or otherwise operate live equipment unless that separate hardware
action is explicitly authorized.

Record the target role, commit, feature profile, toolchain source, commands,
result, and environment limitations in the private handoff. Public Issues or
PRs should contain only the redacted summary and reproducible evidence; never
publish endpoint details, credentials, raw captures, or unreviewed logs.

## Public progress and collaboration

This repository is public, so treat branches, commits, issues, pull requests,
logs, screenshots, and generated reports as public artifacts. Use the following
split:

- Keep the source of truth separated by purpose:
  `docs/project/requirements.md` contains stable product requirements,
  compatibility promises, and security boundaries; GitHub Issues contain the
  purpose, scope, non-goals, acceptance criteria, compatibility impact, and
  security impact of a durable goal; PRs contain the active implementation
  state and validation evidence; `SECURITY.md` contains private
  vulnerability-reporting instructions. A GitHub Project may provide an
  optional dashboard, but it is not a requirements or status source of truth.
- Track one durable, user-facing goal in a GitHub Issue; open a normal linked
  PR when implementation starts and keep its description as a short
  scope/outcome/validation/blocker checklist. Do not use a Draft PR phase.
- When implementation starts, link the PR to the Issue with `Refs #N` and keep
  it open for review from the beginning. Record Review 1 on the Issue or design
  discussion and Review 2 on the complete staged PR diff, tests, and generated
  artifacts. Merge only after the acceptance criteria and validation evidence
  are complete, the current head is up to date, required and relevant checks
  pass on that head, conversations are resolved, and no blocking review remains.
  With explicit maintainer authorization, merge first, then perform any
  required deployment or release verification and close the Issue explicitly.
- Record stable instructions in README or docs and released user-visible
  changes in the changelog. Do not turn the repository into a chronological
  progress diary or commit private deliberation.
- Keep temporary planning in the conversation or ignored local files. Never
  commit secrets, credentials, raw measurements, personal data, private URLs,
  internal-only roadmap details, local filesystem paths, or unreviewed logs.
- Never use a public Issue, PR, discussion, or commit to disclose a security
  vulnerability. Follow `SECURITY.md`; do not add a public security-reporting
  template that asks for exploit details.
- For UI work, attach concise screenshot or recording evidence to the Issue or
  PR with route, locale, viewport, theme, and dev/export context. Keep large or
  sensitive captures out of the repository unless versioned snapshots are an
  explicit project contract.
- Report progress at meaningful milestones: scope, user-visible result,
  validation status, known environment limitation, and next decision. Keep
  public wording factual and safe to quote.

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

Use the repository-pinned tools where possible: Nix-provided Rust stable with
edition 2024 and the `wasm32-unknown-unknown` target, Python 3.12 in CI,
Node.js `22.23.1` from `website/.node-version`, pnpm `11.18.0`, and wasm-pack
`0.15.0`. Enter the repository shell before local validation:

```bash
nix develop
```

Check the resulting versions against the repository pins; do not suppress an
engine or toolchain mismatch by installing outside Nix. Validate the shell
definition itself with `nix flake check --all-systems` when changing it.

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
source change. For the site/WASM path, enter `nix develop`, then run the
relevant `cargo check` commands for `pmoke-analysis-core` and `pmoke-web-wasm`
against `wasm32-unknown-unknown`.

### Website

From the repository root, enter `nix develop`, then switch to `website/` and
install with `pnpm install --frozen-lockfile`. Run the checks relevant to the
change; `pnpm check` is the normal source gate and
includes lint, Japanese editorial checks, content/readme verification, type
checking, search, signal, AI-resource, and export checks.

For iterative UI work, start the development server and review the rendered
route together before treating the change as complete:

```bash
pnpm run build:dev
# inspect http://localhost:3000/pmoke/en/ and /pmoke/ja/
```

Check a changed route at wide, narrow, and phone widths, plus keyboard focus,
light/dark mode, loading/error states, and the browser console. `build:dev`
rebuilds the browser WASM package before starting Next.js; `pnpm dev` is an
equivalent alias. Use `pnpm start` after an export to verify the production
`/pmoke/` path.

```bash
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
