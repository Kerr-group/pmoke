'use client';

import { Moon, Sun } from 'lucide-react';
import { useTheme } from 'next-themes';
import { useSyncExternalStore } from 'react';

const subscribe = () => () => {};

export function ThemeToggle({
  toggleLabel,
  lightLabel,
  darkLabel,
}: {
  toggleLabel: string;
  lightLabel: string;
  darkLabel: string;
}) {
  const { theme, resolvedTheme, setTheme } = useTheme();
  const mounted = useSyncExternalStore(subscribe, () => true, () => false);
  const currentTheme = theme === 'system' ? resolvedTheme : theme;
  const nextTheme = currentTheme === 'dark' ? 'light' : 'dark';
  const label = mounted ? (nextTheme === 'light' ? lightLabel : darkLabel) : toggleLabel;

  return (
    <button
      type="button"
      className="theme-toggle"
      aria-label={label}
      title={label}
      onClick={() => setTheme(nextTheme)}
    >
      <Sun className="theme-icon-to-light" aria-hidden="true" />
      <Moon className="theme-icon-to-dark" aria-hidden="true" />
    </button>
  );
}
