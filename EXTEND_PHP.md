# Extending the PHP Runtime

When you need to expose a new PHP builtin or adjust the interpreter, keep this checklist handy:

1. **Pick or add a builtin module**
   - Each PHP family lives under `src/builtins/` (e.g., `string.rs`, `filesystem.rs`, `math.rs`). Create a new file if no existing module matches the namespace you want to extend.
   - Follow the existing helper patterns—for example, `vm.check_builtin_param_string(...)`, `Val::to_int()`, or `vm.arena.alloc()`—so input/output handling stays consistent.

2. **Implement the handler**
   - Builtins take `&mut VM` and `&[Handle]`, and return `Result<Handle, String>`.
   - Use `vm.arena.get`/`get_mut` and `vm.arena.alloc` to work with arguments/returns and to avoid leaking GC handles.
   - Keep logic small and thread-safe so it works in both native and Wasm builds; avoid relying on blocking OS I/O unless gated.

3. **Register the symbol**
   - Register functions via `registry.register_function(b"your_name", module::php_your_name);` inside `src/runtime/core_extension.rs`.
   - For by-ref parameters, use `register_function_with_by_ref`. For class-level helpers, use the appropriate extension (e.g., `date_extension.rs`, `core_extension`, future modules).
   - Update any helper tables (e.g., `src/runtime/context.rs`) if you add new classes, constants, or auto-loaded resources.

4. **Add runnable PHP coverage**
   - Drop a script into `tests/<category>/` (e.g., `tests/math/pi.php`) that mirrors the PHP.net example you’re targeting.
   - Add the category/description to `tests/README.md` so future contributors know what each folder is covering.
  - Use `bun tests/php/testrunner.js <category>` (from the repo root) to compare `php` vs `php-router`, confirm outputs match, and catch regressions early.

5. **Build & verify**
   - `cargo check --bin php` catches compile-time mistakes; `cargo build --bin php --release` ensures release optimizations still compile.
  - Run the router (`cargo run --bin php-router`) or `bun tests/php/testrunner.js` to exercise the new builtin through the HTTP server or direct CLI, depending on what you changed.

By following this routine you expand the runtime in a way that keeps the interpreter, router, and manual-test suites aligned with the official PHP behavior.
