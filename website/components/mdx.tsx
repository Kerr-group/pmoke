import defaultMdxComponents from 'fumadocs-ui/mdx';
import type { MDXComponents } from 'mdx/types';
import type { HTMLAttributes } from 'react';
import type { Language } from '@/lib/i18n';
import { LocalizedCodeBlock } from './localized-code-block';
import { ReferenceMetadata } from './reference-metadata';
import { ConfigValidator } from './config-validator';
import { WaveformAnalyzer } from './waveform-analyzer';
import { AIResourceHub } from './ai-resource-hub';
import { CitationPanel } from './citation-panel';

export function getMDXComponents(components?: MDXComponents) {
  return createMDXComponents('en', components);
}

export function getLocalizedMDXComponents(locale: Language, components?: MDXComponents) {
  return createMDXComponents(locale, components);
}

function createMDXComponents(locale: Language, components?: MDXComponents) {
  return {
    ...defaultMdxComponents,
    pre: createLocalizedPre(locale),
    ReferenceMetadata,
    ConfigValidator,
    WaveformAnalyzer,
    AIResourceHub,
    CitationPanel,
    ...components,
  } satisfies MDXComponents;
}

function createLocalizedPre(locale: Language) {
  return function LocalizedPre(props: HTMLAttributes<HTMLPreElement>) {
    const title = typeof props.title === 'string' && props.title.trim() ? props.title.trim() : undefined;
    const accessibleName = title ?? (locale === 'ja' ? 'コード例' : 'Code example');

    return <LocalizedCodeBlock {...props} accessibleName={accessibleName} />;
  };
}

export const useMDXComponents = getMDXComponents;

declare global {
  type MDXProvidedComponents = ReturnType<typeof getMDXComponents>;
}
