# pmoke documentation site

Static bilingual documentation for `pmoke`, built with Next.js, Fumadocs, and a
small Rust WebAssembly preview module. Every public route is exported beneath the
GitHub Pages project path `/pmoke/`.

## Toolchain

- Node.js 22.23.1
- pnpm 11.18.0 through Corepack
- Rust stable with `wasm32-unknown-unknown`
- wasm-pack 0.15.0

## Local verification

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-pack --version 0.15.0 --locked
corepack enable pnpm
cd website
pnpm install --frozen-lockfile
pnpm build
pnpm check
pnpm exec playwright install chromium
pnpm test:e2e
```

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
