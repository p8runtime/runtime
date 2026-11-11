// What a render costs: compiling p8.wasm once (cold), then the first
// and the following runs of the example's template and page.
//
//   npm run bench [-- runs]

import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { DEFAULT_WASM } from '../src/index.ts';
import { render, serve, type Host } from '../src/php.ts';
import { P8 } from '../src/runtime.ts';

const runs = Number(process.argv[2] ?? 20);
const example = fileURLToPath(new URL('../example', import.meta.url));
const ms = (from: number) => performance.now() - from;

let t = performance.now();
const p8 = await P8.load(DEFAULT_WASM);
const compile = ms(t);
const host: Host = { p8, root: example, readOnly: [], ini: {} };
const props = { name: 'Ada Lovelace', born: 1815, tags: ['math', 'engines'] };
const request = () => ({
  method: 'GET',
  url: '/hello.php?name=ada',
  headers: [['Host', 'localhost:5173']] as [string, string][],
  body: new Uint8Array(0),
  server: { host: 'localhost', port: 5173 },
  client: { address: '127.0.0.1', port: 50000 },
});

t = performance.now();
await render(host, join(example, 'card.php'), props);
const firstRender = ms(t);
t = performance.now();
await serve(host, request());
const firstServe = ms(t);

const median = (xs: number[]) => xs.sort((a, b) => a - b)[Math.floor(xs.length / 2)];
const renders: number[] = [];
const serves: number[] = [];
for (let i = 0; i < runs; i++) {
  t = performance.now();
  await render(host, join(example, 'card.php'), props);
  renders.push(ms(t));
  t = performance.now();
  await serve(host, request());
  serves.push(ms(t));
}

const f = (n: number) => `${n.toFixed(1)} ms`;
console.log(`p8.wasm compile (cold, once per process): ${f(compile)}`);
console.log(`card.php render: first ${f(firstRender)}, then median ${f(median(renders))} over ${runs}`);
console.log(`GET /hello.php: first ${f(firstServe)}, then median ${f(median(serves))} over ${runs}`);
