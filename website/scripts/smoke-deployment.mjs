const deploymentUrl = process.argv[2];
if (!deploymentUrl) throw new Error('deployment URL is required');

const root = new URL(deploymentUrl.endsWith('/') ? deploymentUrl : `${deploymentUrl}/`);
const checks = [
  { path: 'en/', contains: '<h1>pmoke</h1>' },
  { path: 'ja/', contains: '精密信号ラボ' },
  { path: 'en/docs/quickstart/', contains: 'Quickstart' },
  { path: 'ja/docs/quickstart/', contains: 'クイックスタート' },
  { path: 'api/search', contains: 'Kerr' },
  { path: 'llms.txt', contains: 'https://kerr-group.github.io/pmoke/en/docs/' },
  { path: 'sitemap.xml', contains: '<loc>https://kerr-group.github.io/pmoke/en/' },
  { path: 'robots.txt', contains: 'Sitemap: https://kerr-group.github.io/pmoke/sitemap.xml' },
  { path: 'wasm/pmoke_web_wasm_bg.wasm', contentType: 'application/wasm' },
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
      if (check.contains && !(await response.text()).includes(check.contains)) {
        throw new Error(`missing marker: ${check.contains}`);
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
