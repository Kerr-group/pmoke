import { mkdir, readdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';

const resultsDirectory = path.resolve('.lighthouseci');
const evidenceDirectory = path.resolve('test-results/release-evidence');
const routeClasses = [
  { suffix: '/en/docs/quickstart/', className: 'content', performance: 0.95 },
  { suffix: '/ja/docs/quickstart/', className: 'content', performance: 0.95 },
  { suffix: '/en/', className: 'interactive-shell', performance: 0.9 },
  { suffix: '/en/docs/configuration/validation/', className: 'interactive-shell', performance: 0.9 },
  { suffix: '/en/docs/interactive/waveform-analyzer/', className: 'interactive-shell', performance: 0.9 },
];

const resultFiles = (await readdir(resultsDirectory))
  .filter((name) => name.startsWith('lhr-') && name.endsWith('.json'))
  .sort();
const results = await Promise.all(
  resultFiles.map(async (name) => JSON.parse(await readFile(path.join(resultsDirectory, name), 'utf8'))),
);
const failures = [];
const routes = [];

for (const route of routeClasses) {
  const runs = results.filter((result) => new URL(result.finalDisplayedUrl).pathname.endsWith(route.suffix));
  if (runs.length !== 3) {
    failures.push(`${route.suffix}: expected 3 Lighthouse runs, received ${runs.length}`);
    continue;
  }
  const summary = {
    route: route.suffix,
    class: route.className,
    runs: runs.length,
    performance: median(runs.map((result) => categoryScore(result, 'performance'))),
    accessibility: median(runs.map((result) => categoryScore(result, 'accessibility'))),
    best_practices: median(runs.map((result) => categoryScore(result, 'best-practices'))),
    seo: median(runs.map((result) => categoryScore(result, 'seo'))),
    lcp_ms: median(runs.map((result) => auditValue(result, 'largest-contentful-paint'))),
    cls: median(runs.map((result) => auditValue(result, 'cumulative-layout-shift'))),
    tbt_ms: median(runs.map((result) => auditValue(result, 'total-blocking-time'))),
  };
  routes.push(summary);
  checkAtLeast(summary, 'performance', route.performance, failures);
  for (const category of ['accessibility', 'best_practices', 'seo']) {
    checkAtLeast(summary, category, 0.95, failures);
  }
  checkAtMost(summary, 'lcp_ms', 2_500, failures);
  checkAtMost(summary, 'cls', 0.1, failures);
  checkAtMost(summary, 'tbt_ms', 200, failures);
}

const evidence = {
  schema: 1,
  profile: {
    runs_per_route: 3,
    form_factor: 'desktop',
    viewport: '1440x900@1x',
    throttling: 'Lighthouse simulated desktop',
  },
  routes,
};
await mkdir(evidenceDirectory, { recursive: true });
await writeFile(
  path.join(evidenceDirectory, 'lighthouse-summary.json'),
  `${JSON.stringify(evidence, null, 2)}\n`,
);
console.log(JSON.stringify(evidence, null, 2));
if (failures.length > 0) throw new Error(failures.join('; '));

function categoryScore(result, category) {
  const score = result.categories?.[category]?.score;
  if (!Number.isFinite(score)) throw new Error(`missing Lighthouse category: ${category}`);
  return score;
}

function auditValue(result, audit) {
  const value = result.audits?.[audit]?.numericValue;
  if (!Number.isFinite(value)) throw new Error(`missing Lighthouse audit: ${audit}`);
  return value;
}

function median(values) {
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.floor(sorted.length / 2)];
}

function checkAtLeast(summary, key, minimum, failures) {
  if (summary[key] < minimum) failures.push(`${summary.route}: ${key} ${summary[key]} is below ${minimum}`);
}

function checkAtMost(summary, key, maximum, failures) {
  if (summary[key] > maximum) failures.push(`${summary.route}: ${key} ${summary[key]} exceeds ${maximum}`);
}
