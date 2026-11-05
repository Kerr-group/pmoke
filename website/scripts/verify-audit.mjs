import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ALLOWED_ADVISORIES = Object.freeze([
  Object.freeze({
    // Temporary exact exception for the dev-only Lighthouse chain. Remove it
    // when a verifiable patched extract-zip release is available.
    github_advisory_id: 'GHSA-jmr9-qjv8-65gv',
    module_name: 'extract-zip',
    severity: 'high',
    vulnerable_versions: '<=2.0.1',
    patched_versions: '>=2.0.2',
    finding: Object.freeze({
      version: '2.0.1',
      paths: Object.freeze([
        '.>@lhci/cli>@lhci/utils>lighthouse>puppeteer-core>@puppeteer/browsers>extract-zip',
        '.>@lhci/cli>lighthouse>puppeteer-core>@puppeteer/browsers>extract-zip',
      ]),
      dev: true,
      optional: false,
      bundled: false,
    }),
  }),
  Object.freeze({
    // Temporary exact exception for the dev-only Lighthouse chain. No
    // patched extract-zip release exists (2.0.1 is still latest); remove it
    // when a verifiable patched release is available.
    github_advisory_id: 'GHSA-7pqw-9j4j-h8q3',
    module_name: 'extract-zip',
    severity: 'high',
    vulnerable_versions: '<=2.0.1',
    patched_versions: '>=2.0.2',
    finding: Object.freeze({
      version: '2.0.1',
      paths: Object.freeze([
        '.>@lhci/cli>@lhci/utils>lighthouse>puppeteer-core>@puppeteer/browsers>extract-zip',
        '.>@lhci/cli>lighthouse>puppeteer-core>@puppeteer/browsers>extract-zip',
      ]),
      dev: true,
      optional: false,
      bundled: false,
    }),
  }),
  Object.freeze({
    // Temporary exact exception for dev-only lint/Lighthouse tooling.
    // minimatch below 11.0.1 is unpatched; the only fixed release (11.0.1)
    // is a breaking major the eslint 8 chains cannot take. Only 10.2.6
    // remains in the revised advisory range; 3.1.5 is no longer listed.
    // Both the finding and its paths are dev-only and never shipped.
    // Remove it when the eslint chain moves to minimatch >= 11.0.1.
    github_advisory_id: "GHSA-g3hv-75wg-m249",
    module_name: "minimatch",
    severity: "moderate",
    vulnerable_versions: ">=7.0.0 <11.0.1",
    patched_versions: ">=11.0.1",
    finding: Object.freeze({
      version: "10.2.6",
      paths: Object.freeze([
        ".>eslint-config-next>eslint-import-resolver-typescript>eslint-plugin-import>@typescript-eslint/parser>@typescript-eslint/typescript-estree>minimatch",
        ".>eslint-config-next>eslint-import-resolver-typescript>eslint-plugin-import>eslint-module-utils>@typescript-eslint/parser>@typescript-eslint/typescript-estree>minimatch",
        ".>eslint-config-next>eslint-plugin-import>@typescript-eslint/parser>@typescript-eslint/typescript-estree>minimatch",
        ".>eslint-config-next>eslint-plugin-import>eslint-module-utils>@typescript-eslint/parser>@typescript-eslint/typescript-estree>minimatch",
        ".>eslint-config-next>typescript-eslint>@typescript-eslint/eslint-plugin>@typescript-eslint/parser>@typescript-eslint/typescript-estree>minimatch",
        ".>eslint-config-next>typescript-eslint>@typescript-eslint/eslint-plugin>@typescript-eslint/type-utils>@typescript-eslint/typescript-estree>minimatch",
        ".>eslint-config-next>typescript-eslint>@typescript-eslint/eslint-plugin>@typescript-eslint/type-utils>@typescript-eslint/utils>@typescript-eslint/typescript-estree>minimatch",
        ".>eslint-config-next>typescript-eslint>@typescript-eslint/eslint-plugin>@typescript-eslint/utils>@typescript-eslint/typescript-estree>minimatch",
        ".>eslint-config-next>typescript-eslint>@typescript-eslint/parser>@typescript-eslint/typescript-estree>minimatch",
        ".>eslint-config-next>typescript-eslint>@typescript-eslint/typescript-estree>minimatch",
        ".>eslint-config-next>typescript-eslint>@typescript-eslint/utils>@typescript-eslint/typescript-estree>minimatch",
      ]),
      dev: true,
      optional: false,
      bundled: false,
    }),
  }),
]);

function sorted(values) {
  return [...values].sort();
}

function matchesAllowedAdvisory(advisory) {
  return ALLOWED_ADVISORIES.some((allowed) => matchesAllowedEntry(advisory, allowed));
}

function matchesAllowedEntry(advisory, allowed) {
  if (
    advisory.github_advisory_id !== allowed.github_advisory_id ||
    advisory.module_name !== allowed.module_name ||
    advisory.severity !== allowed.severity ||
    advisory.vulnerable_versions !== allowed.vulnerable_versions ||
    advisory.patched_versions !== allowed.patched_versions ||
    advisory.findings?.length !== 1
  ) {
    return false;
  }

  const [finding] = advisory.findings;
  return (
    finding.version === allowed.finding.version &&
    finding.dev === allowed.finding.dev &&
    finding.optional === allowed.finding.optional &&
    finding.bundled === allowed.finding.bundled &&
    JSON.stringify(sorted(finding.paths)) === JSON.stringify(sorted(allowed.finding.paths))
  );
}

export function evaluateAudit(audit) {
  if (!audit || typeof audit !== 'object' || !audit.advisories || typeof audit.advisories !== 'object') {
    throw new Error('pnpm audit JSON does not contain an advisories object');
  }

  const advisories = Object.values(audit.advisories);
  const vulnerabilityCounts = audit.metadata?.vulnerabilities;
  if (!vulnerabilityCounts || typeof vulnerabilityCounts !== 'object') {
    throw new Error('pnpm audit JSON does not contain vulnerability counts');
  }
  const reportedAdvisoryCount = Object.values(vulnerabilityCounts).reduce(
    (total, count) => total + (Number.isInteger(count) ? count : 0),
    0,
  );
  if (reportedAdvisoryCount !== advisories.length) {
    throw new Error(
      `pnpm audit vulnerability count (${reportedAdvisoryCount}) does not match advisory records (${advisories.length})`,
    );
  }
  return {
    accepted: advisories.filter(matchesAllowedAdvisory),
    unexpected: advisories.filter((advisory) => !matchesAllowedAdvisory(advisory)),
  };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const auditPath = process.argv[2];
  if (!auditPath) throw new Error('Usage: node scripts/verify-audit.mjs <pnpm-audit.json>');

  const audit = JSON.parse(await readFile(auditPath, 'utf8'));
  const { accepted, unexpected } = evaluateAudit(audit);
  for (const advisory of accepted) {
    console.warn(
      `Accepted unresolved development-only advisory ${advisory.github_advisory_id} for ${advisory.module_name}@${advisory.findings[0].version}; monitor upstream for a published fix.`,
    );
  }
  if (unexpected.length > 0) {
    throw new Error(
      `Unexpected npm advisories: ${unexpected
        .map((advisory) => `${advisory.github_advisory_id ?? advisory.id} (${advisory.module_name})`)
        .join(', ')}`,
    );
  }
}
