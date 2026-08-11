import { performance } from 'node:perf_hooks';
import { readFile } from 'node:fs/promises';
import { createHybridEngine } from '../lib/search/hybrid-engine.mjs';
import { MODEL_ID } from '../lib/search/concept-model.mjs';

const index = JSON.parse(await readFile(new URL('../out/api/semantic-search', import.meta.url), 'utf8'));
const fixture = JSON.parse(await readFile(new URL('../tests/fixtures/search-relevance-v1.json', import.meta.url), 'utf8'));
if (index.model !== MODEL_ID || fixture.model !== MODEL_ID) throw new Error('search model metadata mismatch');

const MIN_RECALL_AT_5 = 0.95;
const report = {
  model: MODEL_ID,
  quality_gate: { minimum_recall_at_5: MIN_RECALL_AT_5 },
  locales: {},
  overall: { hits: 0, queries: 0, recall_at_5: 0 },
};
const failures = [];
for (const locale of ['en', 'ja']) {
  const records = index.records.filter((record) => record.locale === locale);
  const queries = fixture.queries.filter((item) => item.locale === locale);
  if (queries.length < 40) throw new Error(`${locale} relevance fixture must contain at least 40 queries`);
  const started = performance.now();
  const engine = createHybridEngine(records);
  const readinessMs = performance.now() - started;
  engine.search(queries[0].query, 5);
  const latencies = [];
  const misses = [];
  let hits = 0;
  for (const item of queries) {
    const queryStarted = performance.now();
    const results = await engine.search(item.query, 5);
    latencies.push(performance.now() - queryStarted);
    if (results.some((result) => result.record.locale !== locale)) {
      throw new Error(`${locale} query leaked a cross-locale result: ${item.query}`);
    }
    const paths = results.map((result) => result.record.url.split('#')[0]);
    if (paths.includes(item.expected)) hits += 1;
    else misses.push({ query: item.query, expected: item.expected, actual: paths });
  }
  latencies.sort((left, right) => left - right);
  const recall = hits / queries.length;
  const p95 = latencies[Math.ceil(latencies.length * 0.95) - 1];
  report.locales[locale] = {
    hits,
    queries: queries.length,
    recall_at_5: recall,
    p95_ms: p95,
    readiness_ms: readinessMs,
    misses,
  };
  report.overall.hits += hits;
  report.overall.queries += queries.length;
  if (recall < MIN_RECALL_AT_5) {
    failures.push(`${locale} Recall@5 ${recall.toFixed(3)} is below ${MIN_RECALL_AT_5}`);
  }
  if (p95 > 100) failures.push(`${locale} warm p95 ${p95.toFixed(2)} ms exceeds 100 ms`);
  if (readinessMs > 1_000) failures.push(`${locale} readiness ${readinessMs.toFixed(2)} ms exceeds 1,000 ms`);
}
report.overall.recall_at_5 = report.overall.hits / report.overall.queries;
if (report.overall.recall_at_5 < MIN_RECALL_AT_5) {
  failures.push(`overall Recall@5 is below ${MIN_RECALL_AT_5}`);
}
console.log(JSON.stringify(report, null, 2));
if (failures.length > 0) throw new Error(failures.join('; '));
