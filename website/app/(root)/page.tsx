import Link from 'next/link';
import { Activity } from 'lucide-react';

export default function LocaleIndexPage() {
  return (
    <main className="locale-index">
      <Activity aria-hidden="true" size={30} />
      <p className="eyebrow">PULSED-FIELD MOKE</p>
      <h1>pmoke</h1>
      <p>Select documentation language / ドキュメント言語の選択</p>
      <div className="locale-actions">
        <Link href="/en">English</Link>
        <Link href="/ja" lang="ja">日本語</Link>
      </div>
    </main>
  );
}
