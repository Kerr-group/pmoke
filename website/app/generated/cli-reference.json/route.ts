import cliReference from '@/generated/cli-reference.json';
import { sourceCommit } from '@/lib/version';

export const dynamic = 'force-static';

export function GET() {
  return Response.json({ ...cliReference, source_commit: sourceCommit });
}
