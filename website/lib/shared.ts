export const appName = 'pmoke';
export const docsRoute = '/docs';
export const docsContentRoute = '/llm';
export const siteDescription =
  'A reproducible Rust workflow for pulsed-field MOKE—from instrument trigger and waveform capture through lock-in X/Y extraction, per-harmonic phase rotation, and Kerr-angle extraction.';
export const siteDescriptionJa =
  '装置トリガーと波形の取得から、ロックインX/Y抽出、高調波ごとの位相回転、Kerr角の算出までを一貫して扱う、再現可能なRustワークフロー。';
const configuredBasePath = process.env.NEXT_PUBLIC_BASE_PATH ?? '/pmoke';
export const basePath = configuredBasePath === '/' ? '' : configuredBasePath.replace(/\/$/, '');
export const siteOrigin = (process.env.NEXT_PUBLIC_SITE_ORIGIN ?? 'https://kerr-group.github.io').replace(
  /\/$/,
  '',
);
export const siteUrl = `${siteOrigin}${basePath}`;

export function absoluteUrl(pathname = '/'): string {
  const normalized = pathname.replace(/^\/+|\/+$/g, '');
  const path = pathname === '/' ? '/' : `/${normalized}${/\.[^/]+$/.test(normalized) ? '' : '/'}`;
  return `${siteUrl}${path}`;
}

export const socialImage = {
  url: absoluteUrl('/og.png'),
  width: 1200,
  height: 630,
  alt: 'pmoke pulsed-field MOKE reproducible measurement',
};

export const faviconImage = `${basePath}/favicon.svg`;

export const gitConfig = {
  user: 'Kerr-group',
  repo: 'pmoke',
  branch: 'main',
};
