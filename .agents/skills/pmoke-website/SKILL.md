---
name: pmoke-website
description: Build and review pmoke's bilingual Next.js/Fumadocs website and browser-facing tools with development-server UI checkpoints, responsive and accessibility checks, static `/pmoke/` routing, search/AI/export validation, and Rust/WASM integration. Use for website UI, MDX content, components, CSS, workers, browser tests, or site builds; do not use for unrelated Rust changes or live instrument operations.
---

# Pmoke website UI

Use this skill for changes under `website/`, browser-facing Rust/WASM behavior,
or documentation changes whose rendered result is part of the public site. Read
the repository root `AGENTS.md` first and combine this skill with
`$pmoke-maintenance` for cross-boundary changes.

Use the Nix-managed local toolchain. Before installing anything, inspect a
repository `flake.nix`/`devenv.nix` and enter `nix develop` when available. If
there is no repository shell, use a temporary Nix shell for the missing tools;
do not use `cargo install`, `rustup target add`, global npm/pnpm, or
`pnpm exec playwright install` on the workstation.

## Product map

- `website/app/`, `components/`, and `styles/`: Next.js routes and UI.
- `website/content/docs/en` and `website/content/docs/ja`: paired MDX sources.
- `website/public/workers/`: bounded signal and waveform worker adapters.
- `website/public/fixtures/`: deterministic, public waveform fixtures.
- `crates/pmoke-web-wasm/` and `crates/pmoke-analysis-core/`: browser-facing
  Rust boundary and deterministic shared analysis.
- `website/scripts/`: search, export, AI-resource, license, and release gates.

Preserve the `/pmoke/` base path, static export behavior, English/Japanese
parity, no-JavaScript readability, and bounded worker/WASM failure recovery.
Treat generated references and release metadata as owned by their generators.

## UI-first workflow

1. Inspect the owning route/component, its locale counterpart, styles, tests,
   worker/WASM consumer, and any generated-file owner before editing.
2. From `website/`, use the repository Nix shell when available. Otherwise,
   provide the missing website tools ephemerally with Nix, then restore the
   project dependencies:

   ```bash
   nix shell nixpkgs#nodejs_22 nixpkgs#pnpm nixpkgs#wasm-pack nixpkgs#wasm-bindgen-cli_0_2_126 nixpkgs#lld
   pnpm install --frozen-lockfile
   ```

   `pnpm install --frozen-lockfile` restores project-local dependencies and is
   not a global tool installation. Check Node/pnpm/wasm-pack versions against
   `website/package.json` and `website/README.md`.

3. For iterative UI work, run the repository's canonical development command:

   ```bash
   pnpm run build:dev
   ```

   This builds the browser WASM package and then starts `next dev`. Keep the
   owned server running during the visual review. Inspect the changed route and
   both locale roots:

   - `http://localhost:3000/pmoke/en/`
   - `http://localhost:3000/pmoke/ja/`

   Use an attached browser or Playwright when available. Check at least a wide
   desktop viewport, a tablet or narrow desktop viewport, and a phone viewport;
   check light/dark mode, keyboard focus, reduced-motion behavior, loading and
   error states, and the browser console for relevant errors. If no browser is
   available, report the live URL and the limitation instead of claiming a
   visual pass.

4. Stop at a visual checkpoint when layout, typography, interaction, or visual
   hierarchy is a matter of product choice. Show the route/viewport evidence
   and obtain the user's direction before making a broad design change. Do not
   silently turn a typecheck or static build into a visual approval.

5. Before handoff, run the smallest relevant production and contract checks:

   ```bash
   pnpm run build:site
   pnpm check
   pnpm test:licenses
   pnpm test:e2e
   ```

   Use `pnpm build` when the change includes the browser WASM build or needs the
   full release-shaped site build. Use `pnpm start` to inspect the exported
   `/pmoke/` tree, not only the development server.

   For Chromium-based checks on this workstation, use a Nix-provided browser
   and set `PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH` to its executable. Do not
   download Playwright-managed browsers with a package-manager install command.

## Content and integration rules

- Keep the English and Japanese route trees paired. Update both locale pages,
  navigation labels, metadata, and examples when the public concept exists in
  both languages; record an intentional locale exception explicitly.
- Use base-path-safe links and assets. Verify direct navigation, refresh, and
  static export paths below `/pmoke/`.
- Keep browser computation off the main thread where the existing worker
  boundary requires it. Preserve input-size limits, finite-input validation,
  cancellation, typed error handling, and user-visible recovery.
- Keep AI-facing text exports, search indexes, signal fixtures, release
  metadata, and license manifests synchronized with their scripts. Do not
  hand-edit generated output.
- Keep public examples deterministic and free of real measurements, private
  URLs, credentials, machine paths, or identifying data.
- Prefer small, composable UI changes. Add or update browser tests for stable
  behavior, and use screenshot evidence for visual changes when the project
  workflow supports it.

## Public progress and handoff

Use a GitHub Issue for the durable user-facing goal, a Draft PR for active work,
and the PR description for a concise checklist of scope, visible outcome,
validation, screenshots, and blockers. Keep private deliberation and temporary
notes out of tracked files. Use README/docs for stable instructions and a
changelog for released user-visible changes; do not add a chronological public
work diary unless the project explicitly wants one.

For UI checkpoints, record the route, locale, viewport, theme, and whether the
evidence came from `next dev` or the exported site. Never include tokens,
private hostnames, raw captures, internal-only roadmap details, or personal
data in issues, PRs, screenshots, logs, or commits.

## Safe stopping conditions

Pause for direction when the desired visual behavior, locale contract, public
route, generated-file owner, or accessibility behavior is ambiguous. Report a
blocked validation when `wasm-pack`, the pinned Node/pnpm toolchain, a browser,
or a native dependency is missing. Do not run deployment, upload artifacts,
contact live hardware, or change external project tracking without explicit
authorization.
