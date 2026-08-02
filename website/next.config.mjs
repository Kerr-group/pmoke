import { createMDX } from 'fumadocs-mdx/next';
import { execFileSync } from 'node:child_process';

const withMDX = createMDX();
const basePath = process.env.NEXT_PUBLIC_BASE_PATH ?? '/pmoke';
const sourceCommit =
  process.env.GITHUB_SHA ??
  execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();

/** @type {import('next').NextConfig} */
const config = {
  output: 'export',
  reactStrictMode: true,
  generateBuildId: async () => sourceCommit,
  basePath,
  trailingSlash: true,
  images: { unoptimized: true },
  env: {
    NEXT_PUBLIC_BASE_PATH: basePath,
    NEXT_PUBLIC_SOURCE_COMMIT: sourceCommit,
  },
};

export default withMDX(config);
