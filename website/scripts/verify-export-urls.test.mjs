import assert from 'node:assert/strict';
import test from 'node:test';

import { hasExactUrl } from './verify-export-urls.mjs';

const expected = 'https://kerr-group.github.io/pmoke/en/docs/quickstart/';

test('accepts an exact HTML URL attribute', () => {
  assert.equal(hasExactUrl(`<link rel="canonical" href="${expected}">`, expected), true);
});

test('accepts an exact sitemap URL', () => {
  assert.equal(hasExactUrl(`<url>\n  <loc>${expected}</loc>\n</url>`, expected), true);
});

test('rejects a URL that only contains the expected URL in its path', () => {
  const malicious = `https://evil.example.invalid/redirect?target=${encodeURIComponent(expected)}`;
  assert.equal(hasExactUrl(`<meta content="${malicious}">`, expected), false);
});

test('rejects a lookalike host', () => {
  const malicious = 'https://kerr-group.github.io.evil.example.invalid/pmoke/en/docs/quickstart/';
  assert.equal(hasExactUrl(`<loc>${malicious}</loc>`, expected), false);
});
