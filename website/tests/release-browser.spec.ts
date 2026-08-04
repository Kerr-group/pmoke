import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';

test('release browser renders, searches, and navigates by keyboard', async ({ page }) => {
  await page.goto('/pmoke/en/docs/quickstart/');
  await expect(page.getByRole('heading', { level: 1, name: 'Quickstart' })).toBeVisible();
  expect(await hasHorizontalOverflow(page)).toBeFalsy();

  const search = page.getByRole('textbox');
  const fullSearchTrigger = page.locator('[data-search-full]:visible');
  const searchTrigger = (await fullSearchTrigger.count()) > 0
    ? fullSearchTrigger.first()
    : page.locator('[data-search]:visible').first();
  await searchTrigger.click();
  await expect(search).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(search).toBeHidden();

  await page.keyboard.press('Control+k');
  await expect(search).toBeFocused();
  await search.fill('lock-in filter');
  await expect(page.getByRole('option', { name: /Waveform Analyzer/u })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(search).toBeHidden();

  await page.keyboard.press('Tab');
  const focus = await page.evaluate(() => {
    const element = document.activeElement;
    if (!(element instanceof HTMLElement)) return null;
    const style = getComputedStyle(element);
    return { tag: element.tagName, outline: style.outlineStyle, width: element.getBoundingClientRect().width };
  });
  expect(focus?.tag).not.toBe('BODY');
  expect(focus?.width).toBeGreaterThan(0);
  expect(focus?.outline).not.toBe('none');
});

test('release browser executes the shared Wasm config validator', async ({ page }) => {
  const policyViolations: string[] = [];
  page.on('console', (message) => {
    if (message.text().includes('Content Security Policy')) policyViolations.push(message.text());
  });
  await page.goto('/pmoke/en/docs/configuration/validation/');
  const validator = page.locator('.config-validator');
  await expect(validator.getByText('Valid config', { exact: true })).toBeVisible({ timeout: 20_000 });
  await validator.getByRole('button', { name: 'Load diagnostic sample' }).click();
  await expect(validator.getByText('duplicate_channel', { exact: true })).toBeVisible();
  expect(await hasHorizontalOverflow(page)).toBeFalsy();

  const result = await new AxeBuilder({ page }).analyze();
  const blocking = result.violations.filter(
    (violation) => violation.impact === 'serious' || violation.impact === 'critical',
  );
  expect(blocking, blocking.map((violation) => violation.id).join(', ')).toEqual([]);
  expect(policyViolations).toEqual([]);
});

async function hasHorizontalOverflow(page: import('@playwright/test').Page) {
  return page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth + 1);
}
