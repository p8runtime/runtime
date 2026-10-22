//! Zend OPcache's PHP-visible surface. This runtime compiles every file
//! afresh and keeps no shared opcode cache, so the functions report the
//! state a running OPcache would show for a request: enabled when
//! `opcache.enable` (and, on the CLI, `opcache.enable_cli`) is on, the
//! request's compiled files as its cached scripts, nothing evicted.

use crate::core::value::{ArrayData, ArrayKey, Handle, Val};
use crate::vm::engine::VM;
use std::rc::Rc;

/// opcache.enable and opcache.enable_cli, as PHP's CLI requires both.
fn enabled(vm: &VM) -> bool {
    vm.context.ini.is_on("opcache.enable") && vm.context.ini.is_on("opcache.enable_cli")
}

fn assoc(vm: &mut VM, entries: Vec<(&str, Val)>) -> Handle {
    let mut array = ArrayData::new();
    for (key, value) in entries {
        let handle = vm.arena.alloc(value);
        array.insert(ArrayKey::Str(Rc::new(key.as_bytes().to_vec())), handle);
    }
    vm.arena.alloc(Val::Array(Rc::new(array)))
}

fn array_val(vm: &mut VM, entries: Vec<(&str, Val)>) -> Val {
    let handle = assoc(vm, entries);
    vm.arena.get(handle).value.clone()
}

/// The files compiled in this request: the main script and every include.
fn cached_scripts(vm: &VM) -> Vec<String> {
    let mut scripts: Vec<String> = vm.context.included_files.iter().cloned().collect();
    if let Some(main) = vm
        .frames
        .first()
        .and_then(|frame| frame.chunk.file_path.clone())
        && !scripts.contains(&main)
    {
        scripts.push(main);
    }
    scripts.sort();
    scripts
}

fn path_arg(vm: &VM, args: &[Handle]) -> String {
    args.first()
        .map(|&h| String::from_utf8_lossy(&vm.arena.get(h).value.to_php_string_bytes()).into_owned())
        .unwrap_or_default()
}

/// The path as the cache keys it: resolved like include does.
fn resolve(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string())
}

/// An ini size ("64M", "8K", "128"): zend_ini_parse_quantity.
fn parse_quantity(raw: &[u8]) -> i64 {
    let text = String::from_utf8_lossy(raw);
    let text = text.trim();
    let (digits, factor) = match text.chars().last().map(|c| c.to_ascii_lowercase()) {
        Some('k') => (&text[..text.len() - 1], 1 << 10),
        Some('m') => (&text[..text.len() - 1], 1 << 20),
        Some('g') => (&text[..text.len() - 1], 1 << 30),
        _ => (text, 1),
    };
    Val::String(Rc::new(digits.as_bytes().to_vec())).to_int().saturating_mul(factor)
}

/// opcache_get_status(bool $include_scripts = true): array|false
pub fn php_opcache_get_status(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if !enabled(vm) {
        return Ok(vm.arena.alloc(Val::Bool(false)));
    }
    let include_scripts = args.first().is_none_or(|&h| vm.arena.get(h).value.to_bool());
    let scripts = cached_scripts(vm);
    let memory = array_val(
        vm,
        vec![
            ("used_memory", Val::Int(0)),
            ("free_memory", Val::Int(128 * 1024 * 1024)),
            ("wasted_memory", Val::Int(0)),
            ("current_wasted_percentage", Val::Float(0.0)),
        ],
    );
    let interned = array_val(
        vm,
        vec![
            ("buffer_size", Val::Int(8 * 1024 * 1024)),
            ("used_memory", Val::Int(0)),
            ("free_memory", Val::Int(8 * 1024 * 1024)),
            ("number_of_strings", Val::Int(0)),
        ],
    );
    let statistics = array_val(
        vm,
        vec![
            ("num_cached_scripts", Val::Int(scripts.len() as i64)),
            ("num_cached_keys", Val::Int(scripts.len() as i64)),
            ("max_cached_keys", Val::Int(16229)),
            ("hits", Val::Int(0)),
            ("start_time", Val::Int(0)),
            ("last_restart_time", Val::Int(0)),
            ("oom_restarts", Val::Int(0)),
            ("hash_restarts", Val::Int(0)),
            ("manual_restarts", Val::Int(0)),
            ("misses", Val::Int(scripts.len() as i64)),
            ("blacklist_misses", Val::Int(0)),
            ("blacklist_miss_ratio", Val::Float(0.0)),
            ("opcache_hit_rate", Val::Float(0.0)),
        ],
    );
    let jit = array_val(
        vm,
        vec![
            ("enabled", Val::Bool(false)),
            ("on", Val::Bool(false)),
            ("kind", Val::Int(0)),
            ("opt_level", Val::Int(0)),
            ("opt_flags", Val::Int(0)),
            ("buffer_size", Val::Int(0)),
            ("buffer_free", Val::Int(0)),
        ],
    );
    let mut entries = vec![
        ("opcache_enabled", Val::Bool(true)),
        ("cache_full", Val::Bool(false)),
        ("restart_pending", Val::Bool(false)),
        ("restart_in_progress", Val::Bool(false)),
        ("memory_usage", memory),
        ("interned_strings_usage", interned),
        ("opcache_statistics", statistics),
    ];
    if include_scripts {
        let mut list = ArrayData::new();
        for script in &scripts {
            let entry = assoc(
                vm,
                vec![
                    ("full_path", Val::String(Rc::new(script.as_bytes().to_vec()))),
                    ("hits", Val::Int(0)),
                    ("memory_consumption", Val::Int(0)),
                    ("last_used", Val::String(Rc::new(Vec::new()))),
                    ("last_used_timestamp", Val::Int(0)),
                    ("timestamp", Val::Int(0)),
                    ("revalidate", Val::Int(0)),
                ],
            );
            list.insert(ArrayKey::Str(Rc::new(script.as_bytes().to_vec())), entry);
        }
        entries.push(("scripts", Val::Array(Rc::new(list))));
    }
    entries.push(("jit", jit));
    Ok(assoc(vm, entries))
}

/// opcache_get_configuration(): array|false
pub fn php_opcache_get_configuration(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let mut directives = ArrayData::new();
    for (name, kind) in crate::builtins::defaults::OPCACHE_DIRECTIVE_TYPES {
        let raw = vm.context.ini.get(name).unwrap_or_default();
        let text = Val::String(Rc::new(raw.clone()));
        let value = match kind {
            'b' => Val::Bool(vm.context.ini.is_on(name)),
            'i' => Val::Int(parse_quantity(&raw)),
            'f' => Val::Float(text.to_float()),
            _ => text,
        };
        let handle = vm.arena.alloc(value);
        directives.insert(ArrayKey::Str(Rc::new(name.as_bytes().to_vec())), handle);
    }
    let version = array_val(
        vm,
        vec![
            ("version", Val::String(Rc::new(b"8.4.27".to_vec()))),
            ("opcache_product_name", Val::String(Rc::new(b"Zend OPcache".to_vec()))),
        ],
    );
    let blacklist = Val::Array(Rc::new(ArrayData::new()));
    Ok(assoc(
        vm,
        vec![
            ("directives", Val::Array(Rc::new(directives))),
            ("version", version),
            ("blacklist", blacklist),
        ],
    ))
}

/// opcache_compile_file(string $filename): bool
pub fn php_opcache_compile_file(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let path = path_arg(vm, args);
    let readable = std::fs::metadata(&path).is_ok_and(|m| m.is_file());
    if !readable {
        vm.report_error(
            crate::vm::engine::ErrorLevel::Warning,
            &format!("Zend OPcache could not compile file {}", path),
        );
    } else if enabled(vm) {
        vm.context.included_files.insert(resolve(&path));
    }
    Ok(vm.arena.alloc(Val::Bool(readable)))
}

/// opcache_is_script_cached(string $filename): bool
pub fn php_opcache_is_script_cached(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let path = resolve(&path_arg(vm, args));
    let cached = enabled(vm) && cached_scripts(vm).contains(&path);
    Ok(vm.arena.alloc(Val::Bool(cached)))
}

/// opcache_invalidate(string $filename, bool $force = false): bool
pub fn php_opcache_invalidate(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let path = resolve(&path_arg(vm, args));
    let cached = enabled(vm) && cached_scripts(vm).contains(&path);
    Ok(vm.arena.alloc(Val::Bool(cached)))
}

/// opcache_reset(): bool
pub fn php_opcache_reset(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let on = enabled(vm);
    Ok(vm.arena.alloc(Val::Bool(on)))
}

/// opcache_jit_blacklist(Closure $closure): void
pub fn php_opcache_jit_blacklist(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(vm.arena.alloc(Val::Null))
}
