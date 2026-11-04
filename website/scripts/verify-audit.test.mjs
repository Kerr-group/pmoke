import assert from 'node:assert/strict';
import test from 'node:test';

import { evaluateAudit, matchesAllowedEntry } from './verify-audit.mjs';

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

test('accepts the second documented extract-zip advisory', () => {
  const secondAdvisory = {
    ...knownAdvisory,
    github_advisory_id: 'GHSA-7pqw-9j4j-h8q3',
  };
  const result = evaluateAudit({
    advisories: { first: knownAdvisory, second: secondAdvisory },
    metadata: { vulnerabilities: { high: 2 } },
  });
  assert.equal(result.accepted.length, 2);
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

const multiFindingAdvisory = {
  github_advisory_id: 'GHSA-multi-example',
  module_name: 'example',
  severity: 'moderate',
  vulnerable_versions: '<2.0.0',
  patched_versions: '>=2.0.0',
  findings: [
    {
      version: '1.0.0',
      paths: ['.>@tool-a>example'],
      dev: true,
      optional: false,
      bundled: false,
    },
    {
      version: '1.5.0',
      paths: ['.>@tool-b>example'],
      dev: true,
      optional: false,
      bundled: false,
    },
  ],
};

const multiFindingAllowed = {
  github_advisory_id: 'GHSA-multi-example',
  module_name: 'example',
  severity: 'moderate',
  vulnerable_versions: '<2.0.0',
  patched_versions: '>=2.0.0',
  findings: [...multiFindingAdvisory.findings].reverse(),
};

test('matches a multi-finding exception as an exact set regardless of order', () => {
  assert.equal(matchesAllowedEntry(multiFindingAdvisory, multiFindingAllowed), true);
});

test('rejects a multi-finding advisory with a missing finding', () => {
  const subset = {
    ...multiFindingAdvisory,
    findings: [multiFindingAdvisory.findings[0]],
  };
  assert.equal(matchesAllowedEntry(subset, multiFindingAllowed), false);
});

test('rejects a multi-finding advisory with an extra finding', () => {
  const superset = {
    ...multiFindingAdvisory,
    findings: [
      ...multiFindingAdvisory.findings,
      {
        version: '1.9.0',
        paths: ['.>@tool-c>example'],
        dev: true,
        optional: false,
        bundled: false,
      },
    ],
  };
  assert.equal(matchesAllowedEntry(superset, multiFindingAllowed), false);
});

test('rejects a multi-finding advisory with a drifted version or path', () => {
  const driftedVersion = {
    ...multiFindingAdvisory,
    findings: [{ ...multiFindingAdvisory.findings[0], version: '1.0.1' }, multiFindingAdvisory.findings[1]],
  };
  assert.equal(matchesAllowedEntry(driftedVersion, multiFindingAllowed), false);
  const driftedPath = {
    ...multiFindingAdvisory,
    findings: [{ ...multiFindingAdvisory.findings[0], paths: ['.>@tool-other>example'] }, multiFindingAdvisory.findings[1]],
  };
  assert.equal(matchesAllowedEntry(driftedPath, multiFindingAllowed), false);
});
