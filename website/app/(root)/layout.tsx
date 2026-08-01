import type { ReactNode } from 'react';
import { FontVariables } from '@/components/font-variables';
import '../global.css';

export default function RootRouteLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" className={FontVariables} suppressHydrationWarning>
      <body>
        {children}
      </body>
    </html>
  );
}
