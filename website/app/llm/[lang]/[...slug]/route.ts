import { notFound } from 'next/navigation';
import { getLLMText, source } from '@/lib/source';

type Params = { lang: string; slug: string[] };

export async function GET(_request: Request, { params }: { params: Promise<Params> }) {
  const { lang, slug } = await params;
  const page = source.getPage(slug, lang);
  if (!page) notFound();
  return new Response(await getLLMText(page), {
    headers: { 'Content-Type': 'text/markdown; charset=utf-8' },
  });
}

export function generateStaticParams() {
  return source.generateParams().filter((params) => params.slug.length > 0);
}
