import assert from 'node:assert/strict';
import { once } from 'node:events';
import { createRequire } from 'node:module';
import { createServer } from 'node:net';
import test from 'node:test';
import { pathToFileURL } from 'node:url';

const require = createRequire(import.meta.url);
const cliRequire = createRequire(require.resolve('@lhci/cli/package.json'));
const lighthousePath = cliRequire.resolve('lighthouse');
const lighthouseRequire = createRequire(lighthousePath);
const proxyRequire = createRequire(cliRequire.resolve('proxy-agent'));
const pacRequire = createRequire(proxyRequire.resolve('pac-proxy-agent'));
const getUriPath = pacRequire.resolve('get-uri');
const getUriRequire = createRequire(getUriPath);

// These pins are scoped to the existing dev-tool parents in pnpm-workspace.yaml.
// Review this integration again when changing either override or parent version.
test('Lighthouse keeps its engine while loading the patched Puppeteer chain', async () => {
  assert.equal(cliRequire('lighthouse/package.json').version, '12.6.1');
  assert.equal(lighthouseRequire('puppeteer-core/package.json').version, '25.16.0');
  const lighthouse = await import(pathToFileURL(lighthousePath));
  const puppeteer = await import(pathToFileURL(lighthouseRequire.resolve('puppeteer-core')));
  assert.equal(typeof lighthouse.default, 'function');
  assert.equal(typeof puppeteer.default.connect, 'function');
  assert.equal(typeof puppeteer.default.launch, 'function');
});

test('get-uri can download through the patched FTP client and closes it', { timeout: 10_000 }, async (t) => {
  assert.equal(getUriRequire('get-uri/package.json').version, '6.0.5');
  assert.equal(getUriRequire('basic-ftp/package.json').version, '6.2.1');
  const { getUri } = getUriRequire(getUriPath);
  const payload = Buffer.from('dependency compatibility fixture\n');
  const sockets = new Set();
  const errors = [];
  let acceptData;
  let closeControl;
  const dataConnected = new Promise((resolve) => { acceptData = resolve; });
  const controlClosed = new Promise((resolve) => { closeControl = resolve; });

  function track(socket) {
    sockets.add(socket);
    socket.on('close', () => sockets.delete(socket));
    socket.on('error', (error) => errors.push(error));
  }

  const dataServer = createServer((socket) => {
    track(socket);
    acceptData(socket);
  });
  const controlServer = createServer((socket) => {
    track(socket);
    socket.once('close', closeControl);
    socket.setEncoding('utf8');
    socket.write('220 Local test fixture\r\n');
    let pending = '';
    socket.on('data', (chunk) => {
      pending += chunk;
      let end;
      while ((end = pending.indexOf('\r\n')) !== -1) {
        const command = pending.slice(0, end);
        pending = pending.slice(end + 2);
        const verb = command.split(' ', 1)[0];
        switch (verb) {
          case 'USER': socket.write('331 Continue\r\n'); break;
          case 'PASS': socket.write('230 Logged in\r\n'); break;
          case 'FEAT': socket.write('211 End\r\n'); break;
          case 'OPTS':
          case 'TYPE':
          case 'STRU': socket.write('200 OK\r\n'); break;
          case 'MDTM': socket.write('213 20260102030405\r\n'); break;
          case 'EPSV':
            socket.write(`229 Entering Extended Passive Mode (|||${dataServer.address().port}|)\r\n`);
            break;
          case 'RETR':
            if (command !== 'RETR /fixture.txt') {
              socket.write('550 Not found\r\n');
              break;
            }
            socket.write('150 Transfer starting\r\n');
            dataConnected.then((dataSocket) => {
              dataSocket.end(payload, () => socket.write('226 Transfer complete\r\n'));
            }).catch((error) => {
              errors.push(error);
              socket.destroy();
            });
            break;
          case 'QUIT':
            // Client.close() sends QUIT and destroys its socket without reading a reply.
            socket.end();
            break;
          default: socket.write('502 Unsupported test command\r\n');
        }
      }
    });
  });

  t.after(async () => {
    for (const socket of sockets) socket.destroy();
    await Promise.all([dataServer, controlServer].map((server) => new Promise((resolve, reject) => {
      if (!server.listening) return resolve();
      server.close((error) => error ? reject(error) : resolve());
    })));
  });
  for (const server of [dataServer, controlServer]) {
    const listening = once(server, 'listening');
    server.listen(0, '127.0.0.1');
    await listening;
  }

  const stream = await getUri(`ftp://127.0.0.1:${controlServer.address().port}/fixture.txt`);
  const chunks = [];
  for await (const chunk of stream) chunks.push(chunk);
  assert.deepEqual(Buffer.concat(chunks), payload);
  assert.equal(stream.lastModified.toISOString(), '2026-01-02T03:04:05.000Z');
  await controlClosed;
  assert.deepEqual(errors, []);
});
