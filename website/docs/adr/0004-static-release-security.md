# ADR 0004: Static release security and evidence

## Status

Accepted for M6.

## Context

GitHub Pages serves a static export and does not expose arbitrary response-header
configuration. The release still needs reproducible performance, accessibility,
supply-chain, integrity, deployment, and rollback evidence.

## Decision

- Enforce a document-level Content Security Policy and strict referrer policy in
  every exported HTML document. The policy permits same-origin scripts, Workers,
  Wasm, fonts, and images required by the site; runtime MDX remains prohibited.
- Treat response-only controls such as `frame-ancestors` and
  `X-Content-Type-Options` as a documented GitHub Pages platform limitation. Do not
  represent a meta directive as an equivalent response header.
- Gate npm and Cargo advisories, explicit license allowlists, dependency sources,
  and a pinned Gitleaks binary before the production build.
- Generate a deterministic CycloneDX 1.6 SBOM and SHA-256 manifest inside the exact
  Pages artifact. Verify complete checksum coverage before upload.
- Record the Pages artifact ID, protected-environment deployment ID, workflow run,
  commit, and deployed URL. Retain build and visual evidence for 30 days.
- Roll back by rerunning a known-good protected deployment rather than rewriting
  `main` or manually mutating the Pages branch.

## Consequences

The browser can enforce most static document restrictions, while HSTS remains a
GitHub Pages response control. A future host with configurable headers should move
the CSP to an HTTP response and add `frame-ancestors`, `nosniff`, Permissions Policy,
and a stricter script policy with nonces or hashes.
