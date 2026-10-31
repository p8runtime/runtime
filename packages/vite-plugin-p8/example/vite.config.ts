import { defineConfig } from 'vite';
import p8 from '../src/index.ts';

export default defineConfig({
  plugins: [p8()],
});
