import { Hono } from 'hono';
import { renderToResponse } from 'vite-plugin-p8/runtime';
import Page from './components/Page.php';

const app = new Hono();

// Page.php reads $props['name'].
app.get('/hello/:name', (c) =>
  renderToResponse(Page, { props: { name: c.req.param('name') } }),
);

export default app;
