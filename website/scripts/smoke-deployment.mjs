const deploymentUrl = process.argv[2];
const expectedCommit = process.argv[3];
if (!deploymentUrl) throw new Error('deployment URL is required');
if (!expectedCommit) throw new Error('expected commit SHA is required');

const root = new URL(deploymentUrl.endsWith('/') ? deploymentUrl : `${deploymentUrl}/`);
const checks = [
  {
    path: 'en/',
    contains: '<h1>pmoke</h1>',
    headers: { 'strict-transport-security': 'max-age=' },
  },
  { path: 'ja/', contains: '精密信号ラボ' },
  { path: 'en/docs/quickstart/', contains: 'Quickstart' },
  { path: 'ja/docs/quickstart/', contains: 'クイックスタート' },
  { path: 'en/docs/interactive/waveform-analyzer/', contains: 'Waveform Analyzer' },
  { path: 'ja/docs/interactive/waveform-analyzer/', contains: '波形アナライザー' },
  { path: 'en/docs/ai/', contains: 'Agent resource console' },
  { path: 'ja/docs/ai/', contains: 'エージェントリソースコンソール' },
  { path: 'en/docs/ai/search-privacy/', contains: 'Local retrieval' },
  { path: 'ja/docs/ai/search-privacy/', contains: 'Local検索' },
  { path: 'en/docs/citation/', contains: 'Reproducible attribution' },
  { path: 'ja/docs/citation/', contains: '再現可能な帰属情報' },
  { path: 'api/search', contains: 'Kerr' },
  { path: 'api/semantic-search', contains: 'pmoke-domain-v1' },
  { path: 'llms.txt', contains: 'https://kerr-group.github.io/pmoke/llm/en/' },
  { path: 'llms-en.txt', contains: '- locales: en' },
  { path: 'llms-ja.txt', contains: '- locales: ja' },
  { path: 'ai-index.json', contains: 'search_query_upload' },
  { path: 'sitemap.xml', contains: '<loc>https://kerr-group.github.io/pmoke/en/' },
  { path: 'robots.txt', contains: 'Sitemap: https://kerr-group.github.io/pmoke/sitemap.xml' },
  { path: 'wasm/pmoke_web_wasm_bg.wasm', contentType: 'application/wasm' },
  { path: 'workers/waveform-analyzer.worker.js', contains: 'max_total_harmonic_points' },
  { path: 'fixtures/m4-synthetic-reference.json', contains: 'CC0-1.0' },
  { path: '_meta/sbom.cdx.json', contains: `"version": "${expectedCommit}"` },
  { path: 'SHA256SUMS', contains: '_meta/sbom.cdx.json' },
  { path: '.nojekyll' },
];

for (const check of checks) {
  const url = new URL(check.path, root);
  let lastError;
  for (let attempt = 1; attempt <= 5; attempt += 1) {
    try {
      const response = await fetch(url, { cache: 'no-store' });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      if (check.contentType && !response.headers.get('content-type')?.includes(check.contentType)) {
        throw new Error(`expected ${check.contentType}, received ${response.headers.get('content-type')}`);
      }
      for (const [header, marker] of Object.entries(check.headers ?? {})) {
        if (!response.headers.get(header)?.includes(marker)) {
          throw new Error(`expected ${header} to contain ${marker}`);
        }
      }
      const body = check.contains || check.path.endsWith('/') ? await response.text() : '';
      if (check.contains && !body.includes(check.contains)) {
        throw new Error(`missing marker: ${check.contains}`);
      }
      if (check.path.endsWith('/') && (!body.includes('Content-Security-Policy') || !body.includes('strict-origin-when-cross-origin'))) {
        throw new Error('static security metadata is missing');
      }
      lastError = undefined;
      break;
    } catch (error) {
      lastError = error;
      if (attempt < 5) await new Promise((resolve) => setTimeout(resolve, attempt * 2_000));
    }
  }
  if (lastError) throw new Error(`deployment smoke failed for ${url}: ${lastError.message}`);
  console.log(`Verified ${url}`);
}
