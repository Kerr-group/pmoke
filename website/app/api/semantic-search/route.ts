import { getSortedPages, source } from '@/lib/source';
import { MODEL_ID, VECTOR_DIMENSIONS, embedSearchText } from '@/lib/search/concept-model.mjs';
import { versionMetadata } from '@/lib/version';

export const revalidate = false;

type StructuredData = {
  headings: { id: string; content: string }[];
  contents: { heading: string | undefined; content: string }[];
};

// Keep high-intent user vocabulary close to the page that owns the concept.
// These compact, locale-specific hints improve retrieval without displaying
// artificial keywords in the rendered documentation or AI exports.
const SEARCH_HINTS: Record<string, string> = {
  '/en/docs': 'what is pmoke overview design priorities reproducibility',
  '/en/docs/configuration': 'schema configuration create config init guided configuration signal roles artifacts',
  '/en/docs/ai': 'machine readable documentation feed llms agent index versioned markdown feeds typed contracts local retrieval',
  '/ja/docs/ai': '機械可読 documentation feed llms エージェント Markdown feed 型付き契約 ローカル検索',
  '/ja/docs/configuration': 'schema 設定の作成 設定作成 signal role artifact 現行 template 起点',
  '/ja/docs': 'pmoke とは 概要 設計方針 再現可能 パルス磁場 MOKE 取得 ロックイン Kerr 角',
  '/en/docs/quickstart': 'run the reference sensor lock-in phase Kerr chain complete analysis workflow',
  '/en/docs/configuration/validation': 'migrate a legacy configuration invalid TOML diagnostics configuration error field path',
  '/en/docs/installation/feature-flags': 'analysis-only build features Cargo feature flags',
  '/en/docs/installation': 'set up the Python analysis environment install from source source build cargo install requirements pip',
  '/en/docs/installation/transports': 'TCP instrument timeout settings network instrument connection',
  '/en/docs/interactive/waveform-analyzer': 'calculate Kerr angle in the browser browser lock-in simulator',
  '/ja/docs/quickstart': '参照 センサー ロックイン 位相 Kerr 一括 解析 ワークフロー クイックスタート 最初の解析 取得済み波形 analyze',
  '/ja/docs/configuration/validation': '不正な TOML の診断 legacy 設定の移行 設定エラーのフィールドパス',
  '/ja/docs/installation/feature-flags': '解析専用 build feature Cargo 機能フラグ',
  '/ja/docs/installation': 'source からの build Python 解析環境の準備 導入手順 前提環境 cargo install requirements 導入確認',
  '/ja/docs/installation/transports': 'TCP 測定装置 timeout 設定 ネットワーク 接続',
  '/ja/docs/interactive/waveform-analyzer': 'ブラウザ Kerr 角度 Kerr 角 計算 lock-in simulator',
};

const SEARCH_SECTION_HINTS: Record<string, string> = {
  '/en/docs/configuration/validation#migrate-legacy-schema': SEARCH_HINTS['/en/docs/configuration/validation'],
};

export function GET() {
  const records = getSortedPages().flatMap((page) => buildRecords(page, page.data.structuredData));
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
  const hints = SEARCH_HINTS[page.url] ?? '';
  const pageText = cleanText([page.data.title, page.data.description, hints, allContent].filter(Boolean).join(' '));
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
    const sectionHints = SEARCH_SECTION_HINTS[`${page.url}#${headingId}`] ?? '';
    records.push(
      makeRecord({
        id: `${locale}:${page.url}:${headingId}`,
        url: `${page.url}#${headingId}`,
        locale,
        title: page.data.title,
        section,
        excerpt: content.slice(0, 360),
        searchText: cleanText(`${page.data.title} ${section} ${sectionHints} ${content}`).slice(0, 5_000),
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
