# ADR 0003: Local-first bilingual hybrid search

- Status: accepted
- Date: 2026-08-02

## Context

GitHub Pages provides immutable static assets but no trusted search backend. Orama
Cloud requires an account, private synchronization credential, public browser key,
cost owner, and data-processing approval. None is available to the repository. A
silent browser credential or unapproved query transfer would violate the M5 privacy
boundary.

## Decision

Generate a bilingual `pmoke-domain-v1` concept-vector index from canonical MDX during
the static build. Lazy-load Orama OSS after the first query, rank text and vectors in
the browser, and fuse those results with the independently generated locale-scoped
ZBSearch index. Display the active mode and query privacy in the search dialog.

The model is a deterministic domain projection with shared English/Japanese synonym
axes and bounded hashed lexical features. It is described as local hybrid or concept-
vector search, never as a neural embedding, LLM answer engine, or cloud service.

## Consequences

- Search queries, telemetry, config data, and experimental data remain in-browser.
- The semantic index and engine can fail without taking down full-text search.
- Relevance is reproducible and enforced by a versioned bilingual fixture.
- The domain model needs explicit maintenance when documentation terminology grows.
- Orama Cloud remains possible only through a separate security, cost, privacy, and
  relevance ADR with scoped credentials and outage tests.
