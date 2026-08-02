import { createContentHighlighter, type SortedResult } from 'fumadocs-core/search';
import type { SearchClient } from 'fumadocs-core/search/client';
import { staticClient } from 'fumadocs-core/search/client/orama-static';
import { basePath } from '@/lib/shared';
import { MODEL_ID, VECTOR_DIMENSIONS } from './concept-model.mjs';

export type SearchMode = 'idle' | 'loading' | 'hybrid' | 'fallback';

type SemanticRecord = {
  id: string;
  url: string;
  locale: string;
  title: string;
  section: string;
  excerpt: string;
  searchText: string;
  embedding: number[];
};

type SemanticIndex = {
  schema: number;
  model: string;
  dimensions: number;
  source_commit: string;
  records: SemanticRecord[];
};

type SemanticHit = { score: number; record: SemanticRecord };

const indexCache = new Map<string, Promise<SemanticIndex>>();
const engineCache = new Map<
  string,
  Promise<ReturnType<(typeof import('./hybrid-engine.mjs'))['createHybridEngine']>>
>();

export function createHybridSearchClient(
  locale: string,
  onModeChange: (mode: SearchMode) => void,
): SearchClient {
  const lexical = staticClient({ locale, from: `${basePath}/api/search` });
  let generation = 0;

  return {
    deps: [locale],
    async search(query) {
      const current = ++generation;
      onModeChange('loading');
      const lexicalPromise = Promise.resolve(lexical.search(query)).catch(() => []);
      try {
        const [lexicalResults, semanticResults] = await Promise.all([
          lexicalPromise,
          searchSemantic(query, locale),
        ]);
        if (current !== generation) return [];
        onModeChange('hybrid');
        return fuseResults(query, locale, lexicalResults, semanticResults);
      } catch {
        const lexicalResults = await lexicalPromise;
        if (current !== generation) return [];
        onModeChange('fallback');
        return compactFallbackResults(query, locale, lexicalResults);
      }
    },
  };
}

async function searchSemantic(query: string, locale: string): Promise<SemanticHit[]> {
  return (await loadHybridEngine(locale)).search(query);
}

function loadHybridEngine(locale: string) {
  const cached = engineCache.get(locale);
  if (cached) return cached;
  const loading = Promise.all([loadSemanticIndex(), import('./hybrid-engine.mjs')])
    .then(([index, module]) => {
      const records = index.records.filter((record) => record.locale === locale);
      if (records.length === 0) throw new Error(`semantic index has no records for ${locale}`);
      return module.createHybridEngine(records);
    })
    .catch((error: unknown) => {
      engineCache.delete(locale);
      throw error;
    });
  engineCache.set(locale, loading);
  return loading;
}

async function loadSemanticIndex(): Promise<SemanticIndex> {
  const url = `${basePath}/api/semantic-search`;
  const cached = indexCache.get(url);
  if (cached) return cached;
  const loading = fetch(url, { cache: 'force-cache' })
    .then(async (response) => {
      if (!response.ok) throw new Error(`semantic index request failed with ${response.status}`);
      const index = (await response.json()) as SemanticIndex;
      if (
        index.schema !== 1 ||
        index.model !== MODEL_ID ||
        index.dimensions !== VECTOR_DIMENSIONS ||
        !Array.isArray(index.records)
      ) {
        throw new Error('semantic index metadata mismatch');
      }
      return index;
    })
    .catch((error: unknown) => {
      indexCache.delete(url);
      throw error;
    });
  indexCache.set(url, loading);
  return loading;
}

function fuseResults(
  query: string,
  locale: string,
  lexical: SortedResult[],
  semantic: SemanticHit[],
): SortedResult[] {
  const scores = new Map<string, { score: number; result: SortedResult }>();
  const semanticByPage = new Map(
    semantic.map((hit) => [hit.record.url.split('#')[0], hit.record]),
  );
  const lexicalPages = uniquePages(lexical);
  lexicalPages.forEach((result, rank) => {
    const page = result.url.split('#')[0];
    scores.set(page, {
      score: 0.42 / (32 + rank),
      result: compactResult(query, locale, result, semanticByPage.get(page)),
    });
  });

  const highlighter = createContentHighlighter(query);
  semantic.forEach(({ record }, rank) => {
    const page = record.url.split('#')[0];
    const existing = scores.get(page);
    const semanticScore = 0.58 / (32 + rank);
    if (existing) {
      existing.score += semanticScore;
      return;
    }
    scores.set(page, {
      score: semanticScore,
      result: {
        id: `semantic:${record.id}`,
        url: record.url,
        type: record.section === record.title ? 'page' : 'text',
        breadcrumbs: [record.locale.toUpperCase(), record.title, record.section],
        content: highlighter.highlightMarkdown(record.excerpt || record.section),
      },
    });
  });

  return Array.from(scores.values())
    .sort((left, right) => right.score - left.score || left.result.url.localeCompare(right.result.url))
    .slice(0, 12)
    .map((entry) => entry.result);
}

function uniquePages(results: SortedResult[]): SortedResult[] {
  const pages = new Set<string>();
  return results.filter((result) => {
    const page = result.url.split('#')[0];
    if (pages.has(page)) return false;
    pages.add(page);
    return true;
  });
}

function compactResult(
  query: string,
  locale: string,
  result: SortedResult,
  metadata?: SemanticRecord,
): SortedResult {
  if (typeof result.content !== 'string') return result;
  const plain = result.content
    .replace(/<\/?mark>/gu, '')
    .replace(/[|·]+/gu, ' ')
    .replace(/\s+/gu, ' ')
    .trim();
  const excerpt = plain.length > 280 ? `${plain.slice(0, 277).trimEnd()}...` : plain;
  return {
    ...result,
    breadcrumbs: metadata
      ? [locale.toUpperCase(), metadata.title, ...(metadata.section === metadata.title ? [] : [metadata.section])]
      : [locale.toUpperCase(), ...(result.breadcrumbs ?? [])],
    content: createContentHighlighter(query).highlightMarkdown(excerpt),
  };
}

function compactFallbackResults(
  query: string,
  locale: string,
  results: SortedResult[],
): SortedResult[] {
  return uniquePages(results).map((result) => compactResult(query, locale, result));
}
