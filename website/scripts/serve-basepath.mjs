import { createReadStream } from 'node:fs';
import { stat } from 'node:fs/promises';
import { createServer } from 'node:http';
import path from 'node:path';
import { createGzip } from 'node:zlib';

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
    const contentType = mime.get(path.extname(target)) ?? 'application/octet-stream';
    const headers = {
      'Content-Type': contentType,
      'Cache-Control': relative.startsWith('_next/static/')
        ? 'public, max-age=31536000, immutable'
        : 'public, max-age=0, must-revalidate',
    };
    const compress = metadata.size > 1_024
      && request.headers['accept-encoding']?.includes('gzip')
      && /^(?:text\/|application\/(?:javascript|json|xml))/u.test(contentType);
    if (compress) {
      response.writeHead(200, { ...headers, 'Content-Encoding': 'gzip', Vary: 'Accept-Encoding' });
      createReadStream(target).pipe(createGzip({ level: 9 })).pipe(response);
    } else {
      response.writeHead(200, { ...headers, 'Content-Length': metadata.size });
      createReadStream(target).pipe(response);
    }
  } catch {
    response.writeHead(404).end('Not found');
  }
}).listen(port, '127.0.0.1', () => console.log(`Serving ${root} at http://127.0.0.1:${port}${basePath}/`));
