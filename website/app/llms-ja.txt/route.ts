import { buildFullLLMText } from '@/lib/llm-resources';

export const revalidate = false;

export async function GET() {
  return new Response(await buildFullLLMText(['ja']), {
    headers: { 'Content-Type': 'text/plain; charset=utf-8' },
  });
}
