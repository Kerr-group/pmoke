module.exports = {
  ci: {
    collect: {
      startServerCommand: 'node scripts/serve-basepath.mjs',
      startServerReadyPattern: 'Serving .*127\\.0\\.0\\.1:4173',
      startServerReadyTimeout: 15_000,
      url: [
        'http://127.0.0.1:4173/pmoke/en/',
        'http://127.0.0.1:4173/pmoke/en/docs/quickstart/',
        'http://127.0.0.1:4173/pmoke/ja/docs/quickstart/',
        'http://127.0.0.1:4173/pmoke/en/docs/configuration/validation/',
        'http://127.0.0.1:4173/pmoke/en/docs/interactive/waveform-analyzer/',
      ],
      numberOfRuns: 3,
      chromeFlags: '--headless=new --no-sandbox --disable-dev-shm-usage',
      settings: {
        preset: 'desktop',
        formFactor: 'desktop',
        screenEmulation: {
          mobile: false,
          width: 1440,
          height: 900,
          deviceScaleFactor: 1,
          disabled: false,
        },
        throttlingMethod: 'simulate',
      },
    },
  },
};
