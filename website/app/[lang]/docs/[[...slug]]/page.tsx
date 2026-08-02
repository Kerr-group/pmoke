import type { Metadata } from 'next';
import { notFound } from 'next/navigation';
import { createRelativeLink } from 'fumadocs-ui/mdx';
import {
  DocsBody,
  DocsDescription,
  DocsPage,
  DocsTitle,
  MarkdownCopyButton,
  ViewOptionsPopover,
} from 'fumadocs-ui/layouts/docs/page';
import { getLocalizedMDXComponents } from '@/components/mdx';
import { isLanguage } from '@/lib/i18n';
import { getPageMarkdownUrl, source } from '@/lib/source';
import { absoluteUrl, gitConfig, socialImage } from '@/lib/shared';

type Params = { lang: string; slug?: string[] };

export default async function Page({ params }: { params: Promise<Params> }) {
  const { lang, slug } = await params;
  if (!isLanguage(lang)) notFound();
  const page = source.getPage(slug, lang);
  if (!page) notFound();
  const MDX = page.data.body;
  const markdownUrl = getPageMarkdownUrl(page).url;

  return (
    <DocsPage toc={page.data.toc} full={page.data.full}>
      <DocsTitle>{page.data.title}</DocsTitle>
      <DocsDescription className="mb-0">{page.data.description}</DocsDescription>
      <div className="flex flex-row gap-2 items-center border-b pb-6">
        <MarkdownCopyButton markdownUrl={markdownUrl} />
        <ViewOptionsPopover
          markdownUrl={markdownUrl}
          githubUrl={`https://github.com/${gitConfig.user}/${gitConfig.repo}/blob/${gitConfig.branch}/website/content/docs/${page.path}`}
        />
      </div>
      <DocsBody>
        <MDX components={getLocalizedMDXComponents(lang, { a: createRelativeLink(source, page) })} />
      </DocsBody>
    </DocsPage>
  );
}

export function generateStaticParams() {
  return source.generateParams();
}

export async function generateMetadata({ params }: { params: Promise<Params> }): Promise<Metadata> {
  const { lang, slug } = await params;
  const page = source.getPage(slug, lang);
  if (!page) notFound();
  const englishPage = source.getPage(slug, 'en');
  const japanesePage = source.getPage(slug, 'ja');
  const canonical = absoluteUrl(page.url);
  const languages: Record<string, string> = {};
  if (englishPage) {
    languages.en = absoluteUrl(englishPage.url);
    languages['x-default'] = absoluteUrl(englishPage.url);
  }
  if (japanesePage) languages.ja = absoluteUrl(japanesePage.url);

  return {
    title: page.data.title,
    description: page.data.description,
    alternates: {
      canonical,
      languages,
    },
    openGraph: {
      title: page.data.title,
      description: page.data.description,
      url: canonical,
      locale: lang === 'ja' ? 'ja_JP' : 'en_US',
      alternateLocale: lang === 'ja' ? ['en_US'] : ['ja_JP'],
      images: [socialImage],
    },
    twitter: { title: page.data.title, description: page.data.description, images: [socialImage.url] },
  };
}
