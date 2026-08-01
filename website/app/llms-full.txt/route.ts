import { getLLMText, source } from '@/lib/source';
import { versionMetadata } from '@/lib/version';

export const revalidate = false;

export async function GET() {
  const scan = source.getPages().map(getLLMText);
  const scanned = await Promise.all(scan);

  const metadata = `# pmoke full documentation\n\n- pmoke: ${versionMetadata.pmoke_version}\n- config schema: ${versionMetadata.schema_version}\n- source commit: ${versionMetadata.source_commit}`;

  return new Response([metadata, ...scanned].join('\n\n'), {
    headers: { 'Content-Type': 'text/plain; charset=utf-8' },
  });
}
