import { defineConfig, devices } from '@playwright/test';

const chromiumLaunchOptions = process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH
  ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH }
  : undefined;

export default defineConfig({
  testDir: './tests',
  outputDir: 'test-results',
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? [['html', { open: 'never' }], ['list']] : 'list',
  use: {
    baseURL: 'http://127.0.0.1:4173/pmoke',
    trace: 'on-first-retry',
  },
  projects: [
    {
      name: 'desktop-chromium',
      use: { ...devices['Desktop Chrome'], launchOptions: chromiumLaunchOptions, viewport: { width: 1440, height: 900 } },
    },
    {
      name: 'wide-chromium',
      use: { ...devices['Desktop Chrome'], launchOptions: chromiumLaunchOptions, viewport: { width: 1920, height: 1080 } },
    },
    {
      name: 'tablet-chromium',
      use: { ...devices['Desktop Chrome'], launchOptions: chromiumLaunchOptions, viewport: { width: 768, height: 1024 } },
    },
    {
      name: 'mobile-chromium',
      use: { ...devices['Pixel 7'], launchOptions: chromiumLaunchOptions, viewport: { width: 360, height: 800 } },
    },
    {
      name: 'desktop-firefox',
      testMatch: /release-browser\.spec\.ts/,
      use: { ...devices['Desktop Firefox'], viewport: { width: 1440, height: 900 } },
    },
    {
      name: 'desktop-chrome-stable',
      testMatch: /release-browser\.spec\.ts/,
      use: {
        ...devices['Desktop Chrome'],
        channel: chromiumLaunchOptions ? undefined : 'chrome',
        launchOptions: chromiumLaunchOptions,
        viewport: { width: 1440, height: 900 },
      },
    },
    {
      name: 'desktop-webkit',
      testMatch: /release-browser\.spec\.ts/,
      use: { ...devices['Desktop Safari'], viewport: { width: 1440, height: 900 } },
    },
  ],
  webServer: {
    command: 'node scripts/serve-basepath.mjs',
    url: 'http://127.0.0.1:4173/pmoke/en/',
    reuseExistingServer: !process.env.CI,
  },
});
