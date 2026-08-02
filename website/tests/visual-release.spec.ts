import { expect, test } from '@playwright/test';

const visualProjects = new Set(['desktop-chromium', 'mobile-chromium']);

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(!visualProjects.has(testInfo.project.name), 'Chromium visual contract');
  await page.addInitScript(() => window.localStorage.setItem('pmoke-theme', 'dark'));
  await page.emulateMedia({ reducedMotion: 'reduce' });
});

test('English home visual contract', async ({ page }, testInfo) => {
  await page.goto('/pmoke/en/');
  await expect(page.locator('.signal-stage')).toHaveAttribute('data-wasm', 'ready', { timeout: 20_000 });
  await expect(page).toHaveScreenshot(`home-${testInfo.project.name}.png`, {
    animations: 'disabled',
    fullPage: true,
    mask: [page.locator('canvas')],
    maskColor: '#0d1213',
    maxDiffPixelRatio: 0.015,
  });
});

test('Japanese quickstart visual contract', async ({ page }, testInfo) => {
  await page.goto('/pmoke/ja/docs/quickstart/');
  await expect(page.getByRole('heading', { level: 1, name: 'クイックスタート' })).toBeVisible();
  await expect(page).toHaveScreenshot(`quickstart-ja-${testInfo.project.name}.png`, {
    animations: 'disabled',
    fullPage: true,
    maxDiffPixelRatio: 0.015,
  });
});

test('config validator visual contract', async ({ page }, testInfo) => {
  await page.goto('/pmoke/en/docs/configuration/validation/');
  const validator = page.locator('.config-validator');
  await expect(validator.getByText('Valid config', { exact: true })).toBeVisible({ timeout: 20_000 });
  await expect(page).toHaveScreenshot(`config-validator-${testInfo.project.name}.png`, {
    animations: 'disabled',
    fullPage: true,
    maxDiffPixelRatio: 0.015,
  });
});
