import { getSortedPages, source } from '@/lib/source';
import { createFromSource } from 'fumadocs-core/search/server';

export const revalidate = false;

const deterministicSource = new Proxy(source, {
  get(target, property, receiver) {
    if (property === 'getPages') return getSortedPages;
    return Reflect.get(target, property, receiver);
  },
});

export const { staticGET: GET } = createFromSource(deterministicSource, {
  buildIndex: async (page) => {
    const structuredData = page.data.structuredData;
    if (!structuredData) throw new Error(`missing search data for ${page.url}`);

    const isGeneratedReference = /\/(?:cli|configuration)\/reference$/u.test(page.url);
    return {
      id: page.url,
      title: page.data.title,
      description: page.data.description,
      url: page.url,
      structuredData: isGeneratedReference ? compactByHeading(structuredData) : structuredData,
    };
  },
});

type StructuredData = {
  headings: { id: string; content: string }[];
  contents: { heading: string | undefined; content: string }[];
};

function compactByHeading(data: StructuredData): StructuredData {
  const grouped = new Map<string | undefined, string[]>();
  for (const item of data.contents) {
    const values = grouped.get(item.heading) ?? [];
    values.push(item.content);
    grouped.set(item.heading, values);
  }
  return {
    headings: data.headings,
    contents: Array.from(grouped, ([heading, values]) => ({
      heading,
      content: values.join(' · ').slice(0, 4_000),
    })),
  };
}
