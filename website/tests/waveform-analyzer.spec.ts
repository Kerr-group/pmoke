import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';

test('Wasm phase adapter preserves typed input errors', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser Wasm boundary gate');
  await page.goto('/pmoke/en/');
  const messages = await page.evaluate(async () => {
    const wasm = (await import('/pmoke/wasm/pmoke_web_wasm.js')) as unknown as {
      default: () => Promise<unknown>;
      rotate_phase_interleaved: (x: Float64Array, y: Float64Array, delta: number) => Float64Array;
    };
    await wasm.default();
    const messageOf = (error: unknown) => (error instanceof Error ? error.message : String(error));
    const capture = (operation: () => unknown) => {
      try {
        operation();
        return 'no error';
      } catch (error) {
        return messageOf(error);
      }
    };
    return {
      length: capture(() => wasm.rotate_phase_interleaved(new Float64Array([1, 2]), new Float64Array([3]), 0.2)),
      finite: capture(() => wasm.rotate_phase_interleaved(new Float64Array([Number.NaN]), new Float64Array([1]), 0.2)),
      valid: Array.from(wasm.rotate_phase_interleaved(new Float64Array([1]), new Float64Array([0]), 0)),
    };
  });
  expect(messages.length).toMatch(/^length_mismatch:/u);
  expect(messages.finite).toMatch(/^non_finite_phase:/u);
  expect(messages.valid).toEqual([1, 0]);
  testInfo.annotations.push({ type: 'boundary', description: 'Wasm adapter returns stable phase error prefixes' });
});

for (const locale of ['en', 'ja'] as const) {
  test(`${locale} waveform analyzer completes native-parity demo`, async ({ page }, testInfo) => {
    await page.goto(`/pmoke/${locale}/docs/interactive/waveform-analyzer/`);
    const analyzer = page.locator('.waveform-analyzer');
    await expect(analyzer).toHaveAttribute('data-state', 'complete', { timeout: 20_000 });
    await expect(analyzer.locator('canvas')).toHaveCount(4);
    await expect(analyzer.getByText(locale === 'ja' ? 'ネイティブ版と数値一致' : 'Native-equivalent')).toBeVisible();
    await expect(analyzer.getByText(/pmoke-web-wasm\/0\.1\.0/u)).toBeVisible();
    const nonBlank = await analyzer.locator('canvas').evaluateAll((canvases) =>
      canvases.map((canvas) => {
        const target = canvas as HTMLCanvasElement;
        const context = target.getContext('2d');
        if (!context) return 0;
        const pixels = context.getImageData(0, 0, target.width, target.height).data;
        let count = 0;
        for (let index = 3; index < pixels.length; index += 128) if (pixels[index] > 0) count += 1;
        return count;
      }),
    );
    expect(nonBlank.every((count) => count > 40)).toBeTruthy();
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
    expect(payload.bytes).toBeLessThan(2_500_000);
    expect(payload.scriptBytes).toBeLessThan(1_500_000);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBeTruthy();
    await page.screenshot({ path: testInfo.outputPath(`${locale}-waveform-analyzer.png`), fullPage: true });
  });
}

test('maximum demo remains responsive and reports bounded output', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser runtime gate');
  await page.goto('/pmoke/en/docs/interactive/waveform-analyzer/');
  const analyzer = page.locator('.waveform-analyzer');
  await expect(analyzer).toHaveAttribute('data-state', 'complete', { timeout: 20_000 });
  await analyzer.getByLabel('Samples').fill('100000');
  await analyzer.getByLabel('Noise RMS').fill('0');
  await analyzer.getByRole('button', { name: 'Run analysis' }).click();
  await expect(analyzer).toHaveAttribute('data-state', 'complete', { timeout: 20_000 });
  const runtime = Number((await analyzer.getByText('Runtime', { exact: true }).locator('..').locator('dd').textContent())?.replace(' ms', ''));
  expect(runtime).toBeLessThan(5_000);
  await expect(analyzer.getByText('100,000', { exact: true })).toBeVisible();
  const kerr = Number((await analyzer.getByText('Median Kerr angle', { exact: true }).locator('..').locator('dd').textContent())?.replace(' rad', ''));
  expect(Math.abs(kerr - 0.01)).toBeLessThanOrEqual(1.0e-10 + 1.0e-8 * 0.01);

  const downloadPromise = page.waitForEvent('download');
  await analyzer.getByRole('button', { name: 'Export CSV' }).click();
  const download = await downloadPromise;
  const stream = await download.createReadStream();
  const chunks: Buffer[] = [];
  for await (const chunk of stream) chunks.push(Buffer.from(chunk));
  const csv = Buffer.concat(chunks).toString('utf8');
  expect(csv).toContain('# sample_rate_hz=100000');
  expect(csv).toContain('# first_input_index=');
  expect(csv).toContain('# warnings=');
  expect(csv).toContain('time_s,x_v,y_v,in_phase_v,out_of_phase_v,magnitude_v,phase_rad,kerr_rad');
});

test('analysis cancellation preserves controls and recovers the worker', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser cancellation gate');
  await page.goto('/pmoke/en/docs/interactive/waveform-analyzer/');
  const analyzer = page.locator('.waveform-analyzer');
  await expect(analyzer).toHaveAttribute('data-state', 'complete', { timeout: 20_000 });
  await analyzer.getByRole('button', { name: 'Local CSV' }).click();
  await analyzer.locator('input[type=file]').setInputFiles({
    name: 'million-samples.csv',
    mimeType: 'text/csv',
    buffer: Buffer.from('0\n'.repeat(1_000_000)),
  });
  await analyzer.getByLabel('Stride').fill('4');
  await analyzer.getByRole('button', { name: 'Run analysis' }).click();
  await analyzer.getByRole('button', { name: 'Cancel analysis' }).click();
  await expect(analyzer).toHaveAttribute('data-state', 'ready', { timeout: 20_000 });
  await expect(analyzer.getByText('million-samples.csv', { exact: true })).toBeVisible();
  await expect(analyzer.getByLabel('Stride')).toHaveValue('4');
  await analyzer.getByRole('button', { name: 'Synthetic' }).click();
  await analyzer.getByRole('button', { name: 'Run analysis' }).click();
  await expect(analyzer).toHaveAttribute('data-state', 'complete', { timeout: 20_000 });

  await analyzer.getByLabel('Samples').fill('100000');
  await analyzer.getByRole('button', { name: 'Run analysis' }).click();
  await analyzer.getByRole('button', { name: 'Reset' }).click();
  await expect(analyzer).toHaveAttribute('data-state', 'ready', { timeout: 20_000 });
  await expect(analyzer.getByLabel('Samples')).toHaveValue('20000');
  await page.waitForTimeout(100);
  await expect(analyzer).toHaveAttribute('data-state', 'ready');
});

test('waveform worker load failure is recoverable without losing input', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser failure-mode gate');
  await page.route('**/wasm/pmoke_web_wasm.js', (route) => route.abort());
  await page.goto('/pmoke/ja/docs/interactive/waveform-analyzer/');
  const analyzer = page.locator('.waveform-analyzer');
  await expect(analyzer).toHaveAttribute('data-state', 'error', { timeout: 20_000 });
  await analyzer.getByLabel('サンプル数').fill('24000');
  await page.unroute('**/wasm/pmoke_web_wasm.js');
  await analyzer.getByRole('button', { name: 'コアを再読み込み' }).click();
  await expect(analyzer).toHaveAttribute('data-state', 'ready', { timeout: 20_000 });
  await expect(analyzer.getByLabel('サンプル数')).toHaveValue('24000');
  await analyzer.getByRole('button', { name: '解析を実行' }).click();
  await expect(analyzer).toHaveAttribute('data-state', 'complete', { timeout: 20_000 });
});

test('local CSV stays in the worker and rejects a nonuniform axis', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser upload gate');
  await page.goto('/pmoke/en/docs/interactive/waveform-analyzer/');
  const analyzer = page.locator('.waveform-analyzer');
  await expect(analyzer).toHaveAttribute('data-state', 'complete', { timeout: 20_000 });
  await analyzer.getByRole('button', { name: 'Local CSV' }).click();
  const rows = ['time,signal'];
  for (let index = 0; index < 2000; index += 1) {
    const time = index === 1500 ? index * 1e-5 + 2e-6 : index * 1e-5;
    rows.push(`${time},${Math.sin(index * 0.02)}`);
  }
  await analyzer.locator('input[type=file]').setInputFiles({
    name: 'nonuniform.csv',
    mimeType: 'text/csv',
    buffer: Buffer.from(rows.join('\n')),
  });
  await analyzer.getByRole('button', { name: 'Run analysis' }).click();
  await expect(analyzer).toHaveAttribute('data-state', 'error', { timeout: 20_000 });
  await expect(analyzer.getByText(/non_uniform_time_axis/u)).toBeVisible();
  await expect(analyzer.getByText('nonuniform.csv', { exact: true })).toBeVisible();

  await analyzer.locator('input[type=file]').setInputFiles({
    name: 'empty-cell.csv',
    mimeType: 'text/csv',
    buffer: Buffer.from(`time,signal\n${Array.from({ length: 64 }, (_, index) => `${index * 1e-5},`).join('\n')}`),
  });
  await analyzer.getByRole('button', { name: 'Run analysis' }).click();
  await expect(analyzer).toHaveAttribute('data-state', 'error', { timeout: 20_000 });
  await expect(analyzer.getByText(/invalid_csv: empty value/u)).toBeVisible();

  await analyzer.locator('input[type=file]').setInputFiles({
    name: 'extra-column.csv',
    mimeType: 'text/csv',
    buffer: Buffer.from(`time,signal,extra\n${Array.from({ length: 64 }, (_, index) => `${index * 1e-5},0,1`).join('\n')}`),
  });
  await analyzer.getByRole('button', { name: 'Run analysis' }).click();
  await expect(analyzer).toHaveAttribute('data-state', 'error', { timeout: 20_000 });
  await expect(analyzer.getByText(/invalid_csv: expected one or two columns/u)).toBeVisible();

  await analyzer.locator('input[type=file]').setInputFiles({
    name: 'oversized.csv',
    mimeType: 'text/csv',
    buffer: Buffer.alloc(16 * 1024 * 1024 + 1, 48),
  });
  await analyzer.getByRole('button', { name: 'Run analysis' }).click();
  await expect(analyzer).toHaveAttribute('data-state', 'error', { timeout: 20_000 });
  await expect(analyzer.getByText(/input_too_large/u)).toBeVisible();

  const coefficients = [
    0.5818649368420833, 0.3157453060879723, 0.10453790247959542,
    0.025139158519404087, 0.00476278673520494, 0.0007455519980140543,
  ];
  const validRows = ['time,signal'];
  for (let index = 0; index < 5000; index += 1) {
    const time = index * 1e-5;
    const value = coefficients.reduce((total, coefficient, offset) => {
      const harmonic = offset + 1;
      const kerrTerm = harmonic % 2 === 0 ? Math.cos(0.02) : Math.sin(0.02);
      return total + 2 * coefficient * kerrTerm * Math.sin(harmonic * 2 * Math.PI * 1000 * time + 0.2);
    }, 0);
    validRows.push(`${time},${value}`);
  }
  await analyzer.locator('input[type=file]').setInputFiles({
    name: 'valid.csv',
    mimeType: 'text/csv',
    buffer: Buffer.from(validRows.join('\n')),
  });
  await analyzer.getByRole('button', { name: 'Run analysis' }).click();
  await expect(analyzer).toHaveAttribute('data-state', 'complete', { timeout: 20_000 });
  await expect(analyzer.getByText('valid.csv', { exact: true })).toBeVisible();
});

test('waveform analyzer has no serious accessibility violations', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-chromium', 'single-browser accessibility gate');
  await page.goto('/pmoke/en/docs/interactive/waveform-analyzer/');
  await expect(page.locator('.waveform-analyzer')).toHaveAttribute('data-state', 'complete', { timeout: 20_000 });
  const result = await new AxeBuilder({ page }).analyze();
  const blocking = result.violations.filter(
    (violation) => violation.impact === 'serious' || violation.impact === 'critical',
  );
  expect(blocking, blocking.map((violation) => violation.id).join(', ')).toEqual([]);
});
