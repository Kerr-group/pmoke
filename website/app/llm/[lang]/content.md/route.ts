import { notFound } from 'next/navigation';
import { getLLMText, source } from '@/lib/source';
import { languages } from '@/lib/i18n';

export async function GET(_request: Request, { params }: { params: Promise<{ lang: string }> }) {
  const { lang } = await params;
  const page = source.getPage(undefined, lang);
  if (!page) notFound();
  return new Response(await getLLMText(page), {
    headers: { 'Content-Type': 'text/markdown; charset=utf-8' },
  });
}

export function generateStaticParams() {
  return languages.map((lang) => ({ lang }));
}
