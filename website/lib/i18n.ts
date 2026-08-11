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
  'Ask AI(AI chat button)': 'AIに質問',
  'Back to Home(404 not found page)': 'ホームに戻る',
  'Choose a language(language switcher)': '言語を選択',
  'Choose a language(language switcher)(aria-label)': '言語を選択',
  'Close Banner(banner)(aria-label)': 'バナーを閉じる',
  'Close Search(search dialog)(aria-label)': '検索を閉じる',
  'Close Sidebar(aria-label)': 'サイドバーを閉じる',
  'Close Sidebar(sidebar)(aria-label)': 'サイドバーを閉じる',
  'Collapse Sidebar(sidebar)(aria-label)': 'サイドバーを折りたたむ',
  'Copied Text(code block)(aria-label)': 'コピー済み',
  'Copy Anchor Link(heading anchor)(aria-label)': '見出しリンクをコピー',
  'Copy Link(accordion)(aria-label)': 'リンクをコピー',
  'Copy Markdown(page actions)': 'Markdownをコピー',
  'Copy Text(code block)(aria-label)': 'テキストをコピー',
  'Dark(theme switcher)(aria-label)': 'ダークテーマ',
  'Default(type table)': 'デフォルト',
  'Edit on GitHub(edit page)': 'GitHubで編集',
  'Hide Sidebar(sidebar)': 'サイドバーを非表示',
  'Last updated on(page footer)': '最終更新日',
  'Layout Tab(layout tab trigger)': 'レイアウトタブ',
  'Light(theme switcher)(aria-label)': 'ライトテーマ',
  'Next Page(pagination)': '次のページ',
  'No Headings(table of contents)': '見出しなし',
  'No results found(search dialog)': '検索結果なし',
  'On this page(table of contents)': '目次',
  'Open Search(search trigger)(aria-label)': '検索を開く',
  'Open Sidebar(aria-label)': 'サイドバーを開く',
  'Open Sidebar(sidebar)(aria-label)': 'サイドバーを開く',
  'Open in ChatGPT(page actions)': 'ChatGPTで開く',
  'Open in Claude(page actions)': 'Claudeで開く',
  'Open in Cursor(page actions)': 'Cursorで開く',
  'Open in GitHub(page actions)': 'GitHubで開く',
  'Open in Scira AI(page actions)': 'Scira AIで開く',
  'Open(page actions)': '開く',
  'Page Not Found(404 not found page)': 'ページが見つかりません',
  'Parameters(type table)': 'パラメーター',
  'Previous Page(pagination)': '前のページ',
  'Prop(type table)': 'プロパティ',
  'Read {url}, I want to ask questions about it.(page actions)':
    '「{url}」について質問する',
  'Returns(type table)': '戻り値',
  'Search(search trigger)': '検索',
  'Search(search dialog)': 'ドキュメント検索',
  'Show Sidebar(sidebar)': 'サイドバーを表示',
  'System(theme switcher)(aria-label)': 'システムテーマ',
  'Table of Contents(inline table of contents)': '目次',
  'The page you are looking for might have been removed, had its name changed, or is temporarily unavailable.(404 not found page)':
    'お探しのページは削除・名称変更・一時的な利用不可の可能性。',
  'Toggle Menu(home layout header)(aria-label)': 'メニューを切り替える',
  'Toggle Theme(theme switcher)(aria-label)': 'テーマ選択',
  'Type(type table)': '型',
  'View as Markdown(page actions)': 'Markdownで表示',
} satisfies Translations;

export const i18nUI = defineI18nUI(i18n, {
  en: { displayName: 'English' },
  ja: japaneseUI,
});

export function isLanguage(value: string): value is Language {
  return languages.some((language) => language === value);
}
