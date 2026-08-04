export const appName = 'pmoke';
export const docsRoute = '/docs';
export const docsContentRoute = '/llm';
export const siteDescription =
  'Precision pulsed-MOKE acquisition, lock-in analysis, and instrument control.';
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
  alt: 'pmoke pulsed-MOKE precision signal lab',
};

export const faviconImage = `${basePath}/pmoke_faviicon.png`;

export const gitConfig = {
  user: 'Kerr-group',
  repo: 'pmoke',
  branch: 'main',
};
