import { expect, test } from '@playwright/test';

type BrowserVitals = { cls: number; lcp: number; tbt: number };

const vitalRoutes = [
  '/pmoke/en/',
  '/pmoke/en/docs/quickstart/',
  '/pmoke/ja/docs/quickstart/',
  '/pmoke/en/docs/configuration/validation/',
  '/pmoke/en/docs/interactive/waveform-analyzer/',
] as const;

for (const route of vitalRoutes) {
  test(`mobile browser vitals stay within budget: ${route}`, async ({ page }, testInfo) => {
    test.skip(testInfo.project.name !== 'mobile-chromium', 'single-profile Web Vitals gate');
    await installVitalsObserver(page);
    await page.goto(route, { waitUntil: 'load' });
    await page.evaluate(() => document.fonts.ready);
    await page.waitForTimeout(1_000);
    const vitals = await page.evaluate(() => (window as typeof window & { __pmokeVitals: BrowserVitals }).__pmokeVitals);
    expect(vitals.lcp, `${route} LCP`).toBeLessThanOrEqual(2_500);
    expect(vitals.cls, `${route} CLS`).toBeLessThanOrEqual(0.1);
    expect(vitals.tbt, `${route} TBT`).toBeLessThanOrEqual(200);
    testInfo.annotations.push({ type: 'web-vitals', description: JSON.stringify(vitals) });
  });
}

test('content remains operable at 200 percent zoom', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser zoom gate');
  await page.goto('/pmoke/ja/docs/quickstart/');
  await page.evaluate(() => {
    document.documentElement.style.zoom = '2';
  });
  await expect(page.getByRole('heading', { level: 1, name: 'クイックスタート' })).toBeVisible();
  expect(
    await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth + 1),
  ).toBeTruthy();
  await page.keyboard.press('Control+k');
  await expect(page.getByRole('textbox')).toBeFocused();
  await page.getByRole('textbox').fill('Kerr');
  await expect(page.getByText('概要', { exact: true }).last()).toBeVisible();
});

async function installVitalsObserver(page: import('@playwright/test').Page) {
  await page.addInitScript(() => {
    const vitals: BrowserVitals = { cls: 0, lcp: 0, tbt: 0 };
    Object.defineProperty(window, '__pmokeVitals', { value: vitals, writable: false });
    new PerformanceObserver((list) => {
      const entries = list.getEntries();
      const last = entries.at(-1);
      if (last) vitals.lcp = last.startTime;
    }).observe({ type: 'largest-contentful-paint', buffered: true });
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        const shift = entry as PerformanceEntry & { hadRecentInput?: boolean; value?: number };
        if (!shift.hadRecentInput) vitals.cls += shift.value ?? 0;
      }
    }).observe({ type: 'layout-shift', buffered: true });
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) vitals.tbt += Math.max(0, entry.duration - 50);
    }).observe({ type: 'longtask', buffered: true });
  });
}
