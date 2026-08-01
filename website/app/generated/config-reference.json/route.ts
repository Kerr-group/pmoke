import configReference from '@/generated/config-reference.json';
import { sourceCommit } from '@/lib/version';

export const dynamic = 'force-static';

export function GET() {
  return Response.json({ ...configReference, source_commit: sourceCommit });
}
