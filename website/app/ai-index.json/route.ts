import { buildAIManifest } from '@/lib/llm-resources';

export const revalidate = false;

export async function GET() {
  return Response.json(await buildAIManifest());
}
