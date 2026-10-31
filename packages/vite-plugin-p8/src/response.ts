// Server-side rendering helpers, with no Node or Vite imports: what a
// route handler (Hono, a Worker) uses with a `.php` module.
//
//   import Page from './Page.php';
//   import { renderToResponse } from 'vite-plugin-p8/runtime';
//   app.get('/hello/:name', (c) => renderToResponse(Page, { props: { name: c.req.param('name') } }));

/** What `import page from './x.php'` gives. */
export interface PhpModule {
  file: string;
  render(props?: unknown): Promise<string>;
}

export interface RenderOptions {
  /** The template's `$props` (JSON objects arrive as associative arrays). */
  props?: unknown;
  status?: number;
  headers?: HeadersInit;
}

/** The template rendered with `props`, as an HTML Response. */
export async function renderToResponse(page: PhpModule, options: RenderOptions = {}): Promise<Response> {
  const html = await page.render(options.props);
  const headers = new Headers(options.headers);
  if (!headers.has('content-type')) headers.set('content-type', 'text/html; charset=utf-8');
  return new Response(html, { status: options.status ?? 200, headers });
}
