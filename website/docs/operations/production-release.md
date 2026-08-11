# Production documentation release

## Required gates

The `Documentation site` workflow blocks upload and deployment until all of these
gates pass:

- Rust native/Wasm tests, clippy, generated-reference drift, and cargo-deny
- Gitleaks 8.30.1, full npm audit, and explicit Cargo/npm license policy
- lint, type checking, bilingual content, search relevance, AI resources, and export
- Wasm payload budgets: raw output up to 640 KiB for host-specific build variance,
  with a stricter 210 KiB gzip limit for transfer size
- Chromium at four viewport sizes plus representative Firefox and WebKit flows
- axe serious/critical count of zero, keyboard focus, reduced motion, JavaScript-off,
  200% zoom proxy, and Wasm/search failure recovery
- tracked desktop/mobile visual baselines
- three-run median Lighthouse gates and mobile browser Web Vitals budgets
- complete CycloneDX SBOM and SHA-256 artifact manifest

The workflow uploads the exact `website/out` directory, including `.nojekyll`, and
retains the deployable artifact and visual/Lighthouse evidence for 30 days.

## Manual accessibility evidence

Before the M6 production approval, record the browser, operating system, assistive
technology, date, and result in the pull request for this checklist:

1. Navigate home, Quickstart, search, config validation, and waveform analysis using
   only Tab, Shift+Tab, Enter, Space, Escape, and arrow keys.
2. Confirm a visible focus indicator and logical focus order at every step.
3. With a screen reader, confirm headings and landmarks, search dialog naming,
   validation status/error announcements, plot summaries, and control labels.
4. At browser zoom 200%, confirm text reflow, no page-wide horizontal scroll, and no
   obscured controls at 1440x900 and 360x800 CSS viewports.
5. With reduced motion enabled, confirm the signal scene is static and all tools
   remain operable.

Automated axe, accessibility-tree, keyboard, zoom, and reduced-motion checks support
this review but do not replace the screen-reader pass.

## Release evidence

The deploy job writes these identifiers to the GitHub Actions summary:

- source commit SHA
- workflow run ID and attempt
- Pages artifact ID
- `github-pages` environment deployment ID
- production URL

The post-deploy job checks localized deep links, search indexes, LLM resources, Wasm
MIME, Worker assets, HSTS, static CSP/referrer metadata, SBOM commit identity,
checksums, and `.nojekyll`.

## Rollback

1. Find the newest earlier `Documentation site` run on `main` whose build, deploy,
   and post-deploy jobs are all green.
2. Verify its commit and artifact/deployment identifiers in the run summary.
3. Re-run all jobs for that workflow run:

   ```bash
   gh run rerun <known-good-run-id>
   gh run watch <new-run-id> --exit-status
   ```

4. Confirm approval in the protected `github-pages` environment when required.
5. Verify the new run summary and production smoke job. Record the incident, old and
   new deployment IDs, rolled-back commit, operator, and reason.

For a tagged release created after M6, a protected manual deployment is also valid:

```bash
gh workflow run docs-site.yml --ref <known-good-tag> -f deploy=true
```

Never force-push `main`, edit generated Pages content manually, or deploy an artifact
whose checksum/SBOM gate did not pass.

## Platform limitation

GitHub Pages supplies HTTPS and HSTS but does not provide repository-controlled
arbitrary response headers. The static meta CSP cannot express `frame-ancestors`,
and meta tags cannot provide `X-Content-Type-Options` or Permissions Policy. These
limitations are accepted in ADR 0004 and must be re-evaluated if hosting changes.
