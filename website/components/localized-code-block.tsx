'use client';

import { CodeBlock, Pre } from 'fumadocs-ui/components/codeblock';
import type { HTMLAttributes, KeyboardEvent } from 'react';

type LocalizedCodeBlockProps = HTMLAttributes<HTMLPreElement> & {
  accessibleName: string;
};

export function LocalizedCodeBlock({ accessibleName, children, ...props }: LocalizedCodeBlockProps) {
  return (
    <CodeBlock
      {...props}
      viewportProps={{ 'aria-label': accessibleName, onKeyDown: scrollCodeViewport }}
    >
      <Pre>{children}</Pre>
    </CodeBlock>
  );
}

function scrollCodeViewport(event: KeyboardEvent<HTMLElement>) {
  const viewport = event.currentTarget;
  const lineStep = 40;
  const pageStep = Math.max(40, Math.floor(viewport.clientHeight * 0.8));
  const offsets: Partial<Record<string, readonly [number, number]>> = {
    ArrowLeft: [-lineStep, 0],
    ArrowRight: [lineStep, 0],
    ArrowUp: [0, -lineStep],
    ArrowDown: [0, lineStep],
    PageUp: [0, -pageStep],
    PageDown: [0, pageStep],
  };
  const offset = offsets[event.key];
  if (!offset) return;
  const maxLeft = Math.max(0, viewport.scrollWidth - viewport.clientWidth);
  const maxTop = Math.max(0, viewport.scrollHeight - viewport.clientHeight);
  const nextLeft = Math.min(Math.max(0, viewport.scrollLeft + offset[0]), maxLeft);
  const nextTop = Math.min(Math.max(0, viewport.scrollTop + offset[1]), maxTop);
  if (nextLeft === viewport.scrollLeft && nextTop === viewport.scrollTop) return;
  event.preventDefault();
  viewport.scrollTo({ left: nextLeft, top: nextTop, behavior: 'auto' });
}
