import devServer from '@hono/vite-dev-server';
import { defineConfig } from 'vite';
import p8 from '../src/index.ts';

export default defineConfig({
  plugins: [
    p8(),
    devServer({ entry: 'src/app.ts' }),
  ],
});
