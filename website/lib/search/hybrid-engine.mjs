// @ts-check

import { create, insertMultiple, search } from '@orama/orama';
import { embedSearchText, VECTOR_DIMENSIONS } from './concept-model.mjs';

/**
 * @typedef {{ id: string, url: string, locale: string, title: string, section: string,
 * excerpt: string, searchText: string, embedding: number[] }} SemanticRecord
 */

/** @param {SemanticRecord[]} records */
export function createHybridEngine(records) {
  const database = create({
    schema: {
      id: 'string',
      url: 'string',
      locale: 'string',
      title: 'string',
      section: 'string',
      excerpt: 'string',
      searchText: 'string',
      embedding: `vector[${VECTOR_DIMENSIONS}]`,
    },
  });
  insertMultiple(database, records);

  return {
    /** @param {string} query @param {number} [limit] */
    async search(query, limit = 12) {
      const result = await search(database, {
        mode: 'hybrid',
        term: query,
        vector: { value: embedSearchText(query), property: 'embedding' },
        hybridWeights: { text: 0.25, vector: 0.75 },
        similarity: 0,
        includeVectors: false,
        limit: Math.max(limit * 8, 40),
      });
      const byPage = new Map();
      for (const hit of result.hits) {
        if (hit.score <= 0.015) continue;
        const record = /** @type {SemanticRecord} */ (hit.document);
        const page = record.url.split('#')[0];
        const existing = byPage.get(page);
        if (!existing || hit.score > existing.score) byPage.set(page, { score: hit.score, record });
      }
      return Array.from(byPage.values())
        .sort((left, right) => right.score - left.score || left.record.url.localeCompare(right.record.url))
        .slice(0, limit)
        .filter((hit) => hit.score > 0.015)
        .map((hit) => ({ score: hit.score, record: hit.record }));
    },
  };
}
