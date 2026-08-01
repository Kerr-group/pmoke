# ADR 0001: Static documentation architecture

- Status: Accepted
- Date: 2026-08-01

## Context

The pmoke documentation must support researchers, hardware operators, and AI
agents from GitHub Pages. GitHub Pages cannot provide a persistent Next.js server,
while the site still needs bilingual navigation, search, LLM text, and Rust-backed
interactive previews.

## Decision

1. Export every route statically with Next.js `output: "export"` and the fixed
   `/pmoke` project base path.
2. Publish explicit `/en` and `/ja` trees. Do not use middleware, cookies, or
   runtime locale rewrites.
3. Use Fumadocs' built-in multilingual static search as the local baseline.
   Treat semantic cloud search as a later, independently degradable layer.
4. Keep browser-compatible Rust in focused pure crates. Build Wasm with a pinned
   wasm-pack version and load it in a Web Worker.
5. Generate AI-facing text from the same typed Fumadocs source as human pages.
6. Verify deep links, locale metadata, Wasm pixels, responsive layouts, and payload
   budgets against a local server mounted at the production base path.

## Consequences

- All dynamic parameters require deterministic static generation.
- Search and AI resources remain available without an application server.
- Runtime-only Next.js features are outside the architecture boundary.
- Interactive Rust functionality requires native/Wasm parity gates as domain
  logic moves into later core crates.
