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

When running in WSL, treat the repository shell as a Linux-native project
environment. The separate host Nix profile may expose optional Windows
clipboard/browser adapters, but website builds do not require GUI, systemd, or
USB/IP integration. Do not enable or mutate those capabilities just to run a
site build; use an already provisioned Nix browser for visual checks and report
its absence instead of downloading one through a package manager.

## Playwright MCP UI checkpoint

When Playwright MCP is available in the agent session, prefer it for the
interactive visual and accessibility checkpoint. Start the application with
`pnpm run build:dev` from the Nix shell and keep the owned development server
running. Then use the MCP browser tools to:

1. Navigate to `http://localhost:3000/pmoke/en/` and
   `http://localhost:3000/pmoke/ja/` with `browser_navigate`.
2. Use `browser_snapshot` to inspect landmarks, headings, accessible names,
   focusable controls, and the no-JavaScript-readable structure before using
   element actions.
3. Use `browser_resize` for a wide desktop, narrow/tablet, and phone viewport;
   use `browser_take_screenshot` only when visual evidence is useful.
4. Exercise keyboard focus with `browser_press_key`, check light/dark and
   reduced-motion states, and inspect `browser_console_messages` at error and
   warning levels. Check loading, worker/WASM fallback, and interaction states
   relevant to the changed route.

For exported-site validation, run `pnpm start` and repeat the route/deep-link
check below `/pmoke/`. Record only concise, redacted evidence with route,
locale, viewport, theme, and whether it came from `next dev` or the export.
Do not commit MCP screenshots, recordings, console dumps, personal paths, or
raw data unless a reviewed visual snapshot is an explicit project contract.
Prefer snapshots and narrowly scoped read-only evaluation; do not use
`browser_run_code_unsafe` for routine checks. Playwright MCP complements, but
does not replace, `pnpm check`, `pnpm test:e2e`, the Nix-provided browser, or
the CI browser/accessibility/visual and Lighthouse gates. If MCP is unavailable,
use the Nix-provided browser and the repository's Playwright CLI checks, and
report the limitation.

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
For a durable website goal, use the repository's public Issue → normal PR
workflow. Open the PR when implementation starts and keep it available for
review; do not use a Draft PR phase. Keep the Issue's acceptance,
compatibility, and security decisions separate from the PR's current
validation state. Put UI evidence in the Issue or PR with route, locale,
viewport, theme, and dev/export context; do not add an unredacted screenshot,
recording, log, endpoint, or personal path to the repository.

## Long website changes

For a website task that spans content, components, workers/WASM, generated
references, or release checks, split it into independently reviewable slices
with clear ownership and rollback boundaries. Use one normal PR per slice when
it can be validated independently; use explicitly stacked branches and
dependent normal PRs when a later UI slice requires an earlier Rust/WASM or
route slice. State the dependency and merge order, then retarget the next PR
to `main` and rerun CI after the base merges.

Keep commits cohesive: pair a route/component behavior with its tests, keep
English/Japanese content changes together when they describe one public
contract, and include generator-owned output with the source change that
produced it. Do not split commits only by directory or leave a known broken
intermediate state. For each slice, run the narrowest checks, perform Review 1
on behavior and rendered output, stage and perform Review 2 on the complete
diff, then commit. If a fix changes the reviewed surface, repeat both reviews
and the affected checks.

For UI slices, use `pnpm run build:dev` as the iterative visual checkpoint and
record the route, locale, viewport, theme, and dev/export context. Push and
open the normal PR only when external handoff is authorized; merge only after
the exact head has passing required and relevant CI (including browser,
accessibility, visual, and Lighthouse gates when applicable), resolved
conversations, and no blocking review. A single maintainer records Review 1
and Review 2 explicitly rather than fabricating a second approval.

## UI-first workflow

1. Inspect the owning route/component, its locale counterpart, styles, tests,
   worker/WASM consumer, and any generated-file owner before editing.
2. From the repository root, enter the pinned development shell, then switch
   to `website/` and restore the project dependencies:

   ```bash
   nix develop
   cd website
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

   Use the Playwright MCP UI checkpoint above when it is available. Otherwise,
   use an attached browser or the Nix-provided Playwright browser. Check at
   least a wide desktop viewport, a tablet or narrow desktop viewport, and a
   phone viewport; check light/dark mode, keyboard focus, reduced-motion
   behavior, loading and error states, and the browser console for relevant
   errors. If no browser is available, report the live URL and limitation
   instead of claiming a visual pass.

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

Use a GitHub Issue for the durable user-facing goal, a normal PR opened at the
start of active work, and the PR description for a concise checklist of scope,
visible outcome, validation, screenshots, and blockers. Keep private
deliberation and temporary notes out of tracked files. Use README/docs for
stable instructions and a changelog for released user-visible changes; do not
add a chronological public work diary unless the project explicitly wants one.

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
