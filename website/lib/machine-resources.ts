export type MachineResourceGroup = 'discovery' | 'context' | 'contract';

export type MachineResource = {
  id: string;
  path: string;
  mediaType: string;
  group: MachineResourceGroup;
  locale: 'en' | 'ja' | 'bilingual' | 'neutral';
};

export const machineResources = [
  {
    id: 'llms-index',
    path: '/llms.txt',
    mediaType: 'text/plain',
    group: 'discovery',
    locale: 'bilingual',
  },
  {
    id: 'manifest',
    path: '/ai-index.json',
    mediaType: 'application/json',
    group: 'discovery',
    locale: 'bilingual',
  },
  {
    id: 'english-context',
    path: '/llms-en.txt',
    mediaType: 'text/plain',
    group: 'context',
    locale: 'en',
  },
  {
    id: 'japanese-context',
    path: '/llms-ja.txt',
    mediaType: 'text/plain',
    group: 'context',
    locale: 'ja',
  },
  {
    id: 'full-context',
    path: '/llms-full.txt',
    mediaType: 'text/plain',
    group: 'context',
    locale: 'bilingual',
  },
  {
    id: 'cli-contract',
    path: '/generated/cli-reference.json',
    mediaType: 'application/json',
    group: 'contract',
    locale: 'neutral',
  },
  {
    id: 'config-contract',
    path: '/generated/config-reference.json',
    mediaType: 'application/json',
    group: 'contract',
    locale: 'neutral',
  },
  {
    id: 'config-schema',
    path: '/config.schema.json',
    mediaType: 'application/json',
    group: 'contract',
    locale: 'neutral',
  },
] as const satisfies readonly MachineResource[];
