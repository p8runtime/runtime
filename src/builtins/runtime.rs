//! Information about the running request and engine: version and SAPI,
//! backtraces, included files, defined variables, memory and the cycle
//! collector's controls, settype() and get_debug_type().

use crate::core::value::{ArrayData, ArrayKey, Handle, Val};
use crate::vm::engine::VM;
use std::rc::Rc;

fn string(vm: &mut VM, s: &[u8]) -> Handle {
    vm.arena.alloc(Val::String(Rc::new(s.to_vec())))
}

fn int_arg(vm: &VM, args: &[Handle], i: usize, default: i64) -> i64 {
    args.get(i).map_or(default, |&h| vm.arena.get(h).value.to_int())
}

fn list(vm: &mut VM, items: Vec<Vec<u8>>) -> Handle {
    let mut array = ArrayData::new();
    for item in items {
        let h = vm.arena.alloc(Val::String(Rc::new(item)));
        array.push(h);
    }
    vm.arena.alloc(Val::Array(Rc::new(array)))
}

/// phpversion(?string $extension = null): string|false
pub fn php_phpversion(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let extension = args
        .first()
        .map(|&h| vm.arena.get(h).value.clone())
        .filter(|v| !matches!(v, Val::Null));
    match extension {
        None => Ok(string(vm, b"8.4.27")),
        Some(name) => {
            let name = String::from_utf8_lossy(&name.to_php_string_bytes()).into_owned();
            // Bundled extensions report PHP's own version.
            if vm.context.engine.registry.extension_loaded(&name) {
                Ok(string(vm, b"8.4.27"))
            } else {
                Ok(vm.arena.alloc(Val::Bool(false)))
            }
        }
    }
}

/// php_sapi_name(): string|false
pub fn php_php_sapi_name(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(string(vm, b"cli"))
}

/// zend_version(): string
pub fn php_zend_version(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(string(vm, b"4.4.27"))
}

/// getmypid(): int|false
pub fn php_getmypid(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(vm.arena.alloc(Val::Int(std::process::id() as i64)))
}

/// get_debug_type(mixed $value): string
pub fn php_get_debug_type(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let value = args.first().map(|&h| vm.arena.get(h).value.clone()).unwrap_or(Val::Null);
    let name: Vec<u8> = match &value {
        Val::Null => b"null".to_vec(),
        Val::Bool(_) => b"bool".to_vec(),
        Val::Int(_) => b"int".to_vec(),
        Val::Float(_) => b"float".to_vec(),
        Val::String(_) => b"string".to_vec(),
        Val::Array(_) | Val::ConstArray(_) => b"array".to_vec(),
        Val::Object(payload) => match &vm.arena.get(*payload).value {
            Val::ObjPayload(obj) => {
                let class = vm.context.interner.lookup(obj.class).unwrap_or(b"");
                // An anonymous class is "Parent@anonymous" or "class@anonymous"
                match class.iter().position(|&b| b == 0) {
                    Some(nul) => class[..nul].to_vec(),
                    None => class.to_vec(),
                }
            }
            _ => b"object".to_vec(),
        },
        Val::Resource(_) => b"resource (stream)".to_vec(),
        _ => b"null".to_vec(),
    };
    Ok(string(vm, &name))
}

/// debug_backtrace(int $options = DEBUG_BACKTRACE_PROVIDE_OBJECT, int $limit = 0): array
pub fn php_debug_backtrace(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let options = int_arg(vm, args, 0, 1);
    let limit = int_arg(vm, args, 1, 0).max(0) as usize;
    Ok(vm.capture_backtrace_with(options & 1 != 0, options & 2 != 0, limit))
}

/// debug_print_backtrace(int $options = 0, int $limit = 0): void
pub fn php_debug_print_backtrace(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let options = int_arg(vm, args, 0, 0);
    let limit = int_arg(vm, args, 1, 0).max(0) as usize;
    let trace = vm.capture_backtrace_with(false, options & 2 != 0, limit);
    let frames: Vec<Handle> = match &vm.arena.get(trace).value {
        Val::Array(a) => a.map.values().copied().collect(),
        _ => Vec::new(),
    };
    let text = crate::builtins::exception::trace_lines(vm, &frames);
    vm.write_output(text.as_bytes()).map_err(|e| format!("{:?}", e))?;
    Ok(vm.arena.alloc(Val::Null))
}

/// get_included_files(): array — the main script, then each included file.
pub fn php_get_included_files(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let mut files: Vec<Vec<u8>> = Vec::new();
    if let Some(main) = vm.frames.first().and_then(|f| f.chunk.file_path.clone())
        && std::path::Path::new(&main).is_file()
    {
        files.push(main.into_bytes());
    }
    for file in &vm.context.included_files {
        if !files.iter().any(|f| f == file.as_bytes()) {
            files.push(file.as_bytes().to_vec());
        }
    }
    Ok(list(vm, files))
}

/// get_defined_vars(): array — the calling scope's variables, by value.
pub fn php_get_defined_vars(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let locals: Vec<(Vec<u8>, Handle)> = vm
        .frames
        .last()
        .map(|f| {
            f.locals
                .iter()
                .map(|(sym, h)| (vm.context.interner.lookup(*sym).unwrap_or(b"").to_vec(), *h))
                .collect()
        })
        .unwrap_or_default();
    let mut array = ArrayData::new();
    for (name, handle) in locals {
        let value = vm.arena.get(handle).value.clone();
        if matches!(value, Val::Uninitialized) {
            continue;
        }
        let copy = vm.arena.alloc(value);
        array.insert(ArrayKey::Str(Rc::new(name)), copy);
    }
    Ok(vm.arena.alloc(Val::Array(Rc::new(array))))
}

/// get_include_path(): string|false
pub fn php_get_include_path(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let path = vm.context.ini.get("include_path").unwrap_or_default();
    Ok(vm.arena.alloc(Val::String(Rc::new(path))))
}

/// set_include_path(string $include_path): string|false — the old path.
pub fn php_set_include_path(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let path = args
        .first()
        .map(|&h| vm.arena.get(h).value.to_php_string_bytes())
        .unwrap_or_default();
    if path.is_empty() {
        return Ok(vm.arena.alloc(Val::Bool(false)));
    }
    Ok(match crate::builtins::errors::set_ini(vm, "include_path", path) {
        Some(old) => vm.arena.alloc(Val::String(Rc::new(old))),
        None => vm.arena.alloc(Val::Bool(false)),
    })
}

/// memory_get_usage(bool $real_usage = false): int; memory_get_peak_usage
/// reports the same estimate.
pub fn php_memory_get_usage(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    // The runtime's own baseline is about what PHP reports for an empty script.
    let usage = 2 * 1024 * 1024 + vm.get_memory_usage();
    Ok(vm.arena.alloc(Val::Int(usage as i64)))
}

/// memory_reset_peak_usage(): void
pub fn php_memory_reset_peak_usage(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(vm.arena.alloc(Val::Null))
}

/// gc_collect_cycles(): int. The heap is traced, not reference counted, so
/// there are no leaked cycles to count; garbage is reclaimed between
/// statements.
pub fn php_gc_collect_cycles(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(vm.arena.alloc(Val::Int(0)))
}

/// gc_enabled(): bool — the zend.enable_gc directive.
pub fn php_gc_enabled(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let on = vm.context.ini.is_on("zend.enable_gc");
    Ok(vm.arena.alloc(Val::Bool(on)))
}

/// gc_enable(): void
pub fn php_gc_enable(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    crate::builtins::errors::set_ini(vm, "zend.enable_gc", b"1".to_vec());
    Ok(vm.arena.alloc(Val::Null))
}

/// gc_disable(): void
pub fn php_gc_disable(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    crate::builtins::errors::set_ini(vm, "zend.enable_gc", b"0".to_vec());
    Ok(vm.arena.alloc(Val::Null))
}

/// gc_status(): array, with PHP's keys.
pub fn php_gc_status(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let entries: [(&[u8], Val); 12] = [
        (b"running", Val::Bool(false)),
        (b"protected", Val::Bool(false)),
        (b"full", Val::Bool(false)),
        (b"runs", Val::Int(0)),
        (b"collected", Val::Int(0)),
        (b"threshold", Val::Int(10001)),
        (b"buffer_size", Val::Int(16384)),
        (b"roots", Val::Int(0)),
        (b"application_time", Val::Float(0.0)),
        (b"collector_time", Val::Float(0.0)),
        (b"destructor_time", Val::Float(0.0)),
        (b"free_time", Val::Float(0.0)),
    ];
    let mut array = ArrayData::new();
    for (key, value) in entries {
        let h = vm.arena.alloc(value);
        array.insert(ArrayKey::Str(Rc::new(key.to_vec())), h);
    }
    Ok(vm.arena.alloc(Val::Array(Rc::new(array))))
}

/// clearstatcache(bool $clear_realpath_cache = false, string $filename = ""): void.
/// Nothing is cached: every stat reads the filesystem.
pub fn php_clearstatcache(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(vm.arena.alloc(Val::Null))
}

/// get_resource_type(resource $resource): string
pub fn php_get_resource_type(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let is_resource = args
        .first()
        .is_some_and(|&h| matches!(vm.arena.get(h).value, Val::Resource(_)));
    if !is_resource {
        let given = args
            .first()
            .map(|&h| crate::builtins::signature::value_name(vm, h))
            .unwrap_or_else(|| "null".into());
        return Err(vm.throw_from_native(
            b"TypeError",
            &format!("get_resource_type(): Argument #1 ($resource) must be of type resource, {} given", given),
        ));
    }
    Ok(string(vm, b"stream"))
}

/// settype(mixed &$var, string $type): bool
pub fn php_settype(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let (Some(&var), Some(&ty)) = (args.first(), args.get(1)) else {
        return Err(vm.throw_from_native(b"ArgumentCountError", "settype() expects exactly 2 arguments"));
    };
    let ty = vm.arena.get(ty).value.to_php_string_bytes().to_ascii_lowercase();
    let kind = match ty.as_slice() {
        b"integer" | b"int" => 0,
        b"boolean" | b"bool" => 1,
        b"float" | b"double" => 2,
        b"string" => 3,
        b"array" => 4,
        b"object" => 5,
        b"null" => 6,
        b"resource" => {
            return Err(vm.throw_from_native(b"ValueError", "Cannot convert to resource type"));
        }
        _ => {
            return Err(vm.throw_from_native(
                b"ValueError",
                "settype(): Argument #2 ($type) must be a valid type",
            ));
        }
    };
    let converted = vm.cast_value(kind, var).map_err(|e| format!("{:?}", e))?;
    let value = vm.arena.get(converted).value.clone();
    vm.arena.get_mut(var).value = value;
    Ok(vm.arena.alloc(Val::Bool(true)))
}

/// spl_autoload_functions(): array
pub fn php_spl_autoload_functions(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let mut array = ArrayData::new();
    for &h in &vm.context.autoloaders.clone() {
        array.push(h);
    }
    Ok(vm.arena.alloc(Val::Array(Rc::new(array))))
}

/// preg_last_error(): int — matching never fails partway in this engine.
pub fn php_preg_last_error(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(vm.arena.alloc(Val::Int(0)))
}

/// preg_last_error_msg(): string
pub fn php_preg_last_error_msg(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(string(vm, b"No error"))
}
