// The Hono example: a route that renders a PHP page with
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
import { renderToResponse } from '../src/response.ts';

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
    plugins: [p8({ tmpDir, readOnly: [join(packageDir, 'test/fixtures')] }), devServer({ entry: 'src/app.ts' })],
  });
  http = createHttpServer(vite.middlewares);
  await new Promise<void>((done) => http.listen(0, '127.0.0.1', done));
  base = `http://127.0.0.1:${(http.address() as AddressInfo).port}`;
});

after(async () => {
  await new Promise((done) => http.close(done));
  await vite.close();
});

test('a page given props reads them as $props', async () => {
  const r = await fetch(`${base}/hello/Ada%20%3Cb%3E`);
  assert.equal(r.status, 200);
  assert.equal(r.headers.get('content-type'), 'text/html; charset=utf-8');
  const html = await r.text();
  // The path parameter reached PHP as $props['name'], escaped by the page.
  assert.match(html, /<h1>Hello Ada &lt;b&gt;<\/h1>/);
  assert.match(html, /PHP rendered by p8 inside a Hono route/);
});

test('a page given the request reads $_GET and $_SERVER, and sets headers', async () => {
  const page = await vite.ssrLoadModule(join(packageDir, 'test/fixtures/Request.php'));
  const r = await renderToResponse(page.default, {
    request: new Request('http://example.test/hi?name=Ada%20%3Cb%3E'),
  });
  assert.equal(r.status, 200);
  // header() in PHP is the response's.
  assert.equal(r.headers.get('x-greeting'), 'hello');
  assert.equal(await r.text(), 'GET /hi?name=Ada%20%3Cb%3E Ada &lt;b&gt;');
});

test('a path no route takes is Hono\'s 404', async () => {
  const r = await fetch(`${base}/nope`);
  assert.equal(r.status, 404);
});
