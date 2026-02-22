# php-rs

Core PHP/PHPX runtime crate used by the Deka MVP workspace.

## Scope in MVP

- Provides parser, VM, builtins, runtime context, and PHPX-facing internals.
- Consumed by workspace crates like `modules_php` and `cli`.
- Maintained as a runtime library first.

## Build

```sh
cargo build --release -p php-rs
```

## Testing

```sh
cargo test -p php-rs --release
```

## Notes

- Legacy benchmark and old integration payloads were removed from this crate to keep MVP focused.
- Default builds do not emit legacy CLI/FPM binaries.
