// The Hono example: a route that renders a PHP component with
// renderToResponse, run by @hono/vite-dev-server inside Vite.

import assert from 'node:assert/strict';
import { createServer as createHttpServer, type Server } from 'node:http';
import type { AddressInfo } from 'node:net';
import { join } from 'node:path';
import { after, before, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import devServer from '@hono/vite-dev-server';
import { createServer, type ViteDevServer } from 'vite';
import p8 from '../src/index.ts';

const packageDir = fileURLToPath(new URL('..', import.meta.url));
const example = join(packageDir, 'example-hono');
const tmpDir = join(packageDir, 'node_modules/.p8/test-tmp');

let vite: ViteDevServer;
let http: Server;
let base: string;

before(async () => {
  vite = await createServer({
    root: example,
    configFile: false,
    logLevel: 'silent',
    server: { middlewareMode: true },
    plugins: [p8({ tmpDir }), devServer({ entry: 'src/app.ts' })],
  });
  http = createHttpServer(vite.middlewares);
  await new Promise<void>((done) => http.listen(0, '127.0.0.1', done));
  base = `http://127.0.0.1:${(http.address() as AddressInfo).port}`;
});

after(async () => {
  await new Promise((done) => http.close(done));
  await vite.close();
});

test('a page given the request reads $_GET and $_SERVER, and sets headers', async () => {
  const r = await fetch(`${base}/hello?name=Ada%20%3Cb%3E`);
  assert.equal(r.status, 200);
  // header() in PHP is the response's.
  assert.equal(r.headers.get('x-greeting'), 'hello');
  const html = await r.text();
  // The query string reached PHP as $_GET, escaped by the page.
  assert.match(html, /<h1>Hello Ada &lt;b&gt;<\/h1>/);
  assert.match(html, /GET \/hello\?name=Ada%20%3Cb%3E,/);
  assert.match(html, /rendered by PHP 8\.4\.\d+ on p8/);
});

test('a template given props reads them as $props', async () => {
  const r = await fetch(`${base}/card/Grace%20%3Ci%3E`);
  assert.equal(r.status, 200);
  assert.equal(r.headers.get('content-type'), 'text/html; charset=utf-8');
  assert.match(await r.text(), /<h2>Grace &lt;i&gt;<\/h2>/);
});

test('a path no route takes is Hono\'s 404', async () => {
  const r = await fetch(`${base}/nope`);
  assert.equal(r.status, 404);
});
