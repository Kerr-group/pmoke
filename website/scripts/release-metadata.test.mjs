import assert from 'node:assert/strict';
import test from 'node:test';

import { licenseExpressionAllowed, licenseFailures } from './release-metadata.mjs';

test('SPDX OR accepts an allowed license branch', () => {
  assert.equal(licenseExpressionAllowed('MIT OR GPL-3.0-only'), true);
  assert.equal(licenseExpressionAllowed('GPL-3.0-only OR MIT'), true);
  assert.equal(licenseExpressionAllowed('MIT/Apache-2.0'), true);
  assert.equal(licenseExpressionAllowed('GPL-3.0-only / MIT'), true);
});

test('SPDX AND requires every license branch', () => {
  assert.equal(licenseExpressionAllowed('MIT AND Apache-2.0'), true);
  assert.equal(licenseExpressionAllowed('MIT AND GPL-3.0-only'), false);
});

test('SPDX precedence, parentheses, and exceptions are evaluated', () => {
  assert.equal(licenseExpressionAllowed('GPL-3.0-only OR (MIT AND Apache-2.0)'), true);
  assert.equal(licenseExpressionAllowed('(GPL-3.0-only OR MIT) AND Apache-2.0'), true);
  assert.equal(licenseExpressionAllowed('Apache-2.0 WITH LLVM-exception'), true);
  assert.equal(licenseExpressionAllowed('MIT WITH Classpath-exception-2.0'), false);
});

test('malformed or unsupported SPDX expressions are rejected', () => {
  assert.equal(licenseExpressionAllowed(''), false);
  assert.equal(licenseExpressionAllowed('MIT OR'), false);
  assert.equal(licenseExpressionAllowed('(MIT OR Apache-2.0'), false);
  assert.equal(licenseExpressionAllowed('MIT, Apache-2.0'), false);
});

test('Cargo inventory accepts a dual license when one branch is allowed', () => {
  const failures = licenseFailures({
    npm: {},
    cargo: [{ name: 'unescaper', version: '0.1.10', license: 'MIT OR GPL-3.0-only' }],
  });
  assert.deepEqual(failures, []);
});
