import { notFound } from 'next/navigation';
import { getLLMText, source } from '@/lib/source';

type Params = { lang: string; slug: string[] };

export async function GET(_request: Request, { params }: { params: Promise<Params> }) {
  const { lang, slug } = await params;
  const last = slug.at(-1);
  if (!last?.endsWith('.md')) notFound();
  const pageSlug = [...slug.slice(0, -1), last.slice(0, -3)];
  const page = source.getPage(pageSlug, lang);
  if (!page) notFound();
  return new Response(await getLLMText(page), {
    headers: { 'Content-Type': 'text/markdown; charset=utf-8' },
  });
}

export function generateStaticParams() {
  return source.generateParams().flatMap((params) => {
    if (params.slug.length === 0) return [];
    const last = params.slug.at(-1);
    if (!last) return [];
    return [{ ...params, slug: [...params.slug.slice(0, -1), `${last}.md`] }];
  });
}
