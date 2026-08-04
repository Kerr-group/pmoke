import { expect, test } from '@playwright/test';

const visualProjects = new Set(['desktop-chromium', 'mobile-chromium']);

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(!visualProjects.has(testInfo.project.name), 'Chromium AI resource contract');
  await page.addInitScript(() => window.localStorage.setItem('pmoke-theme', 'dark'));
});

test('AI resource hub exposes live manifest and bounded endpoint modes', async ({ page, context }, testInfo) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  const consoleErrors: string[] = [];
  const failedResponses: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text());
  });
  page.on('response', (response) => {
    if (response.status() >= 400) failedResponses.push(`${response.status()} ${response.url()}`);
  });

  await page.goto('/pmoke/en/docs/ai/');
  const hub = page.locator('.ai-hub');
  await expect(hub).toHaveAttribute('data-manifest', 'ready');
  await expect(hub.getByText('MANIFEST ONLINE', { exact: true })).toBeVisible();
  await expect(hub.locator('.ai-resource')).toHaveCount(2);

  await hub.getByRole('button', { name: 'Context', exact: true }).click();
  await expect(hub.locator('.ai-resource')).toHaveCount(3);
  await hub.getByRole('button', { name: 'Copy endpoint URL: English context' }).click();
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText()))
    .toBe('http://127.0.0.1:4173/pmoke/llms-en.txt');

  await hub.getByRole('button', { name: 'Contracts', exact: true }).click();
  await expect(hub.locator('.ai-resource')).toHaveCount(3);
  await expect(hub.getByRole('link', { name: 'Open resource: JSON Schema' }))
    .toHaveAttribute('href', '/pmoke/config.schema.json');

  const metrics = await hub.locator('.ai-hub__metrics dd').allTextContents();
  expect(Number(metrics[0])).toBeGreaterThanOrEqual(30);
  expect(metrics[1]).toBe('8');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1)).toBeTruthy();
  expect({ consoleErrors, failedResponses }).toEqual({ consoleErrors: [], failedResponses: [] });
  await page.screenshot({ path: testInfo.outputPath('ai-resource-hub.png'), fullPage: true });
});

test('Japanese resource hub stays localized and responsive', async ({ page }, testInfo) => {
  await page.goto('/pmoke/ja/docs/ai/');
  const hub = page.locator('.ai-hub');
  await expect(hub).toHaveAttribute('data-manifest', 'ready');
  await expect(hub.getByRole('button', { name: 'コンテキスト', exact: true })).toBeVisible();
  await expect(hub.getByText('最小コンテキスト優先', { exact: true })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1)).toBeTruthy();
  await page.screenshot({ path: testInfo.outputPath('ai-resource-hub-ja.png'), fullPage: true });
});

test('resource hub preserves static discovery when the manifest is unavailable', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser failure-mode gate');
  await page.route('**/ai-index.json', (route) => route.abort());
  await page.goto('/pmoke/en/docs/ai/');

  const hub = page.locator('.ai-hub');
  await expect(hub).toHaveAttribute('data-manifest', 'error');
  await expect(hub.getByText('STATIC LINKS ONLY', { exact: true })).toBeVisible();
  await expect(hub.locator('.ai-resource')).toHaveCount(2);
  await expect(hub.getByRole('link', { name: 'Open resource: llms.txt' }))
    .toHaveAttribute('href', '/pmoke/llms.txt');
});

test('machine feeds point to Markdown and publish an integrity manifest', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser machine feed gate');

  const indexResponse = await page.request.get('/pmoke/llms.txt');
  expect(indexResponse.ok()).toBeTruthy();
  const index = await indexResponse.text();
  expect(index).toContain('## Machine contracts');
  expect(index).toContain('## Optional');
  expect(index).toContain('https://kerr-group.github.io/pmoke/llm/en/quickstart.md');
  expect(index).not.toContain('https://kerr-group.github.io/pmoke/en/docs/quickstart/');

  const markdownResponse = await page.request.get('/pmoke/llm/en/quickstart.md');
  expect(markdownResponse.ok()).toBeTruthy();
  expect(markdownResponse.headers()['content-type']).toContain('text/markdown');

  const manifestResponse = await page.request.get('/pmoke/ai-index.json');
  const manifest = await manifestResponse.json();
  expect(manifest.schema).toBe(1);
  expect(manifest.resources).toHaveLength(8);
  expect(manifest.pages.length).toBeGreaterThanOrEqual(30);
  expect(manifest.pages.every((entry: { sha256: string }) => /^[a-f0-9]{64}$/u.test(entry.sha256))).toBeTruthy();
});
