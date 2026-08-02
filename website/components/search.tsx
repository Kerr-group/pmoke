'use client';
import {
  SearchDialog,
  SearchDialogClose,
  SearchDialogContent,
  SearchDialogHeader,
  SearchDialogIcon,
  SearchDialogInput,
  SearchDialogList,
  SearchDialogListItem,
  SearchDialogOverlay,
  SearchDialogFooter,
  type SharedProps,
} from 'fumadocs-ui/components/dialog/search';
import { useDocsSearch } from 'fumadocs-core/search/client';
import { useI18n } from 'fumadocs-ui/contexts/i18n';
import { Database, ShieldCheck, Sparkles } from 'lucide-react';
import { useMemo, useState } from 'react';
import { createHybridSearchClient, type SearchMode } from '@/lib/search/hybrid-client';

export default function DefaultSearchDialog(props: SharedProps) {
  const { locale: activeLocale } = useI18n();
  const locale = activeLocale ?? 'en';
  const [mode, setMode] = useState<SearchMode>('idle');
  const client = useMemo(() => createHybridSearchClient(locale, setMode), [locale]);
  const { search, setSearch, query } = useDocsSearch({
    client,
    delayMs: 120,
  });

  return (
    <SearchDialog search={search} onSearchChange={setSearch} isLoading={query.isLoading} {...props}>
      <SearchDialogOverlay />
      <SearchDialogContent>
        <SearchDialogHeader>
          <SearchDialogIcon />
          <SearchDialogInput />
          <SearchDialogClose />
        </SearchDialogHeader>
        <SearchDialogList
          items={query.data !== 'empty' ? query.data : null}
          role="listbox"
          aria-label={locale === 'ja' ? '検索結果' : 'Search results'}
          Item={({ item, onClick }) => (
            <SearchDialogListItem item={item} onClick={onClick} role="option" />
          )}
        />
        <SearchDialogFooter>
          <SearchStatus locale={locale} mode={mode} />
        </SearchDialogFooter>
      </SearchDialogContent>
    </SearchDialog>
  );
}

function SearchStatus({ locale, mode }: { locale: string; mode: SearchMode }) {
  const japanese = locale === 'ja';
  const content = {
    idle: {
      Icon: Sparkles,
      label: japanese ? 'ローカル・ハイブリッド' : 'Local hybrid',
      detail: japanese ? '待機中・外部送信なし' : 'Ready with no query upload',
    },
    loading: {
      Icon: Database,
      label: japanese ? 'Concept index 準備中' : 'Preparing concept index',
      detail: japanese ? '同一 origin の静的 asset' : 'Same-origin static asset',
    },
    hybrid: {
      Icon: Sparkles,
      label: japanese ? 'ローカル・ハイブリッド' : 'Local hybrid',
      detail: japanese ? '外部送信・telemetry なし' : 'No query upload or telemetry',
    },
    fallback: {
      Icon: ShieldCheck,
      label: japanese ? '全文検索フォールバック' : 'Full-text fallback',
      detail: japanese ? '同一 origin の ZBSearch' : 'Same-origin ZBSearch',
    },
  }[mode];
  return (
    <div className="search-status" data-search-mode={mode} role="status" aria-live="polite">
      <span className="search-status-mark" aria-hidden="true">
        <content.Icon />
      </span>
      <span>
        <strong>{content.label}</strong>
        <small>{content.detail}</small>
      </span>
      <span className="search-locale">{locale.toUpperCase()}</span>
    </div>
  );
}
