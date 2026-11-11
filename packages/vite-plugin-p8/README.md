# vite-plugin-p8

PHP pages and PHP templates in a Vite project, run by
[p8](../../README.md) (a PHP 8.6 runtime written in Rust) compiled to
WebAssembly (`wasm32-wasip1`) and run in Node through `node:wasi`. No PHP
install and no native binary is needed.

```js
// vite.config.js
import p8 from 'vite-plugin-p8';

export default { plugins: [p8()] };
```

- **Pages.** In the dev server, a request for a `.php` file under the root
  (a path ending in `.php`, or `.php/` with PATH_INFO after it, or a
  directory that has an `index.php` and no `index.html`) is answered by PHP:
  what the script prints is the response, with the status and headers it
  set.
- **Templates.** `import page from './page.php'` gives
  `{ file, render }`. `await render(props)` is the HTML the template prints;
  the props are the PHP global `$props`.

This is an experiment. The package is private and not published.

## Setup

p8.wasm is built from this repository:

```sh
npm install
npm run build:wasm   # cargo build --profile wasm -p php-cli --bin p8 --target wasm32-wasip1, copied to ./p8.wasm
npm test
npm run dev          # the example app, on http://localhost:5173
```

The Rust toolchain needs the `wasm32-wasip1` standard library. If the
`cargo` on PATH lacks it, name a rustup toolchain's binaries:
`CARGO=~/.rustup/toolchains/<toolchain>/bin/cargo RUSTC=~/.rustup/toolchains/<toolchain>/bin/rustc npm run build:wasm`.
Node 23.6 or newer runs the TypeScript sources as they are.

## Options

```ts
p8({
  root,      // the document root PHP pages are served from (default: Vite's root)
  wasm,      // p8.wasm (default: the one beside package.json)
  readOnly,  // more directories PHP may read, e.g. a vendor/ outside the root
  tmpDir,    // a directory PHP may write, its TMPDIR (default: <root>/node_modules/.p8/tmp; false: none)
  ini,       // { name: value } -d settings for every run
})
```

## How it runs PHP

**One instance per run.** The WebAssembly module is compiled once per
process. Every page request and every `render()` instantiates it again and
runs p8's `main`: a WASI command runs `main` once per instance, and p8
serves one script or one request per `main`, as `php` does per process. No
PHP state survives between runs (no opcache across requests, no persistent
connections). The cost of that is measured below.

**Files.** The root (and each `readOnly` directory) is preopened at its own
absolute path, so `__FILE__`, `__DIR__`, `$_SERVER['SCRIPT_FILENAME']`,
include paths and error messages name the same files the native p8
would. These directories are read-only: `node:wasi` grants a preopen every
right, so the plugin refuses the WASI calls that would create, change or
remove anything under them, with `EROFS`, which PHP reports as
"Read-only file system". `tmpDir` is preopened writable and is PHP's
`TMPDIR` (`sys_get_temp_dir()`, uploads, session files). Nothing else on
the machine is visible to PHP. stdin, stdout and stderr are buffers of the
run, not the Node process's.

**A page request** is p8's built-in web server (`php -S`, PHP's
cli-server SAPI) serving one request:

```sh
p8 -S <server host>:<port> --stdio <client address>:<port> -t <root> -q
```

with the raw HTTP/1.1 request (method, target, headers, body) on stdin and
the raw HTTP response read from stdout. `--stdio` is the one option this
work adds to p8: it hands `php -S` one connection's bytes over stdin and
stdout instead of a socket, which a WASI program does not have. Everything
else is what `php -S` does with a request off a socket: `$_SERVER`
(REQUEST_URI, REQUEST_METHOD, QUERY_STRING, SCRIPT_NAME, SCRIPT_FILENAME,
DOCUMENT_ROOT, PATH_INFO, PHP_SELF, REMOTE_ADDR, SERVER_NAME, SERVER_PORT,
HTTP_* for each header), `$_GET`, `$_POST`, `$_COOKIE`, `$_FILES`,
`php://input`, `header()`, `setcookie()`, `http_response_code()`, its 404
page, `PHP_SAPI` being `cli-server`. The plugin makes up none of these
values. The server address is the socket's local address, the client's its
remote address. A request body is read whole first, so the request carries
a `Content-Length`.

**A render** is p8's `-r`:

```sh
p8 -r '$props = json_decode(stream_get_contents(STDIN), true); require $argv[1];' -- /abs/path/page.php
```

with `JSON.stringify(props)` on stdin. So `$props` is a global, decoded as
`json_decode(..., true)` decodes it: JS objects become associative arrays,
arrays lists, `undefined` props `null`. The template runs in the global
scope with its own `__FILE__` and `__DIR__`; `$argv` is
`['Standard input code', '/abs/path/page.php']` and `PHP_SAPI` is `cli`.
Stdin was chosen over a prepended data file because p8 reads it already
and nothing has to be written to disk; over environment variables because
those would also land in `$_SERVER` and `$_ENV`. A run that exits non-zero
(a fatal error) rejects with what it printed.

On the server (SSR, `ssrLoadModule`) the module renders in the Vite
process. In the browser the module's `render()` posts `{ file, props }` to
the dev server (`/@p8/render`), which renders it there; this is a dev
server feature, and only a `.php` file under the root is rendered.

## What PHP can and cannot do here

p8's WASI build is a PHP built without what WASI lacks; see the pull
request for each extension:

- Not loaded (`extension_loaded()` false, their functions undefined):
  curl, mysqli, pdo_mysql, pdo_pgsql, sockets. WASI preview 1 has no
  sockets.
- Fails as PHP does when the system cannot: `fsockopen()` and stream
  sockets (connection fails), `gethostbyname()` (the name back),
  `gethostbynamel()` (false), `proc_open()`, `exec()`, `shell_exec()`
  ("Exec failed" / false: no processes), `getmypid()` (false),
  `chmod()`, `chown()`, `symlink()` ("Not supported").
- What WASI's stat has: sizes, times and file types; no owners (uid and gid
  0) and no permission bits (`fileperms()` is the file type alone);
  `is_readable()` and `is_writable()` answer as wasi-libc's `access()`.
- `max_execution_time` is not enforced (no threads, as in a PHP built
  without timers). Deep recursion has a 64 MiB stack.
- `php_uname()` is wasi-libc's: `wasi (none) 0.0.0 0.0.0 wasm32`.

## Measurements

The example's `card.php` render and `GET /hello.php`, measured by
`npm run bench` (Node 26, Intel iMac):

| | cold | warm (median of 20) |
|---|---|---|
| compile p8.wasm (24.7 MB), once per process | 43 ms | — |
| `render()` of `card.php` | 37 ms (first run) | 13 ms |
| `GET /hello.php` (with an include) | 18 ms (first request) | 13 ms |

V8 compiles the module lazily, so the first run also pays for compiling
the functions it reaches. Nearly all of a warm run is per-instance setup
(a new instance with its 64 MiB stack, p8's start-up, its extensions'
registration), not the script: every run starts PHP afresh, as
`php script.php` does. CPU-bound PHP is close to native: a recursive
`fib(27)` takes 67–73 ms in the wasm against 58 ms in the native p8
(about 1.2×).

## Cloudflare Workers and Durable Objects

Not built; what would carry over:

- **Carries over.** The same p8.wasm: a `wasm32-wasip1` command with no
  threads and no sockets is what Workers run. The request protocol
  (`-S ... --stdio`, raw HTTP in and out) is independent of Node, as is the
  render bootstrap. The read-only file policy is a filter over WASI calls
  and moves as it is.
- **Needs replacing.** `node:wasi` (Workers have no WASI host of their own):
  a small preview1 shim over an in-memory file system, such as
  `@bjorn3/browser_wasi_shim`, or a hand-written one. The PHP files then
  come from the Worker bundle or from R2/KV, not a preopened directory; a
  build step would pack the root.
- **Limits to check.** Workers compile WebAssembly at deploy time
  (no `WebAssembly.compile` of fetched bytes), which suits a fixed
  p8.wasm; the 24.7 MB module (the `wasm` profile: `opt-level = "s"`, stripped)
  is 8.2 MB gzipped: over the free plan's 3 MB limit and under the paid
  plan's 10 MB. `wasm-opt -Oz`, or leaving out intl's data or gd, would
  shrink it further. The 64
  MiB stack plus the heap must fit in a Worker's 128 MB.
- **A Durable Object** could keep one instance's state across requests
  only if p8 served many requests per instance (a reactor-style entry
  point instead of `main`), which is the larger change: today each request
  is a new instance, so a Durable Object would add a per-object writable
  directory (session files, SQLite through pdo_sqlite) kept in its storage,
  not a long-lived PHP process.

## With Hono (server-side rendering)

On the server, `renderToResponse` (`vite-plugin-p8/runtime`, no Node or
Vite imports) renders a `.php` page into a Response. Give it `{ props }` and
the page reads them as `$props`; give it `{ request }` instead and the page
gets the request itself, as under `php -S` (`$_GET`, `$_SERVER`, `header()`).

```ts
// vite.config.ts
import devServer from '@hono/vite-dev-server';
import { defineConfig } from 'vite';
import p8 from 'vite-plugin-p8';

export default defineConfig({
  plugins: [p8(), devServer({ entry: 'src/app.ts' })],
});
```

```ts
// src/app.ts
import { Hono } from 'hono';
import { renderToResponse } from 'vite-plugin-p8/runtime';
import Page from './components/Page.php';

const app = new Hono();

// /hello/Ada: Page.php reads $props['name'].
app.get('/hello/:name', (c) =>
  renderToResponse(Page, { props: { name: c.req.param('name') } }),
);

export default app;
```

Run `npm run dev:hono` for `example-hono/`, then open
`http://localhost:5173/hello/YourName`.
