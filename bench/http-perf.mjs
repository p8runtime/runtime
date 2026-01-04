#!/usr/bin/env node
const args = process.argv.slice(2);
const options = {
  url: 'http://127.0.0.1:8541/bench.php?ms=8',
  duration: 10,
  concurrency: 50,
};

for (let i = 0; i < args.length; i += 1) {
  const arg = args[i];
  if (arg === '--url') options.url = args[i + 1];
  if (arg === '--duration') options.duration = Number(args[i + 1]);
  if (arg === '--concurrency') options.concurrency = Number(args[i + 1]);
}

const durationMs = options.duration * 1000;
const endTime = Date.now() + durationMs;
const latencies = [];
let total = 0;
let errors = 0;

async function worker() {
  while (Date.now() < endTime) {
    const start = performance.now();
    try {
      const res = await fetch(options.url, { headers: { 'x-perf': '1' } });
      await res.arrayBuffer();
      const elapsed = performance.now() - start;
      latencies.push(elapsed);
      total += 1;
    } catch {
      errors += 1;
    }
  }
}

const tasks = [];
for (let i = 0; i < options.concurrency; i += 1) {
  tasks.push(worker());
}
await Promise.all(tasks);

latencies.sort((a, b) => a - b);
const p50 = latencies.length ? latencies[Math.floor(latencies.length * 0.5)] : 0;
const p95 = latencies.length ? latencies[Math.floor(latencies.length * 0.95)] : 0;
const rps = total / options.duration;

const summary = {
  url: options.url,
  duration: options.duration,
  concurrency: options.concurrency,
  total,
  errors,
  rps: Number(rps.toFixed(2)),
  p50_ms: Number(p50.toFixed(2)),
  p95_ms: Number(p95.toFixed(2)),
};

console.log(JSON.stringify(summary, null, 2));
