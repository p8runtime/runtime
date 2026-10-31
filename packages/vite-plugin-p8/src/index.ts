// vite-plugin-p8: PHP pages and templates in a Vite project, run by p8
// (a PHP 8.4 runtime in Rust) built for WebAssembly (wasm32-wasip1).
//
//   import p8 from 'vite-plugin-p8';
//   export default { plugins: [p8()] };
//
// - The dev server answers a request for a .php file under `root` with what
//   the script prints, as `php -S` would serve it.
// - `import page from './page.php'` gives `{ file, render }`;
//   `await render(props)` is the HTML the template prints with `$props`.

import { existsSync, mkdirSync, statSync } from 'node:fs';
import type { IncomingMessage, ServerResponse } from 'node:http';
import { dirname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import type { Plugin, ViteDevServer } from 'vite';
import { render, serve, type Host, type PhpRequest } from './php.ts';
import { P8 } from './runtime.ts';

export interface P8Options {
  /** The document root PHP pages are served from (default: Vite's root). */
  root?: string;
  /** php2.wasm (default: the one beside this package's package.json). */
  wasm?: string;
  /** More directories PHP may read (a vendor directory outside the root). */
  readOnly?: string[];
  /** A directory PHP may write, its TMPDIR (default: node_modules/.p8/tmp under Vite's root; false: none). */
  tmpDir?: string | false;
  /** `-d name=value` settings for every run. */
  ini?: Record<string, string>;
}

const here = dirname(fileURLToPath(import.meta.url));
/** This package's directory (src/ or dist/ is one below it). */
const packageDir = resolve(here, '..');
export const DEFAULT_WASM = join(packageDir, 'php2.wasm');

const loaded = new Map<string, Promise<P8>>();

/** php2.wasm compiled, once per file per process. */
export function loadP8(wasm: string = DEFAULT_WASM): Promise<P8> {
  let p8 = loaded.get(wasm);
  if (!p8) {
    p8 = P8.load(wasm);
    loaded.set(wasm, p8);
  }
  return p8;
}

/** The settings a module made by `import './x.php'` renders with. */
export interface RenderConfig {
  root: string;
  wasm: string;
  readOnly: string[];
  tmpDir?: string;
  ini: Record<string, string>;
}

async function host(config: RenderConfig): Promise<Host> {
  return { p8: await loadP8(config.wasm), root: config.root, readOnly: config.readOnly, tmpDir: config.tmpDir, ini: config.ini };
}

/** Render a PHP template with `props` as `$props`: what it prints. */
export async function renderFile(config: RenderConfig, file: string, props?: unknown): Promise<string> {
  return render(await host(config), file, props);
}

/** Run `file` for a web request, as `php -S` runs a router script: PHP
 * gets the request itself ($_GET, $_POST, $_COOKIE, $_SERVER, php://input)
 * and its response (status, header(), the body) is the Response. */
export async function fetchFile(config: RenderConfig, file: string, request: Request): Promise<Response> {
  const url = new URL(request.url);
  const body = request.method === 'GET' || request.method === 'HEAD' ? new Uint8Array() : new Uint8Array(await request.arrayBuffer());
  const response = await serve(
    await host(config),
    {
      method: request.method,
      url: url.pathname + url.search,
      headers: [...request.headers],
      body,
      server: { host: url.hostname, port: Number(url.port || (url.protocol === 'https:' ? 443 : 80)) },
      client: { address: '127.0.0.1', port: 0 },
    },
    file,
  );
  if (response.log.trim()) console.warn(response.log.trim());
  const headers = new Headers();
  for (const [name, value] of response.headers) {
    if (!['connection', 'transfer-encoding', 'keep-alive'].includes(name.toLowerCase())) headers.append(name, value);
  }
  return new Response(response.body.length > 0 ? new Uint8Array(response.body) : null, { status: response.status, headers });
}

/** Whether a request path names a PHP script under `root` for PHP to serve. */
function phpTarget(root: string, pathname: string): boolean {
  // A .php file that is there, possibly with PATH_INFO after it
  // (/index.php/extra). One that is not is Vite's to answer, not php -S's
  // (which would fall back to the root's index file).
  const script = /^(.*?\.php)(\/|$)/i.exec(pathname)?.[1];
  if (script) return statSync(join(root, decodeURIComponent(script)), { throwIfNoEntry: false })?.isFile() ?? false;
  // A directory with an index.php and no index.html (Vite serves that).
  if (pathname.endsWith('/')) {
    const dir = join(root, decodeURIComponent(pathname));
    return existsSync(join(dir, 'index.php')) && !existsSync(join(dir, 'index.html'));
  }
  return false;
}

function readBody(req: IncomingMessage): Promise<Buffer> {
  return new Promise((done, fail) => {
    const chunks: Buffer[] = [];
    req.on('data', (c: Buffer) => chunks.push(c));
    req.on('end', () => done(Buffer.concat(chunks)));
    req.on('error', fail);
  });
}

const plain = (a: string | undefined) => (a ?? '127.0.0.1').replace(/^::ffff:/, '');

/** A Node request as php2 should see it off a connection. */
async function phpRequest(req: IncomingMessage): Promise<PhpRequest> {
  const headers: [string, string][] = [];
  for (let i = 0; i + 1 < req.rawHeaders.length; i += 2) headers.push([req.rawHeaders[i], req.rawHeaders[i + 1]]);
  const [hostName, hostPort] = (req.headers.host ?? '').split(/:(?=\d+$)/);
  return {
    method: req.method ?? 'GET',
    url: req.url ?? '/',
    headers,
    body: await readBody(req),
    server: { host: plain(req.socket?.localAddress) || hostName || 'localhost', port: req.socket?.localPort ?? Number(hostPort || 80) },
    client: { address: plain(req.socket?.remoteAddress), port: req.socket?.remotePort ?? 0 },
  };
}

const VIRTUAL = '\0p8:';
/** Where the plugin leaves renderFile for the SSR modules it makes: they
 * run in the same process (the dev server's), and use the wasm the plugin
 * compiled. Outside it (a built server bundle) they import the package. */
const RENDERER = Symbol.for('vite-plugin-p8.renderFile');
const FETCHER = Symbol.for('vite-plugin-p8.fetchFile');
const RENDER_ENDPOINT = '/@p8/render';

export default function p8(options: P8Options = {}): Plugin {
  let config: RenderConfig;

  const send = (res: ServerResponse, status: number, headers: [string, string][], body: Buffer) => {
    res.statusCode = status;
    const hop = new Set(['connection', 'host', 'keep-alive', 'transfer-encoding']);
    for (const [name, value] of headers) {
      if (!hop.has(name.toLowerCase())) res.appendHeader(name, value);
    }
    res.end(body);
  };

  return {
    name: 'vite-plugin-p8',
    // Before Vite's resolver, which would take a .php file as a plain asset.
    enforce: 'pre',

    configResolved(resolved) {
      const tmpDir = options.tmpDir === false ? undefined : resolve(options.tmpDir ?? join(resolved.root, 'node_modules/.p8/tmp'));
      if (tmpDir) mkdirSync(tmpDir, { recursive: true });
      (globalThis as Record<symbol, unknown>)[RENDERER] = renderFile;
      (globalThis as Record<symbol, unknown>)[FETCHER] = fetchFile;
      config = {
        root: resolve(options.root ?? resolved.root),
        wasm: resolve(options.wasm ?? DEFAULT_WASM),
        readOnly: (options.readOnly ?? []).map((d) => resolve(d)),
        tmpDir,
        ini: options.ini ?? {},
      };
    },

    configureServer(server: ViteDevServer) {
      // Before Vite's own middlewares: a .php request is PHP's.
      server.middlewares.use(async (req, res, next) => {
        try {
          const pathname = new URL(req.url ?? '/', 'http://localhost').pathname;
          if (req.method === 'POST' && pathname === RENDER_ENDPOINT) {
            // A browser module's render(): the template named, with its props.
            const { file, props } = JSON.parse((await readBody(req)).toString());
            const path = resolve(config.root, String(file).replace(/^\//, ''));
            // Only a .php file under the root: never a path that climbs out.
            if (!path.startsWith(config.root + sep) || !/\.php$/i.test(path)) return send(res, 404, [], Buffer.from('Not found'));
            const html = await renderFile(config, path, props);
            return send(res, 200, [['Content-Type', 'text/html; charset=UTF-8']], Buffer.from(html));
          }
          if (!phpTarget(config.root, pathname)) return next();
          const response = await serve(await host(config), await phpRequest(req));
          if (response.log.trim()) server.config.logger.warn(response.log.trim());
          send(res, response.status, response.headers, response.body);
        } catch (e) {
          next(e);
        }
      });
    },

    resolveId(id, importer) {
      if (!/\.php$/i.test(id) || id.includes('?')) return null;
      // `/page.php` is under the root, as Vite reads such an id; a path
      // relative to the importing module otherwise.
      const candidates = id.startsWith('/') ? [join(config.root, id), id] : [resolve(importer ? dirname(importer) : config.root, id)];
      const file = candidates.find((f) => statSync(f, { throwIfNoEntry: false })?.isFile());
      return file ? VIRTUAL + file : null;
    },

    load(id, opts) {
      if (!id.startsWith(VIRTUAL)) return null;
      const file = id.slice(VIRTUAL.length);
      this.addWatchFile(file);
      if (opts?.ssr) {
        // On the server: rendered here, by the wasm.
        return [
          `const renderFile = globalThis[Symbol.for('vite-plugin-p8.renderFile')] ?? (await import('vite-plugin-p8')).renderFile;`,
          `const fetchFile = globalThis[Symbol.for('vite-plugin-p8.fetchFile')] ?? (await import('vite-plugin-p8')).fetchFile;`,
          `const config = ${JSON.stringify(config)};`,
          `export const file = ${JSON.stringify(file)};`,
          `export function render(props) { return renderFile(config, file, props); }`,
          `export function fetch(request) { return fetchFile(config, file, request); }`,
          `export default { file, render, fetch };`,
        ].join('\n');
      }
      // In the browser: rendered by the dev server.
      const name = '/' + file.slice(config.root.length + 1);
      return [
        `export const file = ${JSON.stringify(name)};`,
        `export async function render(props) {`,
        `  const r = await fetch(${JSON.stringify(RENDER_ENDPOINT)}, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ file, props }) });`,
        `  if (!r.ok) throw new Error(await r.text());`,
        `  return r.text();`,
        `}`,
        `export default { file, render };`,
      ].join('\n');
    },
  };
}
