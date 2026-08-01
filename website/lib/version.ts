import cliReference from '@/generated/cli-reference.json';
import configReference from '@/generated/config-reference.json';

const buildCommit = process.env.NEXT_PUBLIC_SOURCE_COMMIT ?? 'development';

export const pmokeVersion = cliReference.pmoke_version;
export const configSchemaVersion = configReference.schema_version;
export const sourceCommit = /^[0-9a-f]{40}$/i.test(buildCommit) ? buildCommit : 'development';

export const versionMetadata = {
  pmoke_version: pmokeVersion,
  schema_version: configSchemaVersion,
  source_commit: sourceCommit,
} as const;
