import { source } from '@/lib/source';
import { MODEL_ID, VECTOR_DIMENSIONS, embedSearchText } from '@/lib/search/concept-model.mjs';
import { versionMetadata } from '@/lib/version';

export const revalidate = false;

type StructuredData = {
  headings: { id: string; content: string }[];
  contents: { heading: string | undefined; content: string }[];
};

export function GET() {
  const records = source.getPages().flatMap((page) => buildRecords(page, page.data.structuredData));
  return Response.json({
    schema: 1,
    model: MODEL_ID,
    dimensions: VECTOR_DIMENSIONS,
    source_commit: versionMetadata.source_commit,
    records,
  });
}

function buildRecords(
  page: ReturnType<(typeof source)['getPages']>[number],
  structuredData: StructuredData | undefined,
) {
  if (!structuredData) throw new Error(`missing semantic search data for ${page.url}`);
  const locale = page.locale ?? 'en';
  const headingNames = new Map(structuredData.headings.map((heading) => [heading.id, heading.content]));
  const byHeading = new Map<string | undefined, string[]>();
  for (const item of structuredData.contents) {
    const values = byHeading.get(item.heading) ?? [];
    values.push(item.content);
    byHeading.set(item.heading, values);
  }

  const allContent = structuredData.contents.map((item) => item.content).join(' ');
  const pageText = cleanText([page.data.title, page.data.description, allContent].filter(Boolean).join(' '));
  const records = [
    makeRecord({
      id: `${locale}:${page.url}:page`,
      url: page.url,
      locale,
      title: page.data.title,
      section: page.data.title,
      excerpt: cleanText(page.data.description ?? allContent).slice(0, 360),
      searchText: pageText.slice(0, 8_000),
    }),
  ];

  for (const [headingId, values] of byHeading) {
    if (!headingId) continue;
    const section = cleanText(headingNames.get(headingId) ?? headingId);
    const content = cleanText(values.join(' '));
    if (content.length === 0) continue;
    records.push(
      makeRecord({
        id: `${locale}:${page.url}:${headingId}`,
        url: `${page.url}#${headingId}`,
        locale,
        title: page.data.title,
        section,
        excerpt: content.slice(0, 360),
        searchText: cleanText(`${page.data.title} ${section} ${content}`).slice(0, 5_000),
      }),
    );
  }
  return records;
}

function makeRecord(record: {
  id: string;
  url: string;
  locale: string;
  title: string;
  section: string;
  excerpt: string;
  searchText: string;
}) {
  const weighted = `${record.title} ${record.title} ${record.section} ${record.section} ${record.searchText}`;
  return { ...record, embedding: embedSearchText(weighted) };
}

function cleanText(value: string): string {
  return value
    .replace(/<[^>]+>/gu, ' ')
    .replace(/[`*_#|>[\]{}()]/gu, ' ')
    .replace(/\s+/gu, ' ')
    .trim();
}
