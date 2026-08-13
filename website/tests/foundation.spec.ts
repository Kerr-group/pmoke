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
  await expect(page.locator('.signal-process-kicker')).toHaveText('測定ワークフロー');
  await expect(page.locator('.signal-process-rail')).toHaveAttribute('aria-label', 'パルス磁場下でのMOKE測定と解析の工程');
  await expect(page.locator('.signal-step-name').first()).toHaveText('パルス磁場');
  await expect(page.locator('.signal-stage-copy-panel[data-active="true"] .signal-stage-heading h2')).toHaveText('パルス磁場');
  await expect(page.locator('#signal-description')).toContainText('パルス磁場下でのMOKE測定を示す処理図');
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

test('signal process advances automatically and honors reduced motion', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser motion contract');

  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await page.goto('/pmoke/en/');
  const stage = page.locator('.signal-stage');
  await expect(stage).toHaveAttribute('data-wasm', 'ready', { timeout: 15_000 });
  await expect(stage).toHaveAttribute('data-motion', 'running');

  const firstStage = await stage.getAttribute('data-sequence-stage');
  await expect.poll(() => stage.getAttribute('data-sequence-stage'), { timeout: 7_000 }).not.toBe(firstStage);

  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.reload();
  await expect(stage).toHaveAttribute('data-wasm', 'ready', { timeout: 15_000 });
  await expect(stage).toHaveAttribute('data-motion', 'reduced');
  await expect(stage.locator('.signal-panel[data-active="true"]')).toHaveCount(1);
  const reducedStage = await stage.getAttribute('data-sequence-stage');
  await page.waitForTimeout(300);
  expect(await stage.getAttribute('data-sequence-stage')).toBe(reducedStage);
});

test('signal hero exposes localized process semantics and stage replay controls', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser signal semantics contract');

  await page.emulateMedia({ reducedMotion: 'no-preference' });
  for (const [locale, labels, description, processLabel] of [
    [
      'en',
      ['FIELD PULSE', 'NUMERICAL LI ANALYSIS', 'PHASE ALIGNMENT', 'KERR ANGLE'],
      'Pulsed-field MOKE measurement workflow.',
      'Pulsed-field MOKE measurement and analysis stages',
    ],
    [
      'ja',
      ['パルス磁場', '数値LI検波', '位相整合', 'Kerr角度'],
      'パルス磁場下でのMOKE測定を示す処理図。',
      'パルス磁場下でのMOKE測定と解析の工程',
    ],
  ] as const) {
    await page.goto('/pmoke/' + locale + '/');
    const stage = page.locator('.signal-stage');
    const card = stage.locator('.signal-process-card');
    await expect(stage).toHaveAttribute('data-wasm', 'ready', { timeout: 15_000 });
    await expect(stage).toHaveAttribute('data-motion', 'running');
    await expect(card).toHaveAttribute('aria-label', locale === 'en'
      ? 'Pulsed-field MOKE measurement workflow'
      : 'パルス磁場下でのMOKE測定ワークフロー');
    await expect(page.locator('#signal-description')).toContainText(description);
    await expect(stage.locator('.signal-process-kicker')).toHaveText(
      locale === 'en' ? 'MEASUREMENT WORKFLOW' : '測定ワークフロー',
    );
    const rail = stage.locator('.signal-process-rail');
    await expect(rail).toHaveAttribute('aria-label', processLabel);
    await expect(rail.locator('.signal-step-name')).toHaveText(labels);
    expect(await rail.locator('li').evaluateAll((items) => items.map((item) => item.dataset.step))).toEqual([
      'field-pulse',
      'lock-in',
      'phase-correction',
      'kerr-angle',
    ]);

    const readRailState = () => stage.evaluate((element) => {
      const active = [...element.querySelectorAll<HTMLElement>('.signal-process-rail button[aria-current="step"]')]
        .map((button) => button.closest('li')?.dataset.step);
      return {
        stage: element.dataset.sequenceStage,
        active,
        buttons: element.querySelectorAll('.signal-process-rail button').length,
        liveRegions: element.querySelectorAll('.signal-process-rail [aria-live]').length,
        pipelineProgress: Number.parseFloat(
          (element.querySelector<HTMLElement>('.signal-process-track span')?.style.width ?? '0'),
        ),
      };
    });
    const railState = await readRailState();
    expect(railState.active).toEqual([railState.stage]);
    expect(railState.buttons).toBe(4);
    expect(railState.liveRegions).toBe(0);
    await rail.getByRole('button', {
      name: (locale === 'en' ? 'Replay stage' : 'この工程を再生') + ': ' + (locale === 'en' ? 'NUMERICAL LI ANALYSIS' : '数値LI検波'),
    }).click();
    await expect(stage.locator('.signal-stage-copy-panel[data-active="true"] .signal-stage-heading h2')).toHaveText(
      locale === 'en' ? 'NUMERICAL LOCK-IN ANALYSIS' : '数値Lock-in検波',
    );
    const lockInRailState = await readRailState();
    expect(lockInRailState.stage).toBe('lock-in');
    expect(lockInRailState.pipelineProgress).toBeCloseTo(100 / 3, 3);

    await rail.getByRole('button', {
      name: (locale === 'en' ? 'Replay stage' : 'この工程を再生') + ': ' + (locale === 'en' ? 'PHASE ALIGNMENT' : '位相整合'),
    }).click();
    await expect(stage).toHaveAttribute('data-playback-mode', 'stage');
    await expect(stage).toHaveAttribute('data-sequence-stage', 'phase-correction');
    await expect(stage.locator('.signal-stage-copy-panel[data-active="true"] .signal-stage-heading h2')).toHaveText(
      locale === 'en' ? 'PHASE ALIGNMENT' : '位相整合',
    );
    expect((await readRailState()).pipelineProgress).toBeCloseTo((200 / 3), 3);
    await expect(stage.locator('.signal-playback-control')).toHaveCount(0);
    await expect(stage.locator('[aria-pressed], [data-user-paused]')).toHaveCount(0);
    await expect(page.getByText(locale === 'en' ? 'Pause' : '一時停止', { exact: true })).toHaveCount(0);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBeTruthy();
  }
});

test('signal stage transitions preserve completed outgoing visualizations', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser signal transition contract');

  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await page.goto('/pmoke/en/');
  const stage = page.locator('.signal-stage');
  const readPipelineProgress = () => stage.locator('.signal-process-track span').evaluate((element) =>
    Number.parseFloat((element as HTMLElement).style.width),
  );
  await expect(stage).toHaveAttribute('data-wasm', 'ready', { timeout: 15_000 });
  await expect(stage).toHaveAttribute('data-sequence-stage', 'field-pulse');
  await expect.poll(
    async () => stage.getAttribute('data-sequence-stage'),
    { timeout: 7_000 },
  ).toBe('lock-in');
  await expect(stage.locator('#signal-field-reveal rect')).toHaveAttribute('width', '100');
  expect(await stage.locator('.signal-panel').evaluateAll((panels) => panels
    .map((panel) => getComputedStyle(panel).transform))).toEqual(['none', 'none', 'none', 'none']);
  await expect(stage.locator('.signal-panel[data-active="true"]')).toHaveCSS('opacity', '1');
  expect(await stage.locator('.signal-panel').evaluateAll((panels) => panels
    .filter((panel) => getComputedStyle(panel).visibility === 'visible').length)).toBe(1);
  expect(await stage.locator('.signal-stage-copy-panel').evaluateAll((panels) => panels
    .filter((panel) => getComputedStyle(panel).visibility === 'visible').length)).toBe(1);
  expect(await readPipelineProgress()).toBeCloseTo(100 / 3, 3);

  await expect.poll(
    async () => stage.getAttribute('data-sequence-stage'),
    { timeout: 7_000 },
  ).toBe('phase-correction');
  await expect(stage.locator('#signal-lockin-reveal rect')).toHaveAttribute('width', '100');
  await expect(stage.locator('.signal-panel[data-active="true"]')).toHaveCSS('opacity', '1');
  expect(await stage.locator('.signal-panel').evaluateAll((panels) => panels
    .filter((panel) => getComputedStyle(panel).visibility === 'visible').length)).toBe(1);
  expect(await readPipelineProgress()).toBeCloseTo(200 / 3, 3);

  await expect.poll(
    async () => stage.getAttribute('data-sequence-stage'),
    { timeout: 7_000 },
  ).toBe('kerr-angle');
  await expect(stage.locator('.signal-phase-corrected')).toHaveAttribute('x2', '87');
  await expect(stage.locator('.signal-phase-corrected')).toHaveAttribute('y2', '50');
  await expect(stage.locator('.signal-panel[data-active="true"]')).toHaveCSS('opacity', '1');
  expect(await stage.locator('.signal-panel').evaluateAll((panels) => panels
    .filter((panel) => getComputedStyle(panel).visibility === 'visible').length)).toBe(1);
  expect(await readPipelineProgress()).toBeCloseTo(100, 3);
});

test('signal hero keeps an informative fallback when Wasm is unavailable', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser signal fallback contract');

  await page.route('**/wasm/pmoke_web_wasm.js', (route) => route.abort());
  await page.goto('/pmoke/ja/');
  const stage = page.locator('.signal-stage');
  await expect(stage).toHaveAttribute('data-wasm', 'fallback', { timeout: 15_000 });
  await expect(stage.locator('.signal-process-card')).toBeVisible();
  await expect(stage.locator('.signal-process-rail li')).toHaveCount(4);
  await expect(stage.locator('.signal-panel[data-active="true"]')).toHaveCount(1);
  await expect(stage.locator('.signal-process-rail .signal-step-name')).toHaveText([
    'パルス磁場',
    '数値LI検波',
    '位相整合',
    'Kerr角度',
  ]);
});

test('signal process rail stays responsive across compact viewports', async ({ page }, testInfo) => {
  test.skip(!['desktop-chromium', 'tablet-chromium', 'mobile-chromium'].includes(testInfo.project.name), 'responsive process rail gate');

  if (testInfo.project.name === 'desktop-chromium') {
    await page.setViewportSize({ width: 959, height: 900 });
  }

  for (const locale of ['en', 'ja'] as const) {
    await page.goto('/pmoke/' + locale + '/');
    const stage = page.locator('.signal-stage');
    const card = stage.locator('.signal-process-card');
    await expect(stage).toHaveAttribute('data-wasm', 'ready', { timeout: 15_000 });
    await expect(card).toBeVisible();
    await expect(stage.locator('.signal-process-rail li')).toHaveCount(4);
    await expect(stage.locator('.signal-process-rail button')).toHaveCount(4);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBeTruthy();
    if (testInfo.project.name === 'mobile-chromium') {
      const stepName = stage.locator('.signal-step-name').first();
      await expect.poll(() => stepName.evaluate((element) => {
        const style = getComputedStyle(element);
        const rect = element.getBoundingClientRect();
        return { position: style.position, width: rect.width, height: rect.height };
      })).toMatchObject({ position: 'absolute', width: 1, height: 1 });
    }
  }
});

test('signal hero uses non-overlapping responsive card regions', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser responsive geometry gate');

  await page.emulateMedia({ reducedMotion: 'reduce' });
  const viewports = [
    [1440, 900],
    [1000, 852],
    [768, 1024],
    [720, 800],
    [428, 926],
    [360, 800],
  ] as const;

  for (const [width, height] of viewports) {
    await page.setViewportSize({ width, height });
    for (const locale of ['en', 'ja'] as const) {
      await page.goto('/pmoke/' + locale + '/');
      const stage = page.locator('.signal-stage');
      const card = stage.locator('.signal-process-card');
      await expect(stage).toHaveAttribute('data-wasm', 'ready', { timeout: 15_000 });
      const boxes = await page.evaluate(() => {
        const getBox = (selector: string) => {
          const element = document.querySelector(selector);
          const box = element?.getBoundingClientRect();
          return box ? {
            left: box.left,
            top: box.top,
            right: box.right,
            bottom: box.bottom,
            width: box.width,
            height: box.height,
          } : null;
        };
        const h1 = document.querySelector('.hero-copy h1');
        const h1Range = document.createRange();
        if (h1) h1Range.selectNodeContents(h1);
        const h1TextBox = h1 ? h1Range.getBoundingClientRect() : null;
        return {
          stage: getBox('.signal-stage'),
          copy: getBox('.hero-copy-panel'),
          h1Text: h1TextBox ? {
            left: h1TextBox.left,
            top: h1TextBox.top,
            right: h1TextBox.right,
            bottom: h1TextBox.bottom,
            width: h1TextBox.width,
            height: h1TextBox.height,
          } : null,
          card: getBox('.signal-process-card'),
          rail: getBox('.signal-process-rail'),
          visualization: getBox('.signal-visualization'),
          scrollWidth: document.documentElement.scrollWidth,
          innerWidth: window.innerWidth,
        };
      });
      expect(boxes.stage).toBeTruthy();
      expect(boxes.copy).toBeTruthy();
      expect(boxes.h1Text).toBeTruthy();
      expect(boxes.card).toBeTruthy();
      expect(boxes.rail).toBeTruthy();
      expect(boxes.visualization).toBeTruthy();
      expect(boxes.scrollWidth).toBeLessThanOrEqual(boxes.innerWidth);
      expect(boxes.card!.left).toBeGreaterThanOrEqual(boxes.stage!.left - 0.5);
      expect(boxes.card!.right).toBeLessThanOrEqual(boxes.stage!.right + 0.5);
      expect(boxes.visualization!.left).toBeGreaterThanOrEqual(boxes.card!.left - 0.5);
      expect(boxes.visualization!.right).toBeLessThanOrEqual(boxes.card!.right + 0.5);
      expect(boxes.visualization!.height).toBeGreaterThan(180);
      await expect(page.locator('.hero-copy-panel')).toHaveCSS('border-top-width', '0px');
      if (width >= 960) {
        expect(boxes.h1Text!.right).toBeLessThanOrEqual(boxes.visualization!.left - 0.5);
      }
      if (width <= 720) {
        expect(boxes.card!.width).toBeLessThanOrEqual(boxes.stage!.width + 0.5);
        expect(boxes.rail!.width).toBeLessThanOrEqual(boxes.card!.width + 0.5);
      }
    }
  }
});

test('signal chart axes align ticks with grid lines and the 0 ms trigger', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser chart geometry gate');

  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const [width, height] of [
    [1440, 900],
    [1000, 852],
    [768, 1024],
    [720, 800],
    [390, 844],
    [320, 720],
  ] as const) {
    await page.setViewportSize({ width, height });
    await page.goto('/pmoke/ja/');
    const stage = page.locator('.signal-stage');
    await expect(stage).toHaveAttribute('data-wasm', 'ready', { timeout: 15_000 });
    await stage.locator('.signal-process-rail button').first().click();

    const panel = stage.locator('.signal-panel[data-active="true"]');
    await expect(panel.locator('.signal-y-axis > div span')).toHaveText(['100', '50', '0']);
    await expect(panel.locator('.signal-panel-readout strong')).toContainText('100 T');
    const yOffsets = await panel.evaluate((element) => {
      const plot = element.querySelector('.signal-plot-area')?.getBoundingClientRect();
      if (!plot) return [];
      const ticks = [...element.querySelectorAll('.signal-y-axis > div span')];
      return ticks.map((tick, index) => {
        const box = tick.getBoundingClientRect();
        const gridY = plot.top + plot.height * [0.1, 0.5, 0.9][index];
        return box.top + box.height / 2 - gridY;
      });
    });
    expect(yOffsets).toHaveLength(3);
    for (const offset of yOffsets) expect(Math.abs(offset)).toBeLessThanOrEqual(0.75);

    for (const stageIndex of [0, 1, 3]) {
      await stage.locator('.signal-process-rail button').nth(stageIndex).click();
      const activePanel = stage.locator('.signal-panel[data-active="true"]');
      const triggerOffset = await activePanel.evaluate((element) => {
        const plot = element.querySelector('.signal-plot-area')?.getBoundingClientRect();
        const axis = element.querySelector('.signal-time-axis')?.getBoundingClientRect();
        const ticks = [...element.querySelectorAll('.signal-time-axis span')];
        const zeroTick = [...element.querySelectorAll('.signal-time-axis span')]
          .find((tick) => tick.textContent === '0')?.getBoundingClientRect();
        const firstTick = ticks.at(0)?.getBoundingClientRect();
        const lastTick = ticks.at(-1)?.getBoundingClientRect();
        if (!plot || !axis || !zeroTick || !firstTick || !lastTick) return null;
        const triggerX = plot.left + plot.width * (10 / 70);
        return {
          trigger: zeroTick.left + zeroTick.width / 2 - triggerX,
          firstOverflow: axis.left - firstTick.left,
          lastOverflow: lastTick.right - axis.right,
        };
      });
      expect(triggerOffset).not.toBeNull();
      expect(Math.abs(triggerOffset!.trigger)).toBeLessThanOrEqual(0.75);
      expect(triggerOffset!.firstOverflow).toBeLessThanOrEqual(0.75);
      expect(triggerOffset!.lastOverflow).toBeLessThanOrEqual(0.75);
    }
  }
});

test('signal hero survives delayed Wasm readiness and reload', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser renderer lifecycle gate');

  await page.route('**/wasm/pmoke_web_wasm.js', async (route) => {
    await new Promise((resolve) => setTimeout(resolve, 800));
    await route.continue();
  });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/pmoke/en/');
  const stage = page.locator('.signal-stage');
  await expect(stage).toHaveAttribute('data-wasm', 'loading');
  await expect(stage.locator('.signal-process-card')).toBeVisible();
  await expect(stage).toHaveAttribute('data-wasm', 'ready', { timeout: 15_000 });

  await page.reload();
  await expect(stage).toHaveAttribute('data-wasm', 'ready', { timeout: 15_000 });
  await expect(stage.locator('.signal-panel[data-active="true"]')).toHaveCount(1);
});

test('mobile signal workflow remains usable at phone width', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'mobile-chromium', 'mobile signal workflow gate');

  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/pmoke/en/');
  const stage = page.locator('.signal-stage');
  const card = stage.locator('.signal-process-card');
  await expect(stage).toHaveAttribute('data-wasm', 'ready', { timeout: 15_000 });
  await expect(card).toBeVisible();
  const initialHeight = await card.evaluate((element) => element.getBoundingClientRect().height);
  expect(initialHeight).toBeLessThanOrEqual(501);
  const initialVisualizationY = await stage.locator('.signal-visualization').evaluate(
    (element) => element.getBoundingClientRect().y,
  );
  await expect(stage.locator('.signal-visualization')).toHaveCSS('min-height', '280px');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBeTruthy();
  await expect(stage.locator('.signal-process-rail button')).toHaveCount(4);
  await stage.locator('.signal-process-rail button').nth(2).click();
  await expect(stage).toHaveAttribute('data-sequence-stage', 'phase-correction');
  await expect(stage.locator('.signal-panel[data-active="true"]')).toHaveCount(1);
  const phaseHeight = await card.evaluate((element) => element.getBoundingClientRect().height);
  const phaseVisualizationY = await stage.locator('.signal-visualization').evaluate(
    (element) => element.getBoundingClientRect().y,
  );
  const phasePipelineProgress = await stage.locator('.signal-process-track span').evaluate((element) =>
    Number.parseFloat((element as HTMLElement).style.width),
  );
  expect(phasePipelineProgress).toBeCloseTo(200 / 3, 3);
  await stage.locator('.signal-process-rail button').nth(3).click();
  await expect(stage).toHaveAttribute('data-sequence-stage', 'kerr-angle');
  const kerrHeight = await card.evaluate((element) => element.getBoundingClientRect().height);
  const kerrVisualizationY = await stage.locator('.signal-visualization').evaluate(
    (element) => element.getBoundingClientRect().y,
  );
  expect(Math.abs(phaseHeight - initialHeight)).toBeLessThanOrEqual(1);
  expect(Math.abs(kerrHeight - initialHeight)).toBeLessThanOrEqual(1);
  expect(Math.abs(phaseVisualizationY - initialVisualizationY)).toBeLessThanOrEqual(1);
  expect(Math.abs(kerrVisualizationY - initialVisualizationY)).toBeLessThanOrEqual(1);
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
