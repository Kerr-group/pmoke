import { expect, test } from '@playwright/test';

for (const locale of ['en', 'ja'] as const) {
  test(`${locale} home renders a nonblank Wasm canvas`, async ({ page }, testInfo) => {
    await page.goto(`/pmoke/${locale}/`);
    await expect(page.locator('h1')).toHaveText('pmoke');
    await expect(page.locator('.signal-stage')).toHaveAttribute('data-wasm', 'ready', { timeout: 15_000 });
    const pixels = await page.locator('canvas').evaluate((canvas: HTMLCanvasElement) => {
      const context = canvas.getContext('2d');
      if (!context) return 0;
      const data = context.getImageData(0, 0, canvas.width, canvas.height).data;
      let nonTransparent = 0;
      for (let index = 3; index < data.length; index += 64) if (data[index] > 0) nonTransparent += 1;
      return nonTransparent;
    });
    expect(pixels).toBeGreaterThan(100);
    await expect(page.locator('.hero-copy')).toBeInViewport();
    const payload = await page.evaluate(() => {
      const resources = performance.getEntriesByType('resource') as PerformanceResourceTiming[];
      return resources.reduce(
        (total, resource) => ({
          bytes: total.bytes + resource.transferSize,
          scriptBytes: total.scriptBytes + (resource.initiatorType === 'script' ? resource.transferSize : 0),
        }),
        { bytes: 0, scriptBytes: 0 },
      );
    });
    expect(payload.bytes).toBeLessThan(2_000_000);
    expect(payload.scriptBytes).toBeLessThan(1_200_000);
    expect(payload.bytes).toBeGreaterThan(100_000);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBeTruthy();
    const capabilityTop = await page.locator('.capability-grid').evaluate((element) => element.getBoundingClientRect().top);
    expect(capabilityTop).toBeLessThan(page.viewportSize()?.height ?? 0);
    testInfo.annotations.push({ type: 'payload', description: JSON.stringify(payload) });
    await page.screenshot({ path: testInfo.outputPath(`${locale}-home.png`), fullPage: true });
  });
}

test('deep documentation links and static search remain under the project base path', async ({ page }, testInfo) => {
  await page.goto('/pmoke/ja/docs/quickstart/');
  await expect(page.getByRole('heading', { level: 1, name: 'クイックスタート' })).toBeVisible();
  await page.keyboard.press('Control+k');
  const searchInput = page.getByRole('textbox');
  await searchInput.fill('Kerr');
  await expect(page.getByText('概要', { exact: true }).last()).toBeVisible();
  await searchInput.fill('lockin.filter.iir_order');
  await expect(page.getByText('設定リファレンス', { exact: true }).last()).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('textbox')).toBeHidden();
  const response = await page.request.get('/pmoke/api/search');
  expect(response.ok()).toBeTruthy();
  expect(await response.text()).toContain('クイックスタート');
  const llms = await page.request.get('/pmoke/llms.txt');
  expect(await llms.text()).toContain('https://kerr-group.github.io/pmoke/llm/ja/');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBeTruthy();
  await page.screenshot({ path: testInfo.outputPath('ja-docs.png'), fullPage: true });
});
