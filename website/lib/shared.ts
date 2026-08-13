export const appName = 'pmoke';
export const docsRoute = '/docs';
export const docsContentRoute = '/llm';
export const siteDescription =
  'A reproducible Rust workflow for pulsed-field MOKE measurements—from waveform acquisition to lock-in X/Y extraction, per-harmonic phase alignment, and Kerr-angle extraction.';
export const siteDescriptionJa =
  'パルス磁場下でのMOKE測定に必要な波形取得、ロックインX/Yの抽出、高調波ごとの位相整合、Kerr角度の算出を扱う再現可能なRustワークフロー。';
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
  alt: 'pmoke Rust workflow for pulsed-field MOKE measurements',
};

export const faviconImage = `${basePath}/favicon.svg`;

export const gitConfig = {
  user: 'Kerr-group',
  repo: 'pmoke',
  branch: 'main',
};
