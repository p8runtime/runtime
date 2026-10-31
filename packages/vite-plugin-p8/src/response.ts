// Server-side rendering helpers, with no Node or Vite imports: what a
// route handler (Hono, a Worker) uses with a `.php` module.
//
//   import Page from './Page.php';
//   import { renderToResponse } from 'vite-plugin-p8/runtime';
//
//   // The request itself: the page reads $_GET, $_POST, $_COOKIE, $_SERVER.
//   app.get('/hello', (c) => renderToResponse(Page, { request: c.req.raw }));
//
//   // A template given data the route computed: the page reads $props.
//   app.get('/card/:name', (c) => renderToResponse(Card, { props: { name: c.req.param('name') } }));

/** What `import page from './x.php'` gives on the server. */
export interface PhpModule {
  file: string;
  /** The template run with `props` as `$props`: what it prints. */
  render(props?: unknown): Promise<string>;
  /** The file run for `request`, as `php -S` runs a router script. */
  fetch?(request: Request): Promise<Response>;
}

export interface RenderOptions {
  /** The request PHP sees: its superglobals and php://input, and the
   * status and headers PHP sets are the response's. */
  request?: Request;
  /** The template's `$props` (JSON objects arrive as associative arrays). */
  props?: unknown;
  status?: number;
  headers?: HeadersInit;
}

/** The page as a Response: PHP's own for a request, or the template
 * rendered with props as HTML. One or the other: a web request has no
 * place for props, as PHP's has none. */
export async function renderToResponse(page: PhpModule, options: RenderOptions = {}): Promise<Response> {
  if (options.request) {
    if (options.props !== undefined) throw new TypeError('renderToResponse: pass request or props, not both');
    if (!page.fetch) throw new TypeError(`renderToResponse: ${page.file} can only render props here (a browser module)`);
    const response = await page.fetch(options.request);
    if (options.status === undefined && options.headers === undefined) return response;
    const headers = new Headers(response.headers);
    new Headers(options.headers).forEach((value, name) => headers.set(name, value));
    return new Response(response.body, { status: options.status ?? response.status, headers });
  }
  const html = await page.render(options.props);
  const headers = new Headers(options.headers);
  if (!headers.has('content-type')) headers.set('content-type', 'text/html; charset=utf-8');
  return new Response(html, { status: options.status ?? 200, headers });
}
