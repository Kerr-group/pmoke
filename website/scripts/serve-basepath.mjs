import { createReadStream } from 'node:fs';
import { stat } from 'node:fs/promises';
import { createServer } from 'node:http';
import path from 'node:path';

const root = path.resolve('out');
const basePath = process.env.NEXT_PUBLIC_BASE_PATH ?? '/pmoke';
const port = Number(process.env.PORT ?? 4173);
const mime = new Map([
  ['.css', 'text/css; charset=utf-8'], ['.html', 'text/html; charset=utf-8'],
  ['.js', 'text/javascript; charset=utf-8'], ['.json', 'application/json; charset=utf-8'],
  ['.png', 'image/png'], ['.txt', 'text/plain; charset=utf-8'],
  ['.wasm', 'application/wasm'], ['.woff2', 'font/woff2'], ['.xml', 'application/xml; charset=utf-8'],
]);

createServer(async (request, response) => {
  try {
    const url = new URL(request.url ?? '/', 'http://localhost');
    if (!url.pathname.startsWith(`${basePath}/`) && url.pathname !== basePath) {
      response.writeHead(404).end('Not found'); return;
    }
    let relative = decodeURIComponent(url.pathname.slice(basePath.length)).replace(/^\/+/, '');
    if (!relative || relative.endsWith('/')) relative += 'index.html';
    let target = path.resolve(root, relative);
    if (!target.startsWith(`${root}${path.sep}`) && target !== root) {
      response.writeHead(400).end('Invalid path'); return;
    }
    let metadata;
    try { metadata = await stat(target); }
    catch {
      target = `${target}.html`;
      metadata = await stat(target);
    }
    if (metadata.isDirectory()) target = path.join(target, 'index.html');
    response.writeHead(200, { 'Content-Type': mime.get(path.extname(target)) ?? 'application/octet-stream' });
    createReadStream(target).pipe(response);
  } catch {
    response.writeHead(404).end('Not found');
  }
}).listen(port, '127.0.0.1', () => console.log(`Serving ${root} at http://127.0.0.1:${port}${basePath}/`));
