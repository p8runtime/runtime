// Build p8 for wasm32-wasip1 (the workspace's `wasm` profile: release,
// optimized for size, stripped) and copy it beside package.json.
//
//   npm run build:wasm
//
// Needs a Rust toolchain with the wasm32-wasip1 standard library
// (`rustup target add wasm32-wasip1`). CARGO and RUSTC name the toolchain's
// binaries when the ones on PATH lack it; CARGO_TARGET_DIR is passed on.

import { spawnSync } from 'node:child_process';
import { copyFileSync, statSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const packageDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repo = resolve(packageDir, '../..');
const cargo = process.env.CARGO ?? 'cargo';
const ran = spawnSync(cargo, ['build', '--profile', 'wasm', '-p', 'php-cli', '--bin', 'p8', '--target', 'wasm32-wasip1'], { cwd: repo, stdio: 'inherit' });
if (ran.status !== 0) process.exit(ran.status ?? 1);
const target = process.env.CARGO_TARGET_DIR ? resolve(process.env.CARGO_TARGET_DIR) : join(repo, 'target');
const built = join(target, 'wasm32-wasip1/wasm/p8.wasm');
copyFileSync(built, join(packageDir, 'p8.wasm'));
console.log(`p8.wasm: ${statSync(built).size} bytes`);
