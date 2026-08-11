# pmoke documentation site

Static bilingual documentation for `pmoke`, built with Next.js, Fumadocs, and a
small Rust WebAssembly preview module. Every public route is exported beneath the
GitHub Pages project path `/pmoke/`.

## Toolchain

- Node.js 22.23.1
- pnpm 11.18.0 (Nix-managed locally; Corepack in CI)
- Rust stable with `wasm32-unknown-unknown`
- wasm-pack 0.15.0

This workstation is Nix-managed, and this repository provides a pinned
development shell for the website tools:

```bash
nix develop
```

Check the versions after entering the shell. The website pins exact Node and
pnpm versions; a close Nix package version is not a reason to ignore an engine
warning. Do not use `cargo install`, `rustup target add`, or
`pnpm exec playwright install` on the Nix-managed workstation. The repository
shell provides pinned Chromium on Linux and selects it for
`PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH`; use that Nix-provided executable for
browser checks. On macOS, use an existing Nix-managed Chrome/Chromium
executable. If a required Rust target or browser is not available through Nix,
report the validation as blocked rather than installing outside the shell.

## UI development loop

From `website/`, start the development server with:

```bash
pnpm install --frozen-lockfile
pnpm run build:dev
```

The command rebuilds the browser WASM package before starting Next.js. Review
the changed route and both locale roots at
`http://localhost:3000/pmoke/en/` and
`http://localhost:3000/pmoke/ja/`. Check responsive layout, light/dark mode,
keyboard focus, loading/error states, and the browser console before stopping
the server. `pnpm dev` remains an equivalent alias.

Use `pnpm run build:site` for a static site build without rebuilding WASM, and
use `pnpm build` before handoff when the full release-shaped build is needed.

## Local verification

```bash
nix develop
cd website
pnpm install --frozen-lockfile
pnpm build
pnpm check
if command -v chromium >/dev/null 2>&1; then
  export PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH="$(command -v chromium)"
elif command -v google-chrome >/dev/null 2>&1; then
  export PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH="$(command -v google-chrome)"
else
  echo "A Nix-provided Chrome/Chromium executable is required for test:e2e" >&2
  exit 1
fi
pnpm test:e2e
```

If neither `google-chrome` nor `chromium` is available from the Nix-managed
environment, stop and report the browser gate as blocked rather than invoking
Playwright's browser downloader.

`pnpm start` serves the exported tree at `http://127.0.0.1:4173/pmoke/` and
preserves the production base path. On NixOS, set
`PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH` to a wrapped local Chrome or Chromium
binary when running browser tests.

## Boundaries

- `content/docs/{en,ja}`: locale-prefixed MDX sources
- `app/api/search`: generated multilingual static search index
- `app/llm` and `app/llms*.txt`: agent-facing text exports
- `crates/pmoke-web-wasm`: pure Rust browser boundary
- `public/workers/signal.worker.js`: off-main-thread Wasm loader
- `public/workers/waveform-analyzer.worker.js`: bounded lock-in/Kerr analysis Worker
- `public/fixtures/`: CC0 deterministic waveform golden fixtures
- `scripts/verify-export.mjs`: base-path, locale, and payload gates

Generated `public/wasm`, `.next`, and `out` files are intentionally untracked.
