import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';

const locales = ['en', 'ja'] as const;
const themes = ['dark', 'light'] as const;

for (const locale of locales) {
  for (const theme of themes) {
    test(`${locale} ${theme} foundation remains responsive`, async ({ page }, testInfo) => {
      await page.addInitScript((selectedTheme) => {
        window.localStorage.setItem('pmoke-theme', selectedTheme);
      }, theme);

      await page.goto(`/pmoke/${locale}/`);
      await expect(page.locator('html')).toHaveClass(new RegExp(`(^|\\s)${theme}(\\s|$)`));
      await expect(page.locator('h1')).toHaveText('pmoke');
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBeTruthy();
      await page.screenshot({ path: testInfo.outputPath(`${locale}-${theme}-home.png`), fullPage: true });

      await page.goto(`/pmoke/${locale}/docs/quickstart/`);
      await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBeTruthy();
      await page.screenshot({ path: testInfo.outputPath(`${locale}-${theme}-docs.png`), fullPage: true });
    });
  }
}

test('representative routes have no serious accessibility violations', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser accessibility gate');

  for (const route of [
    '/pmoke/en/',
    '/pmoke/ja/',
    '/pmoke/en/docs/quickstart/',
    '/pmoke/ja/docs/quickstart/',
    '/pmoke/en/docs/ai/',
    '/pmoke/ja/docs/ai/',
    '/pmoke/en/docs/citation/',
    '/pmoke/ja/docs/citation/',
    '/pmoke/en/docs/configuration/validation/',
  ]) {
    await page.goto(route);
    const result = await new AxeBuilder({ page }).analyze();
    const blocking = result.violations.filter(
      (violation) => violation.impact === 'serious' || violation.impact === 'critical',
    );
    expect(blocking, `${route}: ${blocking.map((violation) => violation.id).join(', ')}`).toEqual([]);
  }
});

test('citation reference is copyable and mobile-safe', async ({ page, context }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser citation gate');

  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.setViewportSize({ width: 320, height: 800 });
  await page.goto('/pmoke/en/docs/citation/');
  const panel = page.getByRole('region', { name: 'pmoke software citation' });
  await expect(panel).toBeVisible();
  await expect(panel.locator('.citation-panel__metadata dd').first()).toHaveText(/^\d+\.\d+\.\d+(?:[-+][\w.-]+)?$/u);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth + 1))
    .toBeTruthy();

  await panel.getByRole('button', { name: 'Copy BibTeX citation' }).click();
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toContain('@misc{kerr_group_pmoke_2026');
  await expect(page.getByText('BibTeX citation copied')).toBeAttached();
});

test('config validator stays responsive and reports canonical v5 diagnostics', async ({ page }, testInfo) => {
  await page.goto('/pmoke/en/docs/configuration/validation/');
  const validator = page.locator('.config-validator');
  await expect(validator).toBeVisible();
  await expect(validator.getByText('Valid config', { exact: true })).toBeVisible({ timeout: 15_000 });
  await expect(validator.getByText('1 / 2 / 3', { exact: true })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBeTruthy();

  await validator.getByRole('button', { name: 'Load diagnostic sample' }).click();
  await expect(validator.getByText('Config errors', { exact: true })).toBeVisible();
  await expect(validator.getByText('duplicate_channel', { exact: true })).toBeVisible();
  await expect(validator.getByText('invalid_range', { exact: true }).first()).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath('config-validator.png'), fullPage: true });
});

test('config validator recovers from a Wasm load failure without losing input', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser failure-mode gate');

  await page.route('**/wasm/pmoke_web_wasm.js', (route) => route.abort());
  await page.goto('/pmoke/ja/docs/configuration/validation/');
  const validator = page.locator('.config-validator');
  await expect(validator.getByText('検証機能は利用不可', { exact: true })).toBeVisible();
  await expect(validator.getByLabel('設定入力')).toHaveValue(/version = 5/u);

  await page.unroute('**/wasm/pmoke_web_wasm.js');
  await validator.getByRole('button', { name: 'Wasmを再読み込み' }).click();
  await expect(validator.getByText('有効な設定', { exact: true })).toBeVisible({ timeout: 15_000 });
});

test('config validator handles the input cap off the main thread', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser size-limit gate');

  await page.goto('/pmoke/en/docs/configuration/validation/');
  const validator = page.locator('.config-validator');
  await validator.getByLabel('Configuration input').fill('x'.repeat(1_048_577));
  await expect(validator.getByText('input_too_large', { exact: true })).toBeVisible();
  const validSample = validator.getByRole('button', { name: 'Load valid sample' });
  await expect(validSample).toBeEnabled();
  await validSample.click();
  await expect(validator.getByText('Valid config', { exact: true })).toBeVisible();
});

test('documentation language selection preserves the current slug', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser interaction gate');

  await page.goto('/pmoke/en/docs/quickstart/');
  await page.getByRole('button', { name: 'Choose a language' }).click();
  await page.getByRole('button', { name: '日本語' }).click();
  await expect(page).toHaveURL(/\/pmoke\/ja\/docs\/quickstart\/$/);
  await expect(page.getByRole('heading', { level: 1, name: 'クイックスタート' })).toBeVisible();
});

test('home theme selection persists into documentation', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser interaction gate');

  const scriptWarnings: string[] = [];
  page.on('console', (message) => {
    if (message.text().includes('Encountered a script tag while rendering React component')) {
      scriptWarnings.push(message.text());
    }
  });
  await page.addInitScript((selectedTheme) => {
    if (!window.localStorage.getItem('pmoke-theme')) {
      window.localStorage.setItem('pmoke-theme', selectedTheme);
    }
  }, 'dark');
  await page.goto('/pmoke/en/');
  await expect(page.locator('.theme-toggle')).toHaveAttribute('aria-label', 'Switch to light theme');
  await page.locator('.theme-toggle').click();
  await expect(page.locator('html')).toHaveClass(/(^|\s)light(\s|$)/);
  await page.goto('/pmoke/en/docs/');
  await expect(page.locator('html')).toHaveClass(/(^|\s)light(\s|$)/);
  expect(scriptWarnings).toEqual([]);
});

test('documentation remains readable without JavaScript', async ({ browser }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser progressive-enhancement gate');

  const context = await browser.newContext({ javaScriptEnabled: false });
  const page = await context.newPage();
  await page.goto('http://127.0.0.1:4173/pmoke/ja/');
  await expect(page.getByRole('heading', { level: 1, name: 'pmoke' })).toBeVisible();
  await expect(page.locator('.signal-sequence')).toContainText('磁場パルス');
  await expect(page.locator('#signal-description')).toContainText('パルス磁場MOKEの概念図');
  await page.goto('http://127.0.0.1:4173/pmoke/ja/docs/quickstart/');
  await expect(page.getByRole('heading', { level: 1, name: 'クイックスタート' })).toBeVisible();
  await expect(page.locator('code').filter({ hasText: 'pmoke config init' })).toBeVisible();
  await context.close();
});

test('code blocks keep one chrome and localized keyboard-scrollable viewports', async ({ page, context }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser code-block contract');

  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.setViewportSize({ width: 320, height: 800 });

  for (const [locale, name] of [['en', 'Code example'], ['ja', 'コード例']] as const) {
    await page.goto(`/pmoke/${locale}/docs/quickstart/`);
    const figure = page.locator('figure.shiki').first();
    const viewport = figure.getByRole('region', { name });
    const pre = viewport.locator('pre');

    await expect(figure).toBeVisible();
    await expect(viewport).toBeVisible();
    const chrome = await figure.evaluate((element) => {
      const outer = getComputedStyle(element);
      const inner = getComputedStyle(element.querySelector('pre')!);
      return {
        outerBorder: Number.parseFloat(outer.borderTopWidth),
        innerBorders: [inner.borderTopWidth, inner.borderRightWidth, inner.borderBottomWidth, inner.borderLeftWidth]
          .map(Number.parseFloat),
        maxHeight: getComputedStyle(element.querySelector('[role="region"]')!).maxHeight,
        overflow: getComputedStyle(element.querySelector('[role="region"]')!).overflow,
      };
    });
    expect(chrome.outerBorder).toBe(1);
    expect(chrome.innerBorders).toEqual([0, 0, 0, 0]);
    expect(chrome.maxHeight).toBe('600px');
    expect(chrome.overflow).toBe('auto');
    expect(await viewport.evaluate((element) => element.scrollWidth > element.clientWidth)).toBeTruthy();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth + 1))
      .toBeTruthy();

    await viewport.focus();
    await expect(viewport).toBeFocused();
    for (let step = 0; step < 8; step += 1) await page.keyboard.press('ArrowRight');
    const rightScroll = await viewport.evaluate((element) => element.scrollLeft);
    expect(rightScroll).toBeGreaterThan(0);
    await page.keyboard.press('ArrowLeft');
    expect(await viewport.evaluate((element) => element.scrollLeft)).toBeLessThan(rightScroll);

    await figure.getByRole('button', { name: locale === 'ja' ? 'テキストをコピー' : 'Copy Text' }).click();
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toContain('pmoke config init');
  }
});

test('signal canvas sweeps continuously when active and pauses on reduced motion', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser motion contract');

  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await page.goto('/pmoke/en/');
  await expect(page.locator('.signal-stage')).toHaveAttribute('data-wasm', 'ready', { timeout: 15_000 });
  await expect(page.locator('.signal-stage')).toHaveAttribute('data-motion', 'running');

  const firstFrame = await page.locator('.signal-stage canvas').evaluate((canvas: HTMLCanvasElement) => canvas.toDataURL());
  await page.waitForTimeout(300);
  const secondFrame = await page.locator('.signal-stage canvas').evaluate((canvas: HTMLCanvasElement) => canvas.toDataURL());
  expect(secondFrame).not.toBe(firstFrame);

  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.reload();
  await expect(page.locator('.signal-stage')).toHaveAttribute('data-wasm', 'ready', { timeout: 15_000 });
  await expect(page.locator('.signal-stage')).toHaveAttribute('data-motion', 'reduced');

  const firstStatic = await page.locator('.signal-stage canvas').evaluate((canvas: HTMLCanvasElement) => canvas.toDataURL());
  await page.waitForTimeout(300);
  const secondStatic = await page.locator('.signal-stage canvas').evaluate((canvas: HTMLCanvasElement) => canvas.toDataURL());
  expect(secondStatic).toBe(firstStatic);
  await expect(page.getByRole('button', { name: 'Static view (reduced motion)' })).toBeDisabled();
});

test('signal hero exposes localized sequence semantics and a user pause', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser signal semantics contract');

  await page.emulateMedia({ reducedMotion: 'no-preference' });
  for (const [locale, labels, description] of [
    [
      'en',
      ['FIELD PULSE', 'REFERENCE + RESPONSE', 'LOCK-IN X / Y', 'ROTATE PHASE', 'KERR ANGLE'],
      'Illustrative pulsed-field MOKE pipeline',
    ],
    [
      'ja',
      ['磁場パルス', '参照信号 + Kerr応答', 'ロックイン X / Y', '位相回転', 'Kerr角'],
      'パルス磁場MOKEの概念図',
    ],
  ] as const) {
    await page.goto(`/pmoke/${locale}/`);
    const stage = page.locator('.signal-stage');
    const canvas = stage.locator('canvas');
    await expect(stage).toHaveAttribute('data-wasm', 'ready', { timeout: 15_000 });
    await expect(stage).toHaveAttribute('data-motion', 'running');
    await expect(canvas).toHaveAttribute(
      'aria-label',
      locale === 'en' ? 'Illustrative pulsed-field MOKE signal' : 'パルス磁場MOKEの説明図',
    );
    await expect(canvas).toHaveAttribute('aria-describedby', 'signal-description');
    await expect(page.locator('#signal-description')).toContainText(description);
    await expect(stage.locator('.signal-sequence li')).toHaveText(labels);
    expect(await stage.locator('.signal-sequence li').evaluateAll((items) => items.map((item) => item.dataset.step))).toEqual([
      'field-pulse',
      'waveforms',
      'lock-in',
      'rotate-phase',
      'kerr-angle',
    ]);
    await expect(page.getByText(locale === 'en' ? 'ACQUISITION WINDOW' : '取得窓', { exact: true })).toHaveCount(0);
    await expect(page.locator('#signal-description')).toContainText(locale === 'en' ? 'triggered measurement window' : 'トリガー窓');
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBeTruthy();
    const layout = await page.evaluate(() => {
      const bottom = (selector: string) => document.querySelector(selector)?.getBoundingClientRect().bottom ?? 0;
      const panelTop = document.querySelector('.hero-copy-panel')?.getBoundingClientRect().top ?? 0;
      return { panelTop, sequenceBottom: bottom('.signal-sequence'), controlsBottom: bottom('.signal-controls') };
    });
    expect(layout.panelTop).toBeGreaterThan(layout.sequenceBottom);
    expect(layout.panelTop).toBeGreaterThan(layout.controlsBottom);

    if (locale === 'en') {
      const control = stage.locator('.signal-control');
      await expect(control).toHaveAttribute('aria-label', 'Pause animation');
      await expect(control).toHaveAttribute('aria-pressed', 'false');
      await control.click();
      await expect(stage).toHaveAttribute('data-motion', 'paused');
      await expect(stage).toHaveAttribute('data-user-paused', 'true');
      const firstPaused = await canvas.evaluate((element: HTMLCanvasElement) => element.toDataURL());
      await page.waitForTimeout(300);
      const secondPaused = await canvas.evaluate((element: HTMLCanvasElement) => element.toDataURL());
      expect(secondPaused).toBe(firstPaused);

      await page.evaluate(() => document.dispatchEvent(new Event('visibilitychange')));
      await expect(control).toHaveAttribute('aria-pressed', 'true');
      await expect(stage).toHaveAttribute('data-motion', 'paused');

      await expect(control).toHaveAttribute('aria-label', 'Resume animation');
      await control.click();
      await expect(stage).toHaveAttribute('data-motion', 'running');
      await expect(stage).toHaveAttribute('data-user-paused', 'false');
    }
  }
});

test('signal hero reports an informative static fallback when Wasm is unavailable', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser signal fallback contract');

  await page.route('**/wasm/pmoke_web_wasm.js', (route) => route.abort());
  await page.goto('/pmoke/ja/');
  const stage = page.locator('.signal-stage');
  await expect(stage).toHaveAttribute('data-wasm', 'fallback', { timeout: 15_000 });
  await expect(stage).toHaveAttribute('data-motion', 'paused');
  await expect(stage.locator('.signal-status')).toHaveText('静的フォールバック');
  await expect(page.getByRole('button', { name: '静的フォールバック（WASM利用不可）' })).toBeDisabled();
  await expect(page.getByText('WASM ONLINE', { exact: true })).toHaveCount(0);
  await expect(stage.locator('.signal-sequence li')).toHaveText(['磁場パルス', '参照信号 + Kerr応答', 'ロックイン X / Y', '位相回転', 'Kerr角']);
});

test('signal renderer survives reload and Wasm readiness without restarting', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser renderer lifecycle gate');

  await page.route('**/wasm/pmoke_web_wasm.js', async (route) => {
    await new Promise((resolve) => setTimeout(resolve, 800));
    await route.continue();
  });
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await page.goto('/pmoke/en/');
  await expect(page.locator('.signal-stage')).toHaveAttribute('data-wasm', 'ready', { timeout: 15_000 });

  await page.reload();
  const stage = page.locator('.signal-stage');
  const canvas = stage.locator('canvas');
  await expect(stage).toHaveAttribute('data-wasm', 'loading');
  await expect(stage).toHaveAttribute('data-motion', 'running');
  await expect(canvas).toHaveAttribute('data-render-generation', '1');
  expect(await canvas.evaluate((element: HTMLCanvasElement) => {
    const context = element.getContext('2d');
    if (!context) return 0;
    const pixels = context.getImageData(0, 0, element.width, element.height).data;
    let painted = 0;
    for (let index = 3; index < pixels.length; index += 128) if (pixels[index] > 0) painted += 1;
    return painted;
  })).toBeGreaterThan(100);

  await expect(stage).toHaveAttribute('data-wasm', 'ready', { timeout: 15_000 });
  await expect(canvas).toHaveAttribute('data-render-generation', '1');
  await page.locator('.theme-toggle').click();
  await expect(canvas).toHaveAttribute('data-render-generation', '1');
});

test('mobile signal renderer keeps a dense periodic viewport', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'mobile-chromium', 'mobile renderer density gate');

  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/pmoke/en/');
  const canvas = page.locator('.signal-stage canvas');
  await expect(page.locator('.signal-stage')).toHaveAttribute('data-wasm', 'ready', { timeout: 15_000 });
  const metrics = await canvas.evaluate((element: HTMLCanvasElement) => ({
    cssWidth: element.getBoundingClientRect().width,
    renderPoints: Number(element.dataset.renderPoints),
    generation: Number(element.dataset.renderGeneration),
  }));
  expect(metrics.renderPoints).toBeGreaterThanOrEqual(Math.ceil(metrics.cssWidth));
  expect(metrics.generation).toBe(1);
});

test('favicon is optimized and available', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser favicon asset gate');

  await page.goto('/pmoke/en/');
  const favicon = page.locator('link[rel="icon"]');
  await expect(favicon).toHaveAttribute('href', /\/pmoke\/favicon\.svg$/u);
  const response = await page.request.get('/pmoke/favicon.svg');
  expect(response.ok()).toBeTruthy();
  expect(response.headers()['content-type']).toContain('image/svg+xml');
  expect((await response.body()).byteLength).toBeLessThan(1_000);
});

test('all rendered internal links resolve beneath the project path', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser link gate');

  const routes = ['/pmoke/', '/pmoke/en/', '/pmoke/ja/', '/pmoke/en/docs/', '/pmoke/ja/docs/quickstart/'];
  const links = new Set<string>(routes);
  for (const route of routes) {
    await page.goto(route);
    const rendered = await page.locator('a[href]').evaluateAll((anchors) =>
      anchors.map((anchor) => (anchor as HTMLAnchorElement).pathname).filter((path) => path.startsWith('/pmoke/')),
    );
    for (const link of rendered) links.add(link);
  }

  for (const link of links) {
    const response = await page.request.get(link);
    expect(response.status(), link).toBeLessThan(400);
  }
});

test('SEO outputs contain canonical locale mappings and social image', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser metadata gate');

  await page.goto('/pmoke/ja/docs/quickstart/');
  await expect(page.locator('link[rel="canonical"]')).toHaveAttribute(
    'href',
    'https://kerr-group.github.io/pmoke/ja/docs/quickstart/',
  );
  for (const locale of ['en', 'ja', 'x-default']) {
    await expect(page.locator(`link[rel="alternate"][hreflang="${locale}"]`)).toHaveCount(1);
  }
  const imageUrl = await page.locator('meta[property="og:image"]').getAttribute('content');
  expect(imageUrl).toBeTruthy();
  const image = await page.request.get(new URL(imageUrl!).pathname);
  expect(image.ok()).toBeTruthy();
  expect(image.headers()['content-type']).toContain('image/png');

  const sitemap = await page.request.get('/pmoke/sitemap.xml');
  expect(sitemap.ok()).toBeTruthy();
  expect(await sitemap.text()).toContain('https://kerr-group.github.io/pmoke/ja/docs/quickstart/');
  const robots = await page.request.get('/pmoke/robots.txt');
  expect(robots.ok()).toBeTruthy();
  expect(await robots.text()).toContain('Sitemap: https://kerr-group.github.io/pmoke/sitemap.xml');
});
