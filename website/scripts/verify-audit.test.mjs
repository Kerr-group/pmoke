import assert from 'node:assert/strict';
import test from 'node:test';

import { evaluateAudit } from './verify-audit.mjs';

const knownAdvisory = {
  github_advisory_id: 'GHSA-jmr9-qjv8-65gv',
  module_name: 'extract-zip',
  severity: 'high',
  vulnerable_versions: '<=2.0.1',
  patched_versions: '>=2.0.2',
  findings: [
    {
      version: '2.0.1',
      paths: [
        '.>@lhci/cli>@lhci/utils>lighthouse>puppeteer-core>@puppeteer/browsers>extract-zip',
        '.>@lhci/cli>lighthouse>puppeteer-core>@puppeteer/browsers>extract-zip',
      ],
      dev: true,
      optional: false,
      bundled: false,
    },
  ],
};

test('accepts only the documented unresolved development advisory', () => {
  const result = evaluateAudit({
    advisories: { known: knownAdvisory },
    metadata: { vulnerabilities: { high: 1 } },
  });
  assert.equal(result.accepted.length, 1);
  assert.equal(result.unexpected.length, 0);
});

test('accepts an audit with no advisories', () => {
  const result = evaluateAudit({ advisories: {}, metadata: { vulnerabilities: {} } });
  assert.deepEqual(result, { accepted: [], unexpected: [] });
});

test('rejects a new advisory', () => {
  const result = evaluateAudit({
    advisories: {
      known: knownAdvisory,
      unexpected: { ...knownAdvisory, github_advisory_id: 'GHSA-unexpected' },
    },
    metadata: { vulnerabilities: { high: 2 } },
  });
  assert.equal(result.accepted.length, 1);
  assert.equal(result.unexpected.length, 1);
});

test('rejects a changed dependency path or version', () => {
  const result = evaluateAudit({
    advisories: {
      changed: {
        ...knownAdvisory,
        findings: [{ ...knownAdvisory.findings[0], version: '2.0.2' }],
      },
    },
    metadata: { vulnerabilities: { high: 1 } },
  });
  assert.equal(result.accepted.length, 0);
  assert.equal(result.unexpected.length, 1);
});

test('rejects inconsistent audit counts', () => {
  assert.throws(
    () => evaluateAudit({ advisories: { known: knownAdvisory }, metadata: { vulnerabilities: { high: 0 } } }),
    /does not match advisory records/,
  );
});
