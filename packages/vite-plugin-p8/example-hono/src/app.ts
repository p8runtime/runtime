import { Hono } from 'hono';
import { renderToResponse } from 'vite-plugin-p8/runtime';
import Card from './components/Card.php';
import Page from './components/Page.php';

const app = new Hono();

// A PHP page for the request itself: it reads $_GET, $_SERVER, cookies.
app.get('/hello', (c) => renderToResponse(Page, { request: c.req.raw }));

// A PHP template given data the route computed: it reads $props.
app.get('/card/:name', (c) =>
  renderToResponse(Card, {
    props: { name: c.req.param('name') },
  }),
);

export default app;
