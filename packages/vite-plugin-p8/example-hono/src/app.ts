import { Hono } from 'hono';
import { renderToResponse } from 'vite-plugin-p8/runtime';
import Page from './components/Page.php';

const app = new Hono();

// Page.php gets the request itself: $_GET, $_SERVER, header().
app.get('/hello', (c) => renderToResponse(Page, { request: c.req.raw }));

export default app;
