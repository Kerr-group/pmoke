import { Inter, JetBrains_Mono, Noto_Sans_JP } from 'next/font/google';

const inter = Inter({ subsets: ['latin'], variable: '--font-inter', display: 'swap' });
const notoSansJp = Noto_Sans_JP({ subsets: ['latin'], variable: '--font-noto-jp', display: 'swap' });
const jetBrainsMono = JetBrains_Mono({
  subsets: ['latin'],
  variable: '--font-mono',
  display: 'swap',
});

export const FontVariables = `${inter.variable} ${notoSansJp.variable} ${jetBrainsMono.variable}`;
