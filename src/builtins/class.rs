use crate::core::value::{ArrayKey, Handle, Val};
use crate::runtime::context::EnumBackedType;
use crate::vm::engine::{ErrorLevel, PropertyCollectionMode, VM};
use indexmap::IndexMap;
use std::rc::Rc;

//=============================================================================
// Predefined Interface & Class Implementations
// Reference: $PHP_SRC_PATH/Zend/zend_interfaces.c
//=============================================================================

// Iterator interface methods (SPL)
// Reference: $PHP_SRC_PATH/Zend/zend_interfaces.c - zend_user_iterator
pub fn iterator_current(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let _this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("Iterator::current() called outside object context")?;

    // Default implementation returns null if not overridden
    Ok(vm.arena.alloc(Val::Null))
}

pub fn iterator_key(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let _this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("Iterator::key() called outside object context")?;

    Ok(vm.arena.alloc(Val::Null))
}

pub fn iterator_next(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(vm.arena.alloc(Val::Null))
}

pub fn iterator_rewind(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(vm.arena.alloc(Val::Null))
}

pub fn iterator_valid(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(vm.arena.alloc(Val::Bool(false)))
}

// IteratorAggregate interface
pub fn iterator_aggregate_get_iterator(_vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Err("IteratorAggregate::getIterator() must be implemented".into())
}

// Countable interface
// Reference: $PHP_SRC_PATH/Zend/zend_interfaces.c - spl_countable
pub fn countable_count(_vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Err("Countable::count() must be implemented".into())
}

// ArrayAccess interface methods
// Reference: $PHP_SRC_PATH/Zend/zend_interfaces.c - zend_user_arrayaccess
pub fn array_access_offset_exists(_vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Err("ArrayAccess::offsetExists() must be implemented".into())
}

pub fn array_access_offset_get(_vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Err("ArrayAccess::offsetGet() must be implemented".into())
}

pub fn array_access_offset_set(_vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Err("ArrayAccess::offsetSet() must be implemented".into())
}

pub fn array_access_offset_unset(_vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Err("ArrayAccess::offsetUnset() must be implemented".into())
}

// Serializable interface (deprecated in PHP 8.1, but still supported)
pub fn serializable_serialize(_vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Err("Serializable::serialize() must be implemented".into())
}

pub fn serializable_unserialize(_vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Err("Serializable::unserialize() must be implemented".into())
}

// Closure class methods
// Reference: $PHP_SRC_PATH/Zend/zend_closures.c
pub fn closure_bind(_vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    // Closure::bind($closure, $newthis, $newscope = "static")
    // Returns a new closure with bound $this and/or class scope
    // For now, simplified implementation
    if args.is_empty() {
        return Err("Closure::bind() expects at least 1 parameter".into());
    }

    // Return the closure unchanged for now (full implementation would create new binding)
    Ok(args[0])
}

pub fn closure_bind_to(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    // $closure->bindTo($newthis, $newscope = "static")
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("Closure::bindTo() called outside object context")?;

    // Return this unchanged for now
    Ok(this_handle)
}

pub fn closure_call(_vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    // $closure->call($newThis, ...$args)
    Err("Closure::call() not yet fully implemented".into())
}

pub fn closure_from_callable(_vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    // Closure::fromCallable($callable)
    if args.is_empty() {
        return Err("Closure::fromCallable() expects exactly 1 parameter".into());
    }

    // Would convert callable to Closure
    Ok(args[0])
}

// stdClass - empty class, allows dynamic properties
// Reference: $PHP_SRC_PATH/Zend/zend_builtin_functions.c
// No methods needed - pure data container

// Generator class methods
// Reference: $PHP_SRC_PATH/Zend/zend_generators.c

/// `$this` of the Generator method being called.
fn this_generator(vm: &VM) -> Result<Handle, String> {
    vm.frames
        .last()
        .and_then(|f| f.this)
        .ok_or_else(|| "Generator method called outside object context".into())
}

pub fn generator_current(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let generator = this_generator(vm)?;
    vm.generator_current(generator).map_err(|e| vm.callback_error(e))
}

pub fn generator_key(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let generator = this_generator(vm)?;
    vm.generator_key(generator).map_err(|e| vm.callback_error(e))
}

pub fn generator_next(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let generator = this_generator(vm)?;
    vm.generator_next(generator).map_err(|e| vm.callback_error(e))?;
    Ok(vm.arena.alloc(Val::Null))
}

pub fn generator_rewind(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let generator = this_generator(vm)?;
    vm.generator_rewind(generator).map_err(|e| vm.callback_error(e))?;
    Ok(vm.arena.alloc(Val::Null))
}

pub fn generator_send(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let generator = this_generator(vm)?;
    let value = match args.first() {
        Some(&v) => v,
        None => vm.arena.alloc(Val::Null),
    };
    vm.generator_send(generator, value).map_err(|e| vm.callback_error(e))
}

pub fn generator_throw(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let generator = this_generator(vm)?;
    let exception = *args
        .first()
        .ok_or("Generator::throw() expects exactly 1 argument, 0 given")?;
    vm.generator_throw(generator, exception)
        .map_err(|e| vm.callback_error(e))
}

pub fn generator_valid(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let generator = this_generator(vm)?;
    let valid = vm.generator_valid(generator).map_err(|e| vm.callback_error(e))?;
    Ok(vm.arena.alloc(Val::Bool(valid)))
}

pub fn generator_get_return(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let generator = this_generator(vm)?;
    vm.generator_get_return(generator)
        .map_err(|e| vm.callback_error(e))
}

// Fiber class methods (PHP 8.1+)
// Reference: $PHP_SRC_PATH/Zend/zend_fibers.c
pub fn fiber_construct(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    // Fiber::__construct(callable $callback)
    if args.is_empty() {
        return Err("Fiber::__construct() expects exactly 1 parameter".into());
    }
    Ok(vm.arena.alloc(Val::Null))
}

pub fn fiber_start(_vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Err("Fiber::start() not yet implemented".into())
}

pub fn fiber_resume(_vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Err("Fiber::resume() not yet implemented".into())
}

pub fn fiber_suspend(_vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Err("Fiber::suspend() not yet implemented".into())
}

pub fn fiber_throw(_vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Err("Fiber::throw() not yet implemented".into())
}

pub fn fiber_is_started(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(vm.arena.alloc(Val::Bool(false)))
}

pub fn fiber_is_suspended(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(vm.arena.alloc(Val::Bool(false)))
}

pub fn fiber_is_running(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(vm.arena.alloc(Val::Bool(false)))
}

pub fn fiber_is_terminated(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(vm.arena.alloc(Val::Bool(false)))
}

pub fn fiber_get_return(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(vm.arena.alloc(Val::Null))
}

pub fn fiber_get_current(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(vm.arena.alloc(Val::Null))
}

// WeakReference class (PHP 7.4+)
// Reference: $PHP_SRC_PATH/Zend/zend_weakrefs.c
pub fn weak_reference_construct(_vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    // WeakReference::__construct() - private, use ::create() instead
    Err("WeakReference::__construct() is private".into())
}

pub fn weak_reference_create(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    // WeakReference::create(object $object): WeakReference
    if args.is_empty() {
        return Err("WeakReference::create() expects exactly 1 parameter".into());
    }

    let val = vm.arena.get(args[0]);
    if !matches!(val.value, Val::Object(_)) {
        return Err("WeakReference::create() expects parameter 1 to be object".into());
    }

    // Would create a WeakReference object
    Ok(args[0])
}

pub fn weak_reference_get(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    // Returns the referenced object or null if collected
    Ok(vm.arena.alloc(Val::Null))
}

// WeakMap class (PHP 8.0+)
// Reference: $PHP_SRC_PATH/Zend/zend_weakrefs.c
pub fn weak_map_construct(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(vm.arena.alloc(Val::Null))
}

pub fn weak_map_offset_exists(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(vm.arena.alloc(Val::Bool(false)))
}

pub fn weak_map_offset_get(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(vm.arena.alloc(Val::Null))
}

pub fn weak_map_offset_set(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(vm.arena.alloc(Val::Null))
}

pub fn weak_map_offset_unset(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(vm.arena.alloc(Val::Null))
}

pub fn weak_map_count(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(vm.arena.alloc(Val::Int(0)))
}

pub fn weak_map_get_iterator(_vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Err("WeakMap::getIterator() not yet implemented".into())
}

// Stringable interface (PHP 8.0+)
// Reference: $PHP_SRC_PATH/Zend/zend_interfaces.c
pub fn stringable_to_string(_vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Err("Stringable::__toString() must be implemented".into())
}

// UnitEnum interface (PHP 8.1+)
// Reference: $PHP_SRC_PATH/Zend/zend_enum.c
pub fn unit_enum_cases(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let class_sym = vm
        .current_called_scope()
        .ok_or("UnitEnum::cases() called outside class scope")?;
    let class_def = vm
        .context
        .classes
        .get(&class_sym)
        .ok_or("UnitEnum::cases(): Enum class not found")?;

    let mut array = crate::core::value::ArrayData::new();
    for (idx, case_def) in class_def.enum_cases.iter().enumerate() {
        array.insert(crate::core::value::ArrayKey::Int(idx as i64), case_def.handle);
    }

    Ok(vm.arena.alloc(Val::Array(array.into())))
}

// BackedEnum interface (PHP 8.1+)
pub fn backed_enum_from(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let result = resolve_backed_enum_case(vm, args, true)?;
    Ok(result)
}

pub fn backed_enum_try_from(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    match resolve_backed_enum_case(vm, args, false) {
        Ok(handle) => Ok(handle),
        Err(err) if err == "__try_from_value_error" => Ok(vm.arena.alloc(Val::Null)),
        Err(err) => Err(err),
    }
}

fn resolve_backed_enum_case(
    vm: &mut VM,
    args: &[Handle],
    strict_error: bool,
) -> Result<Handle, String> {
    let class_sym = vm
        .current_called_scope()
        .ok_or("BackedEnum::from() called outside class scope")?;
    let class_def = vm
        .context
        .classes
        .get(&class_sym)
        .ok_or("BackedEnum::from(): Enum class not found")?;
    let backed_type = class_def.enum_backed_type.ok_or(
        "BackedEnum::from() called on non-backed enum".to_string(),
    )?;

    if args.len() != 1 {
        return Err("BackedEnum::from() expects exactly 1 argument".into());
    }

    let arg_val = vm.arena.get(args[0]).value.clone();
    let matches_type = match (backed_type, &arg_val) {
        (EnumBackedType::Int, Val::Int(_)) => true,
        (EnumBackedType::String, Val::String(_)) => true,
        _ => false,
    };
    if !matches_type {
        return Err("BackedEnum::from() expects value of correct backing type".into());
    }

    for case_def in &class_def.enum_cases {
        if let Some(case_val) = &case_def.value {
            if values_equal(case_val, &arg_val) {
                return Ok(case_def.handle);
            }
        }
    }

    if strict_error {
        Err("BackedEnum::from(): Value not found in enum".into())
    } else {
        Err("__try_from_value_error".into())
    }
}

fn values_equal(a: &Val, b: &Val) -> bool {
    match (a, b) {
        (Val::Int(a), Val::Int(b)) => a == b,
        (Val::String(a), Val::String(b)) => a == b,
        _ => false,
    }
}

// SensitiveParameterValue class (PHP 8.2+)
// Reference: $PHP_SRC_PATH/Zend/zend_attributes.c
pub fn sensitive_parameter_value_construct(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    // SensitiveParameterValue::__construct($value)
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("SensitiveParameterValue::__construct() called outside object context")?;

    let value = if args.is_empty() {
        vm.arena.alloc(Val::Null)
    } else {
        args[0]
    };

    let value_sym = vm.context.interner.intern(b"value");

    if let Val::Object(payload_handle) = &vm.arena.get(this_handle).value {
        let payload = vm.arena.get_mut(*payload_handle);
        if let Val::ObjPayload(ref mut obj_data) = payload.value {
            obj_data.properties.insert(value_sym, value);
        }
    }

    Ok(vm.arena.alloc(Val::Null))
}

pub fn sensitive_parameter_value_get_value(
    vm: &mut VM,
    _args: &[Handle],
) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("SensitiveParameterValue::getValue() called outside object context")?;

    let value_sym = vm.context.interner.intern(b"value");

    if let Val::Object(payload_handle) = &vm.arena.get(this_handle).value {
        if let Val::ObjPayload(obj_data) = &vm.arena.get(*payload_handle).value {
            if let Some(&val_handle) = obj_data.properties.get(&value_sym) {
                return Ok(val_handle);
            }
        }
    }

    Ok(vm.arena.alloc(Val::Null))
}

pub fn sensitive_parameter_value_debug_info(
    vm: &mut VM,
    _args: &[Handle],
) -> Result<Handle, String> {
    // __debugInfo() returns array with redacted value
    let mut array = IndexMap::new();
    let key = ArrayKey::Str(Rc::new(b"value".to_vec()));
    let val = vm.arena.alloc(Val::String(Rc::new(b"[REDACTED]".to_vec())));
    array.insert(key, val);

    Ok(vm.arena.alloc(Val::Array(
        crate::core::value::ArrayData::from(array).into(),
    )))
}

// __PHP_Incomplete_Class - used during unserialization
// Reference: $PHP_SRC_PATH/ext/standard/incomplete_class.c
pub fn incomplete_class_construct(_vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    // Should not be instantiated directly
    Err("__PHP_Incomplete_Class cannot be instantiated".into())
}

//=============================================================================
// Existing class introspection functions
//=============================================================================

pub fn php_get_object_vars(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("get_object_vars() expects exactly 1 parameter".into());
    }

    let obj_handle = args[0];
    let obj_val = vm.arena.get(obj_handle);

    if let Val::Object(payload_handle) = &obj_val.value {
        let payload = vm.arena.get(*payload_handle);
        if let Val::ObjPayload(obj_data) = &payload.value {
            let mut result_map = IndexMap::new();
            let class_sym = obj_data.class;
            let current_scope = vm.get_current_class();

            let properties: Vec<(crate::core::value::Symbol, Handle)> =
                obj_data.properties.iter().map(|(k, v)| (*k, *v)).collect();

            for (prop_sym, val_handle) in properties {
                if vm
                    .check_prop_visibility(class_sym, prop_sym, current_scope)
                    .is_ok()
                {
                    let prop_name_bytes =
                        vm.context.interner.lookup(prop_sym).unwrap_or(b"").to_vec();
                    let key = ArrayKey::Str(Rc::new(prop_name_bytes));
                    result_map.insert(key, val_handle);
                }
            }

            return Ok(vm.arena.alloc(Val::Array(
                crate::core::value::ArrayData::from(result_map).into(),
            )));
        }
    }

    Err("get_object_vars() expects parameter 1 to be object".into())
}

pub fn php_get_class(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        if let Some(frame) = vm.frames.last() {
            if let Some(class_scope) = frame.class_scope {
                let name = vm
                    .context
                    .interner
                    .lookup(class_scope)
                    .unwrap_or(b"")
                    .to_vec();
                return Ok(vm.arena.alloc(Val::String(name.into())));
            }
        }
        return Err("get_class() called without object from outside a class".into());
    }

    let val = vm.arena.get(args[0]);
    if let Val::Object(h) = val.value {
        let obj_zval = vm.arena.get(h);
        if let Val::ObjPayload(obj_data) = &obj_zval.value {
            let class_name = vm
                .context
                .interner
                .lookup(obj_data.class)
                .unwrap_or(b"")
                .to_vec();
            return Ok(vm.arena.alloc(Val::String(class_name.into())));
        }
    }

    Err("get_class() called on non-object".into())
}

pub fn php_get_parent_class(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let class_name_sym = if args.is_empty() {
        if let Some(frame) = vm.frames.last() {
            if let Some(class_scope) = frame.class_scope {
                class_scope
            } else {
                return Ok(vm.arena.alloc(Val::Bool(false)));
            }
        } else {
            return Ok(vm.arena.alloc(Val::Bool(false)));
        }
    } else {
        let val = vm.arena.get(args[0]);
        match &val.value {
            Val::Object(h) => {
                let obj_zval = vm.arena.get(*h);
                if let Val::ObjPayload(obj_data) = &obj_zval.value {
                    obj_data.class
                } else {
                    return Ok(vm.arena.alloc(Val::Bool(false)));
                }
            }
            Val::String(s) => {
                if let Some(sym) = vm.context.interner.find(s) {
                    sym
                } else {
                    return Ok(vm.arena.alloc(Val::Bool(false)));
                }
            }
            _ => return Ok(vm.arena.alloc(Val::Bool(false))),
        }
    };

    if let Some(def) = vm.context.classes.get(&class_name_sym) {
        if let Some(parent_sym) = def.parent {
            let parent_name = vm
                .context
                .interner
                .lookup(parent_sym)
                .unwrap_or(b"")
                .to_vec();
            return Ok(vm.arena.alloc(Val::String(parent_name.into())));
        }
    }

    Ok(vm.arena.alloc(Val::Bool(false)))
}

pub fn php_is_subclass_of(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("is_subclass_of() expects at least 2 parameters".into());
    }

    let object_or_class = vm.arena.get(args[0]);
    let class_name_val = vm.arena.get(args[1]);

    let child_sym = match &object_or_class.value {
        Val::Object(h) => {
            let obj_zval = vm.arena.get(*h);
            if let Val::ObjPayload(obj_data) = &obj_zval.value {
                obj_data.class
            } else {
                return Ok(vm.arena.alloc(Val::Bool(false)));
            }
        }
        Val::String(s) => {
            if let Some(sym) = vm.context.interner.find(s) {
                sym
            } else {
                return Ok(vm.arena.alloc(Val::Bool(false)));
            }
        }
        _ => return Ok(vm.arena.alloc(Val::Bool(false))),
    };

    let parent_sym = match &class_name_val.value {
        Val::String(s) => {
            if let Some(sym) = vm.context.interner.find(s) {
                sym
            } else {
                return Ok(vm.arena.alloc(Val::Bool(false)));
            }
        }
        _ => return Ok(vm.arena.alloc(Val::Bool(false))),
    };

    if child_sym == parent_sym {
        return Ok(vm.arena.alloc(Val::Bool(false)));
    }

    let result = vm.is_subclass_of(child_sym, parent_sym);
    Ok(vm.arena.alloc(Val::Bool(result)))
}

pub fn php_is_a(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("is_a() expects at least 2 parameters".into());
    }

    let object_or_class = vm.arena.get(args[0]);
    let class_name_val = vm.arena.get(args[1]);

    let child_sym = match &object_or_class.value {
        Val::Object(h) => {
            let obj_zval = vm.arena.get(*h);
            if let Val::ObjPayload(obj_data) = &obj_zval.value {
                obj_data.class
            } else {
                return Ok(vm.arena.alloc(Val::Bool(false)));
            }
        }
        Val::String(s) => {
            if let Some(sym) = vm.context.interner.find(s) {
                sym
            } else {
                return Ok(vm.arena.alloc(Val::Bool(false)));
            }
        }
        _ => return Ok(vm.arena.alloc(Val::Bool(false))),
    };

    let parent_sym = match &class_name_val.value {
        Val::String(s) => {
            if let Some(sym) = vm.context.interner.find(s) {
                sym
            } else {
                return Ok(vm.arena.alloc(Val::Bool(false)));
            }
        }
        _ => return Ok(vm.arena.alloc(Val::Bool(false))),
    };

    if child_sym == parent_sym {
        return Ok(vm.arena.alloc(Val::Bool(true)));
    }

    let result = vm.is_subclass_of(child_sym, parent_sym);
    Ok(vm.arena.alloc(Val::Bool(result)))
}

/// The declared class, interface, trait or enum `args[0]` names (any case,
/// aliases too), autoloading it when `args[autoload_arg]` allows.
fn declared_class(vm: &mut VM, args: &[Handle], autoload_arg: usize) -> Option<crate::core::value::Symbol> {
    let Val::String(name) = vm.arena.get(*args.first()?).value.clone() else {
        return None;
    };
    let name = name.strip_prefix(b"\\").unwrap_or(&name).to_vec();
    let sym = vm.context.interner.intern(&name);
    let found = vm.canonical_class(sym);
    if vm.context.classes.contains_key(&found) {
        return Some(found);
    }
    let autoload = args.get(autoload_arg).is_none_or(|&h| vm.arena.get(h).value.to_bool());
    if autoload {
        let _ = vm.trigger_autoload(sym);
        let found = vm.canonical_class(sym);
        if vm.context.classes.contains_key(&found) {
            return Some(found);
        }
    }
    None
}

pub fn php_class_exists(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let exists = declared_class(vm, args, 1)
        .and_then(|c| vm.context.classes.get(&c))
        .is_some_and(|def| !def.is_interface && !def.is_trait);
    Ok(vm.arena.alloc(Val::Bool(exists)))
}

pub fn php_interface_exists(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let exists = declared_class(vm, args, 1)
        .and_then(|c| vm.context.classes.get(&c))
        .is_some_and(|def| def.is_interface);
    Ok(vm.arena.alloc(Val::Bool(exists)))
}

pub fn php_trait_exists(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let exists = declared_class(vm, args, 1)
        .and_then(|c| vm.context.classes.get(&c))
        .is_some_and(|def| def.is_trait);
    Ok(vm.arena.alloc(Val::Bool(exists)))
}

/// class_alias(string $class, string $alias, bool $autoload = true): bool
/// Reference: $PHP_SRC_PATH/Zend/zend_builtin_functions.c
pub fn php_class_alias(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let alias = vm.arena.get(args[1]).value.to_php_string_bytes();
    let alias = alias.strip_prefix(b"\\").unwrap_or(&alias).to_vec();
    let Some(class) = declared_class(vm, &[args[0], *args.get(2).unwrap_or(&args[0])], 1) else {
        let name = String::from_utf8_lossy(&vm.arena.get(args[0]).value.to_php_string_bytes()).into_owned();
        vm.report_error(ErrorLevel::Warning, &format!("Class \"{}\" not found", name));
        return Ok(vm.arena.alloc(Val::Bool(false)));
    };
    let alias_sym = vm.context.interner.intern(&alias);
    let taken = vm.context.classes.contains_key(&vm.canonical_class(alias_sym));
    if taken {
        let msg = format!(
            "Cannot declare class {}, because the name is already in use",
            String::from_utf8_lossy(&alias)
        );
        vm.report_error(ErrorLevel::Warning, &msg);
        return Ok(vm.arena.alloc(Val::Bool(false)));
    }
    vm.class_aliases.insert(alias.to_ascii_lowercase(), class);
    Ok(vm.arena.alloc(Val::Bool(true)))
}

pub fn php_method_exists(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("method_exists() expects exactly 2 parameters".into());
    }

    let object_or_class = vm.arena.get(args[0]);
    let method_name_val = vm.arena.get(args[1]);

    let class_sym = match &object_or_class.value {
        Val::Object(h) => {
            let obj_zval = vm.arena.get(*h);
            if let Val::ObjPayload(obj_data) = &obj_zval.value {
                obj_data.class
            } else {
                return Ok(vm.arena.alloc(Val::Bool(false)));
            }
        }
        Val::String(s) => {
            if let Some(sym) = vm.context.interner.find(s) {
                sym
            } else {
                return Ok(vm.arena.alloc(Val::Bool(false)));
            }
        }
        _ => return Ok(vm.arena.alloc(Val::Bool(false))),
    };

    let method_sym = match &method_name_val.value {
        Val::String(s) => {
            if let Some(sym) = vm.context.interner.find(s) {
                sym
            } else {
                return Ok(vm.arena.alloc(Val::Bool(false)));
            }
        }
        _ => return Ok(vm.arena.alloc(Val::Bool(false))),
    };

    let exists = vm.find_method(class_sym, method_sym).is_some();
    Ok(vm.arena.alloc(Val::Bool(exists)))
}

pub fn php_property_exists(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("property_exists() expects exactly 2 parameters".into());
    }

    let object_or_class = vm.arena.get(args[0]);
    let prop_name_val = vm.arena.get(args[1]);

    let prop_sym = match &prop_name_val.value {
        Val::String(s) => {
            if let Some(sym) = vm.context.interner.find(s) {
                sym
            } else {
                return Ok(vm.arena.alloc(Val::Bool(false)));
            }
        }
        _ => return Ok(vm.arena.alloc(Val::Bool(false))),
    };

    match &object_or_class.value {
        Val::Object(h) => {
            let obj_zval = vm.arena.get(*h);
            if let Val::ObjPayload(obj_data) = &obj_zval.value {
                // Check dynamic properties first
                if obj_data.properties.contains_key(&prop_sym) {
                    return Ok(vm.arena.alloc(Val::Bool(true)));
                }
                // Check class definition
                let exists = vm.has_property(obj_data.class, prop_sym);
                return Ok(vm.arena.alloc(Val::Bool(exists)));
            }
        }
        Val::String(s) => {
            if let Some(class_sym) = vm.context.interner.find(s) {
                let exists = vm.has_property(class_sym, prop_sym);
                return Ok(vm.arena.alloc(Val::Bool(exists)));
            }
        }
        _ => {}
    }

    Ok(vm.arena.alloc(Val::Bool(false)))
}

pub fn php_get_class_methods(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("get_class_methods() expects exactly 1 parameter".into());
    }

    let val = vm.arena.get(args[0]);
    let class_sym = match &val.value {
        Val::Object(h) => {
            let obj_zval = vm.arena.get(*h);
            if let Val::ObjPayload(obj_data) = &obj_zval.value {
                obj_data.class
            } else {
                return Ok(vm
                    .arena
                    .alloc(Val::Array(crate::core::value::ArrayData::new().into())));
            }
        }
        Val::String(s) => {
            if let Some(sym) = vm.context.interner.find(s) {
                sym
            } else {
                return Ok(vm.arena.alloc(Val::Null));
            }
        }
        _ => return Ok(vm.arena.alloc(Val::Null)),
    };

    let caller_scope = vm.get_current_class();
    let methods = vm.collect_methods(class_sym, caller_scope);
    let mut array = IndexMap::new();

    for (i, method_sym) in methods.iter().enumerate() {
        let name = vm
            .context
            .interner
            .lookup(*method_sym)
            .unwrap_or(b"")
            .to_vec();
        let val_handle = vm.arena.alloc(Val::String(name.into()));
        array.insert(ArrayKey::Int(i as i64), val_handle);
    }

    Ok(vm.arena.alloc(Val::Array(
        crate::core::value::ArrayData::from(array).into(),
    )))
}

pub fn php_get_class_vars(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("get_class_vars() expects exactly 1 parameter".into());
    }

    let val = vm.arena.get(args[0]);
    let class_sym = match &val.value {
        Val::String(s) => {
            if let Some(sym) = vm.context.interner.find(s) {
                sym
            } else {
                return Err("Class does not exist".into());
            }
        }
        _ => return Err("get_class_vars() expects a string".into()),
    };

    let caller_scope = vm.get_current_class();
    let properties =
        vm.collect_properties(class_sym, PropertyCollectionMode::VisibleTo(caller_scope));
    let mut array = IndexMap::new();

    for (prop_sym, val_handle) in properties {
        let name = vm.context.interner.lookup(prop_sym).unwrap_or(b"").to_vec();
        let key = ArrayKey::Str(Rc::new(name));
        array.insert(key, val_handle);
    }

    Ok(vm.arena.alloc(Val::Array(
        crate::core::value::ArrayData::from(array).into(),
    )))
}

pub fn php_get_called_class(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let frame = vm
        .frames
        .last()
        .ok_or("get_called_class() called from outside a function".to_string())?;

    if let Some(scope) = frame.called_scope {
        let name = vm.context.interner.lookup(scope).unwrap_or(b"").to_vec();
        Ok(vm.arena.alloc(Val::String(name.into())))
    } else {
        Err("get_called_class() called from outside a class".into())
    }
}
