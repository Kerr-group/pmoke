import { defineI18n } from 'fumadocs-core/i18n';
import { defineI18nUI } from 'fumadocs-ui/i18n';
import type { Translations } from 'fumadocs-ui/i18n';

export const languages = ['en', 'ja'] as const;
export type Language = (typeof languages)[number];

export const i18n = defineI18n({
  languages: [...languages],
  defaultLanguage: 'en',
  parser: 'dir',
  hideLocale: 'never',
  fallbackLanguage: 'en',
});

const japaneseUI = {
  displayName: '日本語',
  'Ask AI(AI chat button)': 'AI 質問',
  'Back to Home(404 not found page)': 'ホーム',
  'Choose a language(language switcher)': '言語選択',
  'Choose a language(language switcher)(aria-label)': '言語選択',
  'Close Banner(banner)(aria-label)': 'バナー終了',
  'Close Search(search dialog)(aria-label)': '検索画面終了',
  'Close Sidebar(aria-label)': 'サイドバー終了',
  'Close Sidebar(sidebar)(aria-label)': 'サイドバー終了',
  'Collapse Sidebar(sidebar)(aria-label)': 'サイドバー縮小',
  'Copied Text(code block)(aria-label)': 'コピー済み',
  'Copy Anchor Link(heading anchor)(aria-label)': 'アンカーリンクコピー',
  'Copy Link(accordion)(aria-label)': 'リンクコピー',
  'Copy Markdown(page actions)': 'Markdown コピー',
  'Copy Text(code block)(aria-label)': 'テキストコピー',
  'Dark(theme switcher)(aria-label)': 'ダークテーマ',
  'Default(type table)': 'デフォルト',
  'Edit on GitHub(edit page)': 'GitHub 編集',
  'Hide Sidebar(sidebar)': 'サイドバー非表示',
  'Last updated on(page footer)': '最終更新',
  'Layout Tab(layout tab trigger)': 'レイアウトタブ',
  'Light(theme switcher)(aria-label)': 'ライトテーマ',
  'Next Page(pagination)': '次のページ',
  'No Headings(table of contents)': '見出しなし',
  'No results found(search dialog)': '検索結果なし',
  'On this page(table of contents)': '目次',
  'Open Search(search trigger)(aria-label)': '検索画面',
  'Open Sidebar(aria-label)': 'サイドバー表示',
  'Open Sidebar(sidebar)(aria-label)': 'サイドバー表示',
  'Open in ChatGPT(page actions)': 'ChatGPT 表示',
  'Open in Claude(page actions)': 'Claude 表示',
  'Open in Cursor(page actions)': 'Cursor 表示',
  'Open in GitHub(page actions)': 'GitHub 表示',
  'Open in Scira AI(page actions)': 'Scira AI 表示',
  'Open(page actions)': '表示',
  'Page Not Found(404 not found page)': 'ページ未検出',
  'Parameters(type table)': 'パラメーター',
  'Previous Page(pagination)': '前のページ',
  'Prop(type table)': 'プロパティ',
  'Read {url}, I want to ask questions about it.(page actions)':
    '参照対象: {url}。この内容に関する質問。',
  'Returns(type table)': '戻り値',
  'Search(search trigger)': '検索',
  'Search(search dialog)': 'ドキュメント検索',
  'Show Sidebar(sidebar)': 'サイドバー表示',
  'System(theme switcher)(aria-label)': 'システムテーマ',
  'Table of Contents(inline table of contents)': '目次',
  'The page you are looking for might have been removed, had its name changed, or is temporarily unavailable.(404 not found page)':
    'ページの移動、名称変更、または一時的な利用不可。',
  'Toggle Menu(home layout header)(aria-label)': 'メニュー切替',
  'Toggle Theme(theme switcher)(aria-label)': 'テーマ選択',
  'Type(type table)': '型',
  'View as Markdown(page actions)': 'Markdown 表示',
} satisfies Translations;

export const i18nUI = defineI18nUI(i18n, {
  en: { displayName: 'English' },
  ja: japaneseUI,
});

export function isLanguage(value: string): value is Language {
  return languages.some((language) => language === value);
}
