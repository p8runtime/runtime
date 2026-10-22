//! Error handling functions: set_error_handler(), trigger_error(),
//! set_exception_handler(), error_clear_last(), and the ini functions.
//! Reference: $PHP_SRC_PATH/Zend/zend_builtin_functions.c, main/main.c

use crate::core::value::{Handle, Val};
use crate::vm::engine::{ErrorLevel, VM};
use std::rc::Rc;

const E_ALL: i64 = 32767;

fn is_callable_or_null(vm: &VM, h: Handle) -> bool {
    !matches!(
        vm.arena.get(h).value,
        Val::Int(_) | Val::Float(_) | Val::Bool(_) | Val::Resource(_)
    )
}

/// set_error_handler(?callable $callback, int $error_levels = E_ALL): ?callable
pub fn php_set_error_handler(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let Some(&callback) = args.first() else {
        return Err(vm.throw_from_native(
            b"ArgumentCountError",
            "set_error_handler() expects at least 1 argument, 0 given",
        ));
    };
    if !is_callable_or_null(vm, callback) {
        let msg = "set_error_handler(): Argument #1 ($callback) must be a valid callback or null";
        return Err(vm.throw_from_native(b"TypeError", msg));
    }
    let mask = args.get(1).map_or(E_ALL, |h| vm.arena.get(*h).value.to_int());
    let previous = vm.user_error_handlers.last().copied().flatten().map(|(h, _)| h);
    let entry = match vm.arena.get(callback).value {
        Val::Null => None,
        _ => Some((callback, mask)),
    };
    vm.user_error_handlers.push(entry);
    Ok(previous.unwrap_or_else(|| vm.arena.alloc(Val::Null)))
}

/// restore_error_handler(): true
pub fn php_restore_error_handler(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    vm.user_error_handlers.pop();
    Ok(vm.arena.alloc(Val::Bool(true)))
}

/// set_exception_handler(?callable $callback): ?callable
pub fn php_set_exception_handler(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let Some(&callback) = args.first() else {
        return Err(vm.throw_from_native(
            b"ArgumentCountError",
            "set_exception_handler() expects exactly 1 argument, 0 given",
        ));
    };
    if !is_callable_or_null(vm, callback) {
        let msg = "set_exception_handler(): Argument #1 ($callback) must be a valid callback or null";
        return Err(vm.throw_from_native(b"TypeError", msg));
    }
    let previous = vm.user_exception_handlers.last().copied().flatten();
    let entry = match vm.arena.get(callback).value {
        Val::Null => None,
        _ => Some(callback),
    };
    vm.user_exception_handlers.push(entry);
    Ok(previous.unwrap_or_else(|| vm.arena.alloc(Val::Null)))
}

/// restore_exception_handler(): true
pub fn php_restore_exception_handler(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    vm.user_exception_handlers.pop();
    Ok(vm.arena.alloc(Val::Bool(true)))
}

/// trigger_error(string $message, int $error_level = E_USER_NOTICE): true
pub fn php_trigger_error(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let Some(&message) = args.first() else {
        return Err(vm.throw_from_native(
            b"ArgumentCountError",
            "trigger_error() expects at least 1 argument, 0 given",
        ));
    };
    let message = String::from_utf8_lossy(&vm.arena.get(message).value.to_php_string_bytes()).into_owned();
    let level = args.get(1).map_or(1024, |h| vm.arena.get(*h).value.to_int());
    let level = match level {
        256 | 512 | 1024 | 16384 => ErrorLevel::from_bitmask(level),
        _ => None,
    };
    let Some(level) = level else {
        let msg = "trigger_error(): Argument #2 ($error_level) must be one of E_USER_ERROR, E_USER_WARNING, E_USER_NOTICE, or E_USER_DEPRECATED";
        return Err(vm.throw_from_native(b"ValueError", msg));
    };
    if level == ErrorLevel::UserError {
        vm.report_error(
            ErrorLevel::Deprecated,
            "Passing E_USER_ERROR to trigger_error() is deprecated since 8.4, throw an exception or call exit with a string message instead",
        );
    }
    vm.report_error(level, &message);
    if level == ErrorLevel::UserError && vm.pending_throw.is_none() && !vm.error_was_handled() {
        // E_USER_ERROR not handled by a user handler ends the script.
        return Err(vm.terminate());
    }
    Ok(vm.arena.alloc(Val::Bool(true)))
}

/// error_clear_last(): void
pub fn php_error_clear_last(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    vm.context.last_error = None;
    Ok(vm.arena.alloc(Val::Null))
}

fn ini_name(vm: &VM, h: Handle) -> String {
    String::from_utf8_lossy(&vm.arena.get(h).value.to_php_string_bytes()).into_owned()
}

/// Change a directive and whatever reads it; returns the old value, or None
/// for a directive that does not exist. ini_set(), ini_restore() and the
/// `-d` options all go through here.
pub fn set_ini(vm: &mut VM, name: &str, value: Vec<u8>) -> Option<Vec<u8>> {
    if name == "error_reporting" {
        let old = vm.context.config.error_reporting.to_string().into_bytes();
        vm.context.config.error_reporting = Val::String(Rc::new(value)).to_int() as u32;
        return Some(old);
    }
    let old = vm.context.ini.set(name, value.clone())?;
    match name {
        "precision" | "serialize_precision" => {
            crate::core::value::FloatPrecision::set_ini(name, &value);
        }
        "date.timezone" if !value.is_empty() => {
            vm.context.config.timezone = String::from_utf8_lossy(&value).into_owned();
        }
        _ => {}
    }
    Some(old)
}

/// Apply `-d name=value` options before a script runs: values are read as
/// php.ini reads them, unknown directives are ignored.
pub fn apply_startup_ini(vm: &mut VM, settings: &[(String, String)]) {
    for (name, raw) in settings {
        let value = crate::runtime::ini::parse_value(raw, |constant| {
            let sym = vm.context.interner.find(constant.as_bytes())?;
            vm.context.constants.get(&sym).map(Val::to_php_string_bytes)
        });
        set_ini(vm, name.trim(), value);
    }
}

/// ini_get(string $option): string|false
pub fn php_ini_get(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let Some(&name) = args.first() else {
        return Err("ini_get() expects exactly 1 argument, 0 given".into());
    };
    let name = ini_name(vm, name);
    let value = if name == "error_reporting" {
        Some(vm.context.config.error_reporting.to_string().into_bytes())
    } else {
        vm.context.ini.get(&name)
    };
    Ok(match value {
        Some(v) => vm.arena.alloc(Val::String(Rc::new(v))),
        None => vm.arena.alloc(Val::Bool(false)),
    })
}

/// ini_set(string $option, string|int|float|bool|null $value): string|false
pub fn php_ini_set(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("ini_set() expects exactly 2 arguments".into());
    }
    let name = ini_name(vm, args[0]);
    let value = match vm.arena.get(args[1]).value.clone() {
        Val::Bool(b) => if b { b"1".to_vec() } else { Vec::new() },
        Val::Null => Vec::new(),
        other => other.to_php_string_bytes(),
    };
    Ok(match set_ini(vm, &name, value) {
        Some(old) => vm.arena.alloc(Val::String(Rc::new(old))),
        None => vm.arena.alloc(Val::Bool(false)),
    })
}

/// ini_restore(string $option): void
pub fn php_ini_restore(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if let Some(&name) = args.first() {
        let name = ini_name(vm, name);
        vm.context.ini.restore(&name);
        if name == "error_reporting" {
            vm.context.config.error_reporting = E_ALL as u32;
        } else if let Some(value) = vm.context.ini.get(&name) {
            set_ini(vm, &name, value);
            vm.context.ini.restore(&name);
        }
    }
    Ok(vm.arena.alloc(Val::Null))
}

/// register_shutdown_function(callable $callback, mixed ...$args): void
pub fn php_register_shutdown_function(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let Some(&callback) = args.first() else {
        return Err(vm.throw_from_native(
            b"ArgumentCountError",
            "register_shutdown_function() expects at least 1 argument, 0 given",
        ));
    };
    vm.shutdown_functions.push((callback, args[1..].to_vec()));
    Ok(vm.arena.alloc(Val::Null))
}

/// error_log(string $message, int $message_type = 0, ?string $destination = null): bool
///
/// Type 0 goes to the error_log ini file if set, else stderr (the CLI's
/// default); type 3 appends to the destination file.
pub fn php_error_log(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    use std::io::Write;
    let Some(&message) = args.first() else {
        return Err("error_log() expects at least 1 argument, 0 given".into());
    };
    let mut message = vm.arena.get(message).value.to_php_string_bytes();
    let kind = args.get(1).map_or(0, |h| vm.arena.get(*h).value.to_int());
    let destination = args
        .get(2)
        .map(|h| vm.arena.get(*h).value.to_php_string_bytes())
        .filter(|d| !d.is_empty());
    let target = match kind {
        3 => destination,
        _ => vm.context.ini.get("error_log").filter(|d| !d.is_empty()),
    };
    let ok = match target {
        Some(path) => {
            if kind != 3 {
                message.push(b'\n');
            }
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(String::from_utf8_lossy(&path).as_ref())
                .and_then(|mut f| f.write_all(&message))
                .is_ok()
        }
        None => {
            message.push(b'\n');
            std::io::stderr().write_all(&message).is_ok()
        }
    };
    Ok(vm.arena.alloc(Val::Bool(ok)))
}
