import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';

for (const item of [
  {
    locale: 'en',
    query: 'connect an oscilloscope over the network',
    mode: 'Local hybrid',
    expectedFirst: 'Hardware Transports',
  },
  {
    locale: 'ja',
    query: 'ネットワーク経由のオシロ接続',
    mode: 'ローカル・ハイブリッド',
    expectedFirst: 'Hardware Transport',
  },
] as const) {
  test(`${item.locale} local hybrid search stays private and readable`, async ({ page }, testInfo) => {
    const searchRequests: string[] = [];
    let capture = false;
    page.on('request', (request) => {
      if (capture) searchRequests.push(request.url());
    });
    await page.goto(`/pmoke/${item.locale}/docs/ai/search-privacy/`);
    await page.keyboard.press('Control+k');
    capture = true;
    await page.getByRole('textbox').fill(item.query);
    const status = page.locator('[data-search-mode]');
    await expect(status).toHaveAttribute('data-search-mode', 'hybrid', { timeout: 10_000 });
    await expect(status.getByText(item.mode, { exact: true })).toBeVisible();
    const resultButtons = page.locator('#fd-search-dialog-content [role="option"]');
    await expect(resultButtons.first()).toBeVisible();
    await expect(resultButtons.first()).toContainText(item.expectedFirst);
    const resultText = await resultButtons.allTextContents();
    expect(resultText.length).toBeGreaterThan(1);
    expect(resultText.every((value) => value.length <= 520)).toBeTruthy();
    expect(
      searchRequests.every((url) => new URL(url).origin === 'http://127.0.0.1:4173'),
    ).toBeTruthy();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBeTruthy();
    await page.screenshot({ path: testInfo.outputPath(`${item.locale}-hybrid-search.png`), fullPage: true });
  });
}

test('semantic index failure preserves locale-scoped full-text search', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser failure gate');
  await page.route('**/api/semantic-search', (route) => route.abort('failed'));
  await page.goto('/pmoke/ja/docs/quickstart/');
  await page.keyboard.press('Control+k');
  const input = page.getByRole('textbox');
  await input.fill('lockin.filter.iir_order');
  const status = page.locator('[data-search-mode]');
  await expect(status).toHaveAttribute('data-search-mode', 'fallback', { timeout: 10_000 });
  await expect(status.getByText('全文検索フォールバック', { exact: true })).toBeVisible();
  await expect(page.getByText('設定リファレンス', { exact: true }).last()).toBeVisible();
  await expect(input).toHaveValue('lockin.filter.iir_order');
  await page.screenshot({ path: testInfo.outputPath('ja-search-fallback.png'), fullPage: true });
});

test('hybrid search dialog remains keyboard operable and accessible', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser accessibility gate');
  await page.goto('/pmoke/en/docs/');
  await page.keyboard.press('Control+k');
  const input = page.getByRole('textbox');
  await input.fill('local CSV phase rotation');
  await expect(page.locator('[data-search-mode]')).toHaveAttribute('data-search-mode', 'hybrid', {
    timeout: 10_000,
  });
  const accessibility = await new AxeBuilder({ page }).include('#fd-search-dialog-content').analyze();
  const serious = accessibility.violations.filter((violation) =>
    ['serious', 'critical'].includes(violation.impact ?? ''),
  );
  expect(serious, JSON.stringify(serious, null, 2)).toEqual([]);
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('Escape');
  await expect(input).toBeHidden();
  testInfo.annotations.push({ type: 'axe', description: 'zero serious/critical hybrid-search violations' });
});
