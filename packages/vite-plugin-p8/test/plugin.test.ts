// The plugin in Vite's dev server (middleware mode, behind a Node HTTP
// server), with the example app: PHP pages served, PHP templates rendered.

import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { createServer as createHttpServer, type Server } from 'node:http';
import type { AddressInfo } from 'node:net';
import { join } from 'node:path';
import { after, before, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { createServer, type ViteDevServer } from 'vite';
import p8, { DEFAULT_WASM, renderFile } from '../src/index.ts';

const packageDir = fileURLToPath(new URL('..', import.meta.url));
const example = join(packageDir, 'example');
const fixtures = join(packageDir, 'test/fixtures');
const tmpDir = join(packageDir, 'node_modules/.p8/test-tmp');

let vite: ViteDevServer;
let http: Server;
let base: string;

before(async () => {
  assert.ok(existsSync(DEFAULT_WASM), `php2.wasm is built (npm run build:wasm): ${DEFAULT_WASM}`);
  vite = await createServer({
    root: example,
    configFile: false,
    logLevel: 'silent',
    appType: 'custom',
    server: { middlewareMode: true },
    plugins: [p8({ tmpDir })],
  });
  http = createHttpServer(vite.middlewares);
  await new Promise<void>((done) => http.listen(0, '127.0.0.1', done));
  base = `http://127.0.0.1:${(http.address() as AddressInfo).port}`;
});

after(async () => {
  await new Promise((done) => http.close(done));
  await vite.close();
});

const field = (html: string, id: string) => new RegExp(`<dd id="${id}">([^<]*)</dd>`).exec(html)?.[1];

test('a GET of a .php page is the page PHP prints, with the request in $_SERVER and $_GET', async () => {
  const r = await fetch(`${base}/hello.php?name=ada`, { headers: { 'User-Agent': 'p8-test' } });
  assert.equal(r.status, 200);
  assert.equal(r.headers.get('content-type'), 'text/html; charset=UTF-8');
  assert.equal(r.headers.get('x-rendered-by'), 'p8');
  assert.match(r.headers.get('x-powered-by') ?? '', /^PHP\/8\.4\./);
  const html = await r.text();
  assert.match(html, /<h1>Hello, Ada!<\/h1>/);
  assert.equal(field(html, 'REQUEST_METHOD'), 'GET');
  assert.equal(field(html, 'REQUEST_URI'), '/hello.php?name=ada');
  assert.equal(field(html, 'QUERY_STRING'), 'name=ada');
  assert.equal(field(html, 'SCRIPT_NAME'), '/hello.php');
  assert.equal(field(html, 'SCRIPT_FILENAME'), join(example, 'hello.php'));
  assert.equal(field(html, 'DOCUMENT_ROOT'), example);
  assert.equal(field(html, 'HTTP_USER_AGENT'), 'p8-test');
  assert.equal(field(html, 'PHP_SAPI'), 'cli-server');
});

test('a POSTed form reaches $_POST; the cookie PHP sets comes back in $_COOKIE', async () => {
  const posted = await fetch(`${base}/hello.php`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body: 'name=grace+hopper',
  });
  assert.equal(posted.status, 200);
  const html = await posted.text();
  assert.match(html, /<h1>Hello, Grace hopper!<\/h1>/);
  assert.equal(field(html, 'REQUEST_METHOD'), 'POST');
  const cookie = posted.headers.get('set-cookie');
  assert.equal(cookie, 'last_name=grace%20hopper');
  const again = await (await fetch(`${base}/hello.php`, { headers: { Cookie: cookie! } })).text();
  assert.equal(field(again, 'cookie'), 'grace hopper');
});

test('PATH_INFO after a script reaches it; a .php file that is not there is left to Vite', async () => {
  const html = await (await fetch(`${base}/hello.php/extra/path`)).text();
  assert.match(html, /<h1>Hello, World!<\/h1>/);
  assert.equal(field(html, 'SCRIPT_NAME'), '/hello.php');
  assert.equal(field(html, 'PATH_INFO'), '/extra/path');
  const missing = await fetch(`${base}/missing.php`);
  assert.equal(missing.status, 404);
  assert.doesNotMatch(await missing.text(), /PHP|<h1>/);
});

test('other files are left to Vite', async () => {
  const r = await fetch(`${base}/main.js`);
  assert.equal(r.status, 200);
  assert.match(await r.text(), /card\.render\(person\)/);
});

test("import of a .php file on the server: render(props) is the template's HTML, $props its props", async () => {
  const card = await vite.ssrLoadModule('/card.php');
  assert.equal(card.file, join(example, 'card.php'));
  const html: string = await card.render({ name: 'Ada <Lovelace>', born: 1815, tags: ['math', 'engines'] });
  assert.match(html, /<h3>Ada &lt;Lovelace&gt;<\/h3>/);
  assert.match(html, /<li>math<\/li>\s*<li>engines<\/li>/);
  assert.match(html, /<footer>card\.php · PHP 8\.4\.\d+<\/footer>/);
  assert.equal(card.default.render, card.render);
});

test('import of a .php file in the browser: render(props) asks the dev server', async () => {
  const module = await vite.transformRequest('/card.php');
  assert.match(module?.code ?? '', /fetch\("\/@p8\/render"/);
  const r = await fetch(`${base}/@p8/render`, { method: 'POST', body: JSON.stringify({ file: '/card.php', props: { name: 'Rasmus', tags: ['php'] } }) });
  assert.equal(r.status, 200);
  const html = await r.text();
  assert.match(html, /<h3>Rasmus<\/h3>/);
  assert.match(html, /<li>php<\/li>/);
  // Only a .php file under the root.
  const out = await fetch(`${base}/@p8/render`, { method: 'POST', body: JSON.stringify({ file: '/../test/fixtures/files.php', props: null }) });
  assert.equal(out.status, 404);
});

test('the root is read-only to PHP; its TMPDIR is not', async () => {
  const config = { root: fixtures, wasm: DEFAULT_WASM, readOnly: [], tmpDir, ini: {} };
  const out = await renderFile(config, join(fixtures, 'files.php'), { tmp: true });
  assert.match(out, /read: <\?php/);
  assert.match(out, /write root: false \(file_put_contents\(.*written\.txt\): Failed to open stream: Read-only file system\)/);
  assert.equal(existsSync(join(fixtures, 'written.txt')), false);
  assert.match(out, new RegExp(`write tmp: 2 in ${tmpDir.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}`));
  assert.match(out, /props: \{"tmp":true\}/);
});
