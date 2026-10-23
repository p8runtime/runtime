use crate::core::value::{Handle, Val};
use crate::vm::engine::VM;
use std::rc::Rc;

/// spl_autoload_register() - Register a function for autoloading classes
pub fn php_spl_autoload_register(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        // Matching native behavior: registering the default autoloader succeeds
        return Ok(vm.arena.alloc(Val::Bool(true)));
    }

    let callback_handle = args[0];
    let callback_val = vm.arena.get(callback_handle);

    // Optional: throw argument (defaults to true)
    let throw_on_failure = args
        .get(1)
        .and_then(|handle| match vm.arena.get(*handle).value {
            Val::Bool(b) => Some(b),
            _ => None,
        })
        .unwrap_or(true);

    // Optional: prepend argument (defaults to false)
    let prepend = args
        .get(2)
        .and_then(|handle| match vm.arena.get(*handle).value {
            Val::Bool(b) => Some(b),
            _ => None,
        })
        .unwrap_or(false);

    let is_valid_callback = match &callback_val.value {
        Val::Null => false,
        Val::String(_) | Val::Array(_) | Val::Object(_) => true,
        _ => false,
    };

    if !is_valid_callback {
        if throw_on_failure {
            return Err(
                "spl_autoload_register(): Argument #1 must be a valid callback".to_string(),
            );
        } else {
            return Ok(vm.arena.alloc(Val::Bool(false)));
        }
    }

    // Avoid duplicate registrations of the same handle
    let already_registered = vm
        .context
        .autoloaders
        .iter()
        .any(|existing| existing == &callback_handle);

    if !already_registered {
        if prepend {
            vm.context.autoloaders.insert(0, callback_handle);
        } else {
            vm.context.autoloaders.push(callback_handle);
        }
    }

    Ok(vm.arena.alloc(Val::Bool(true)))
}

/// spl_object_id() - the object's id, as var_dump shows it after `#`
pub fn php_spl_object_id(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let Some(&target) = args.first() else {
        return Err("spl_object_id() expects exactly 1 argument, 0 given".to_string());
    };
    match vm.arena.get(target).value {
        Val::Object(payload) => {
            let id = match &vm.arena.get(payload).value {
                Val::ObjPayload(data) => data.id,
                _ => 0,
            };
            Ok(vm.arena.alloc(Val::Int(i64::from(id))))
        }
        ref other => {
            let msg = format!(
                "spl_object_id(): Argument #1 ($object) must be of type object, {} given",
                other.type_name()
            );
            Err(vm.throw_from_native(b"TypeError", &msg))
        }
    }
}

/// spl_object_hash() - Retrieve a unique identifier for an object
pub fn php_spl_object_hash(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("spl_object_hash() expects at least 1 parameter".to_string());
    }

    let target_handle = args[0];
    let target_val = vm.arena.get(target_handle);

    let object_handle = match &target_val.value {
        Val::Object(payload_handle) => *payload_handle,
        _ => {
            return Err(format!(
                "spl_object_hash() expects parameter 1 to be object, {} given",
                target_val.value.type_name()
            ));
        }
    };

    // PHP: the object id as 16 hex digits, then 16 zeros
    let id = match &vm.arena.get(object_handle).value {
        Val::ObjPayload(data) => data.id,
        _ => 0,
    };
    let hash = format!("{:016x}{:016x}", id, 0);
    let hash_bytes = Rc::new(hash.into_bytes());
    Ok(vm.arena.alloc(Val::String(hash_bytes)))
}

/// iterator_to_array(Traversable|array $iterator, bool $preserve_keys = true): array
/// Reference: $PHP_SRC_PATH/ext/spl/spl_iterators.c
pub fn php_iterator_to_array(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let preserve_keys = args.get(1).is_none_or(|&h| vm.arena.get(h).value.to_bool());
    let entries = vm
        .traversable_entries(args[0])
        .map_err(|e| vm.callback_error(e))?;
    let mut out = crate::core::value::ArrayData::new();
    for (key, value) in entries {
        let value = vm.arena.alloc(vm.arena.get(value).value.clone());
        if preserve_keys {
            let key = vm
                .array_key_from_value(&vm.arena.get(key).value.clone())
                .map_err(|e| vm.callback_error(e))?;
            out.insert(key, value);
        } else {
            out.push(value);
        }
    }
    Ok(vm.arena.alloc(Val::Array(Rc::new(out))))
}

/// iterator_count(Traversable|array $iterator): int
pub fn php_iterator_count(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let entries = vm
        .traversable_entries(args[0])
        .map_err(|e| vm.callback_error(e))?;
    Ok(vm.arena.alloc(Val::Int(entries.len() as i64)))
}

/// iterator_apply(Traversable $iterator, callable $callback, ?array $args = null): int
/// Calls the callback once per element until it returns something falsy.
pub fn php_iterator_apply(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let call_args: Vec<Handle> = match args.get(2).map(|&h| vm.arena.get(h).value.clone()) {
        Some(Val::Array(arr)) => arr.map.values().copied().collect(),
        _ => Vec::new(),
    };
    let entries = vm
        .traversable_entries(args[0])
        .map_err(|e| vm.callback_error(e))?;
    let mut count = 0;
    for _ in entries {
        count += 1;
        let result = vm
            .call_callable(args[1], call_args.iter().copied().collect())
            .map_err(|e| vm.callback_error(e))?;
        if !vm.arena.get(result).value.to_bool() {
            break;
        }
    }
    Ok(vm.arena.alloc(Val::Int(count)))
}
