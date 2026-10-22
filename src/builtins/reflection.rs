use crate::compiler::chunk::ReturnType;
use crate::core::value::{ArrayData, ArrayKey, Handle, ObjectData, Val};
use crate::runtime::context::{AttributeArg, AttributeInstance, TypeHint};
use crate::vm::engine::VM;
use indexmap::IndexMap;
use std::rc::Rc;

#[derive(Debug, Clone)]
struct ReflectionClassData {
    class_name: crate::core::value::Symbol,
}

#[derive(Debug, Clone)]
struct ReflectionAttributeData {
    name: crate::core::value::Symbol,
    args: Vec<AttributeArg>,
    /// What the attribute is attached to (VM::ATTR_TARGET_*).
    target: i64,
    /// Whether the same attribute appears more than once there.
    repeated: bool,
}

#[derive(Debug, Clone)]
struct ReflectionMethodData {
    class_name: crate::core::value::Symbol,
    method_name: crate::core::value::Symbol,
    method_lookup: crate::core::value::Symbol,
}

#[derive(Debug, Clone)]
struct ReflectionPropertyData {
    class_name: crate::core::value::Symbol,
    prop_name: crate::core::value::Symbol,
    is_static: bool,
}

#[derive(Debug, Clone)]
struct ReflectionClassConstData {
    class_name: crate::core::value::Symbol,
    const_name: crate::core::value::Symbol,
}

#[derive(Debug, Clone)]
struct ReflectionFunctionData {
    func_name: crate::core::value::Symbol,
}
#[derive(Debug, Clone)]
struct ReflectionParameterData {
    name: crate::core::value::Symbol,
    type_hint: Option<TypeHint>,
    is_variadic: bool,
    is_by_ref: bool,
    position: usize,
    /// No required parameter follows it (it has a default or is variadic).
    is_optional: bool,
    default_value: Option<Val>,
}

#[derive(Debug, Clone)]
enum ReflectionTypeKind {
    Named(TypeHint),
    Union(Vec<TypeHint>),
    Intersection(Vec<TypeHint>),
}

#[derive(Debug, Clone)]
struct ReflectionTypeData {
    kind: ReflectionTypeKind,
}

fn get_this_handle(vm: &VM, method: &str) -> Result<Handle, String> {
    vm.frames
        .last()
        .and_then(|f| f.this)
        .ok_or_else(|| format!("{method} called outside object context"))
}

fn alloc_val(vm: &mut VM, val: &Val) -> Handle {
    match val {
        Val::ConstArray(map) => {
            let mut array = ArrayData::new();
            for (key, value) in map.iter() {
                let runtime_key = match key {
                    crate::core::value::ConstArrayKey::Int(i) => ArrayKey::Int(*i),
                    crate::core::value::ConstArrayKey::Str(s) => ArrayKey::Str(s.clone()),
                };
                let value_handle = alloc_val(vm, value);
                array.insert(runtime_key, value_handle);
            }
            vm.arena.alloc(Val::Array(Rc::new(array)))
        }
        Val::Array(arr) => {
            let mut array = ArrayData::new();
            for (key, handle) in arr.map.iter() {
                array.insert(key.clone(), *handle);
            }
            vm.arena.alloc(Val::Array(Rc::new(array)))
        }
        _ => vm.arena.alloc(val.clone()),
    }
}

fn resolve_class_symbol(vm: &mut VM, arg: Handle) -> Result<crate::core::value::Symbol, String> {
    match &vm.arena.get(arg).value {
        Val::String(s) => Ok(vm.context.interner.intern(s)),
        Val::Object(payload_handle) => {
            let payload = vm.arena.get(*payload_handle);
            if let Val::ObjPayload(obj_data) = &payload.value {
                Ok(obj_data.class)
            } else {
                Err("ReflectionClass::__construct() expects class name or object".into())
            }
        }
        _ => Err("ReflectionClass::__construct() expects class name or object".into()),
    }
}

fn create_reflection_attribute(
    vm: &mut VM,
    attr: &AttributeInstance,
    target: i64,
    repeated: bool,
) -> Result<Handle, String> {
    let class_sym = vm.context.interner.intern(b"ReflectionAttribute");
    let data = ReflectionAttributeData {
        name: attr.name,
        args: attr.args.clone(),
        target,
        repeated,
    };

    let obj_data = ObjectData {
        class: class_sym,
        properties: IndexMap::new(),
        internal: Some(Rc::new(data)),
        dynamic_properties: std::collections::HashSet::new(),
        id: 0,
    };
    let payload_handle = vm.arena.alloc(Val::ObjPayload(obj_data));
    Ok(vm.arena.alloc(Val::Object(payload_handle)))
}

fn build_attribute_array(
    vm: &mut VM,
    attrs: &[AttributeInstance],
    filter: Option<&[u8]>,
    target: i64,
) -> Result<Handle, String> {
    let filter_lower = filter.map(|n| n.to_ascii_lowercase());
    let mut array = ArrayData::new();
    let mut index = 0i64;

    for attr in attrs {
        if let Some(filter) = &filter_lower {
            let attr_name_bytes = vm.context.interner.lookup(attr.name).unwrap_or(b"");
            if attr_name_bytes.to_ascii_lowercase() != *filter {
                continue;
            }
        }

        let repeated = attrs.iter().filter(|a| a.name == attr.name).count() > 1;
        let attr_obj = create_reflection_attribute(vm, attr, target, repeated)?;
        array.insert(ArrayKey::Int(index), attr_obj);
        index += 1;
    }

    Ok(vm.arena.alloc(Val::Array(Rc::new(array))))
}

fn type_hint_name_bytes(vm: &VM, hint: &TypeHint) -> Vec<u8> {
    match hint {
        TypeHint::Int => b"int".to_vec(),
        TypeHint::Float => b"float".to_vec(),
        TypeHint::String => b"string".to_vec(),
        TypeHint::Bool => b"bool".to_vec(),
        TypeHint::Array => b"array".to_vec(),
        TypeHint::Object => b"object".to_vec(),
        TypeHint::Callable => b"callable".to_vec(),
        TypeHint::Iterable => b"iterable".to_vec(),
        TypeHint::Mixed => b"mixed".to_vec(),
        TypeHint::Null => b"null".to_vec(),
        TypeHint::Class(sym) => vm.context.interner.lookup(*sym).unwrap_or(b"").to_vec(),
        TypeHint::Union(_) => b"".to_vec(),
        TypeHint::Intersection(_) => b"".to_vec(),
        TypeHint::Never => b"never".to_vec(),
        TypeHint::Void => b"void".to_vec(),
    }
}

fn type_hint_is_builtin(hint: &TypeHint) -> bool {
    matches!(
        hint,
        TypeHint::Int
            | TypeHint::Float
            | TypeHint::String
            | TypeHint::Bool
            | TypeHint::Array
            | TypeHint::Object
            | TypeHint::Callable
            | TypeHint::Iterable
            | TypeHint::Mixed
            | TypeHint::Null
            | TypeHint::Never
            | TypeHint::Void
    )
}

fn type_hint_allows_null(hint: &TypeHint) -> bool {
    match hint {
        TypeHint::Null => true,
        TypeHint::Mixed => true,
        TypeHint::Union(types) => types.iter().any(type_hint_allows_null),
        _ => false,
    }
}

fn create_reflection_type(vm: &mut VM, hint: TypeHint) -> Handle {
    match hint {
        TypeHint::Union(types) => {
            let data = ReflectionTypeData {
                kind: ReflectionTypeKind::Union(types),
            };
            let class_sym = vm.context.interner.intern(b"ReflectionUnionType");
            let obj_data = ObjectData {
                class: class_sym,
                properties: IndexMap::new(),
                internal: Some(Rc::new(data)),
                dynamic_properties: std::collections::HashSet::new(),
                id: 0,
            };
            let payload_handle = vm.arena.alloc(Val::ObjPayload(obj_data));
            vm.arena.alloc(Val::Object(payload_handle))
        }
        TypeHint::Intersection(types) => {
            let data = ReflectionTypeData {
                kind: ReflectionTypeKind::Intersection(types),
            };
            let class_sym = vm
                .context
                .interner
                .intern(b"ReflectionIntersectionType");
            let obj_data = ObjectData {
                class: class_sym,
                properties: IndexMap::new(),
                internal: Some(Rc::new(data)),
                dynamic_properties: std::collections::HashSet::new(),
                id: 0,
            };
            let payload_handle = vm.arena.alloc(Val::ObjPayload(obj_data));
            vm.arena.alloc(Val::Object(payload_handle))
        }
        other => {
            let data = ReflectionTypeData {
                kind: ReflectionTypeKind::Named(other),
            };
            let class_sym = vm.context.interner.intern(b"ReflectionNamedType");
            let obj_data = ObjectData {
                class: class_sym,
                properties: IndexMap::new(),
                internal: Some(Rc::new(data)),
                dynamic_properties: std::collections::HashSet::new(),
                id: 0,
            };
            let payload_handle = vm.arena.alloc(Val::ObjPayload(obj_data));
            vm.arena.alloc(Val::Object(payload_handle))
        }
    }
}

pub fn reflection_class_construct(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("ReflectionClass::__construct() expects 1 argument".into());
    }

    let class_sym = class_arg(vm, args[0])?;

    let this_handle = get_this_handle(vm, "ReflectionClass::__construct()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionClass::__construct() expects object context".into()),
    };

    let data = ReflectionClassData { class_name: class_sym };
    let payload = vm.arena.get_mut(payload_handle);
    if let Val::ObjPayload(ref mut obj_data) = payload.value {
        obj_data.internal = Some(Rc::new(data));
    }
    let name = name_of(vm, class_sym);
    set_props(vm, payload_handle, &[(b"name", name)]);

    Ok(vm.arena.alloc(Val::Null))
}

pub fn reflection_class_get_name(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionClass::getName()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionClass::getName() expects object context".into()),
    };
    let payload = vm.arena.get(payload_handle);
    if let Val::ObjPayload(obj_data) = &payload.value {
        if let Some(internal) = &obj_data.internal {
            if let Some(data) = internal.downcast_ref::<ReflectionClassData>() {
                let name_bytes = vm.context.interner.lookup(data.class_name).unwrap_or(b"");
                return Ok(vm.arena.alloc(Val::String(name_bytes.to_vec().into())));
            }
        }
    }
    Err("ReflectionClass::getName() called on invalid reflection object".into())
}

pub fn reflection_class_get_attributes(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionClass::getAttributes()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionClass::getAttributes() expects object context".into()),
    };

    let mut filter_name: Option<Vec<u8>> = None;
    if let Some(arg) = args.first() {
        if let Val::String(s) = &vm.arena.get(*arg).value {
            filter_name = Some(s.to_vec());
        } else {
            return Err("ReflectionClass::getAttributes() expects string name filter".into());
        }
    }

    let class_sym = {
        let payload = vm.arena.get(payload_handle);
        if let Val::ObjPayload(obj_data) = &payload.value {
            if let Some(internal) = &obj_data.internal {
                if let Some(data) = internal.downcast_ref::<ReflectionClassData>() {
                    data.class_name
                } else {
                    return Err(
                        "ReflectionClass::getAttributes() called on invalid reflection object"
                            .into(),
                    );
                }
            } else {
                return Err(
                    "ReflectionClass::getAttributes() called on invalid reflection object".into(),
                );
            }
        } else {
            return Err("ReflectionClass::getAttributes() expects object context".into());
        }
    };

    let class_def = vm
        .context
        .classes
        .get(&class_sym)
        .ok_or_else(|| "ReflectionClass::getAttributes(): Class not found".to_string())?;

    let attributes = class_def.attributes.clone();
    build_attribute_array(vm, &attributes, filter_name.as_deref(), VM::ATTR_TARGET_CLASS)
}

pub fn reflection_attribute_get_name(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionAttribute::getName()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionAttribute::getName() expects object context".into()),
    };
    let payload = vm.arena.get(payload_handle);
    if let Val::ObjPayload(obj_data) = &payload.value {
        if let Some(internal) = &obj_data.internal {
            if let Some(data) = internal.downcast_ref::<ReflectionAttributeData>() {
                let name_bytes = vm.context.interner.lookup(data.name).unwrap_or(b"");
                return Ok(vm.arena.alloc(Val::String(name_bytes.to_vec().into())));
            }
        }
    }
    Err("ReflectionAttribute::getName() called on invalid reflection object".into())
}

pub fn reflection_attribute_get_arguments(
    vm: &mut VM,
    _args: &[Handle],
) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionAttribute::getArguments()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionAttribute::getArguments() expects object context".into()),
    };

    let payload = vm.arena.get(payload_handle);
    let data = if let Val::ObjPayload(obj_data) = &payload.value {
        if let Some(internal) = &obj_data.internal {
            internal
                .downcast_ref::<ReflectionAttributeData>()
                .cloned()
        } else {
            None
        }
    } else {
        None
    }
    .ok_or_else(|| {
        "ReflectionAttribute::getArguments() called on invalid reflection object".to_string()
    })?;

    let mut array = ArrayData::new();
    let mut next_index = 0i64;
    for arg in data.args {
        let key = if let Some(name_sym) = arg.name {
            let name_bytes = vm.context.interner.lookup(name_sym).unwrap_or(b"");
            ArrayKey::Str(Rc::new(name_bytes.to_vec()))
        } else {
            let idx = next_index;
            next_index += 1;
            ArrayKey::Int(idx)
        };
        let val_handle = alloc_val(vm, &arg.value);
        array.insert(key, val_handle);
    }

    Ok(vm.arena.alloc(Val::Array(Rc::new(array))))
}

pub fn reflection_attribute_new_instance(
    vm: &mut VM,
    _args: &[Handle],
) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionAttribute::newInstance()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionAttribute::newInstance() expects object context".into()),
    };

    let payload = vm.arena.get(payload_handle);
    let data = if let Val::ObjPayload(obj_data) = &payload.value {
        if let Some(internal) = &obj_data.internal {
            internal
                .downcast_ref::<ReflectionAttributeData>()
                .cloned()
        } else {
            None
        }
    } else {
        None
    }
    .ok_or_else(|| {
        "ReflectionAttribute::newInstance() called on invalid reflection object".to_string()
    })?;

    // User attributes are validated here, not where they are declared.
    if !vm.context.classes.contains_key(&data.name) {
        let _ = vm.trigger_autoload(data.name);
    }
    if let Err(err) = vm.check_attribute_use(data.name, data.target, data.repeated) {
        let msg = match err {
            crate::vm::engine::VmError::RuntimeError(msg) => msg,
            other => format!("{:?}", other),
        };
        return Err(vm.throw_from_native(b"Error", &msg));
    }

    let mut args = Vec::new();
    for arg in data.args {
        args.push(alloc_val(vm, &arg.value));
    }

    vm.instantiate_class(data.name, &args)
        .map_err(|err| format!("ReflectionAttribute::newInstance(): {err}"))
}

pub fn reflection_method_construct(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    // new ReflectionMethod("Class::method")
    if args.len() == 1
        && let Val::String(s) = vm.arena.get(args[0]).value.clone()
        && let Some(pos) = s.windows(2).position(|w| w == b"::")
    {
        let class = vm.arena.alloc(Val::String(Rc::new(s[..pos].to_vec())));
        let method = vm.arena.alloc(Val::String(Rc::new(s[pos + 2..].to_vec())));
        return reflection_method_construct(vm, &[class, method]);
    }
    if args.len() < 2 {
        return Err("ReflectionMethod::__construct() expects 2 arguments".into());
    }
    let class_sym = class_arg(vm, args[0])?;
    let method_bytes = vm.arena.get(args[1]).value.to_php_string_bytes();
    let Some(entry) = method_of(vm, class_sym, &method_bytes) else {
        let msg = format!(
            "Method {}::{}() does not exist",
            String::from_utf8_lossy(&name_of(vm, class_sym)),
            String::from_utf8_lossy(&method_bytes)
        );
        return Err(reflection_error(vm, &msg));
    };
    let method_sym = entry.name;
    let method_lookup = vm.context.interner.intern(&method_bytes.to_ascii_lowercase());

    let this_handle = get_this_handle(vm, "ReflectionMethod::__construct()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionMethod::__construct() expects object context".into()),
    };

    let data = ReflectionMethodData {
        class_name: class_sym,
        method_name: method_sym,
        method_lookup,
    };
    let payload = vm.arena.get_mut(payload_handle);
    if let Val::ObjPayload(ref mut obj_data) = payload.value {
        obj_data.internal = Some(Rc::new(data));
    }
    let declaring = method_of(vm, class_sym, &name_of(vm, method_sym)).map_or(class_sym, |m| m.declaring_class);
    let (name, class) = (name_of(vm, method_sym), name_of(vm, declaring));
    set_props(vm, payload_handle, &[(b"name", name), (b"class", class)]);

    Ok(vm.arena.alloc(Val::Null))
}

pub fn reflection_method_get_name(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionMethod::getName()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionMethod::getName() expects object context".into()),
    };
    let payload = vm.arena.get(payload_handle);
    if let Val::ObjPayload(obj_data) = &payload.value {
        if let Some(internal) = &obj_data.internal {
            if let Some(data) = internal.downcast_ref::<ReflectionMethodData>() {
                let name_bytes = vm.context.interner.lookup(data.method_name).unwrap_or(b"");
                return Ok(vm.arena.alloc(Val::String(name_bytes.to_vec().into())));
            }
        }
    }
    Err("ReflectionMethod::getName() called on invalid reflection object".into())
}

pub fn reflection_method_get_attributes(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionMethod::getAttributes()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionMethod::getAttributes() expects object context".into()),
    };

    let filter_name = if let Some(arg) = args.first() {
        if let Val::String(s) = &vm.arena.get(*arg).value {
            Some(s.to_vec())
        } else {
            return Err("ReflectionMethod::getAttributes() expects string name filter".into());
        }
    } else {
        None
    };

    let data = {
        let payload = vm.arena.get(payload_handle);
        if let Val::ObjPayload(obj_data) = &payload.value {
            if let Some(internal) = &obj_data.internal {
                internal.downcast_ref::<ReflectionMethodData>().cloned()
            } else {
                None
            }
        } else {
            None
        }
    }
    .ok_or_else(|| "ReflectionMethod::getAttributes() called on invalid reflection object".to_string())?;

    let class_def = vm
        .context
        .classes
        .get(&data.class_name)
        .ok_or("ReflectionMethod::getAttributes(): Class not found")?;
    let attributes = class_def
        .methods
        .get(&data.method_lookup)
        .map(|entry| entry.attributes.clone())
        .unwrap_or_default();

    build_attribute_array(vm, &attributes, filter_name.as_deref(), VM::ATTR_TARGET_METHOD)
}

pub fn reflection_method_get_parameters(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionMethod::getParameters()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionMethod::getParameters() expects object context".into()),
    };

    let data = {
        let payload = vm.arena.get(payload_handle);
        if let Val::ObjPayload(obj_data) = &payload.value {
            if let Some(internal) = &obj_data.internal {
                internal.downcast_ref::<ReflectionMethodData>().cloned()
            } else {
                None
            }
        } else {
            None
        }
    }
    .ok_or_else(|| {
        "ReflectionMethod::getParameters() called on invalid reflection object".to_string()
    })?;

    let class_def = vm
        .context
        .classes
        .get(&data.class_name)
        .ok_or("ReflectionMethod::getParameters(): Class not found")?;
    let params = class_def
        .methods
        .get(&data.method_lookup)
        .ok_or("ReflectionMethod::getParameters(): Method not found")?
        .signature
        .parameters
        .clone();

    let mut array = ArrayData::new();
    for idx in 0..params.len() {
        let param_obj = create_reflection_parameter(vm, &params, idx)?;
        array.insert(ArrayKey::Int(idx as i64), param_obj);
    }

    Ok(vm.arena.alloc(Val::Array(Rc::new(array))))
}

pub fn reflection_method_get_return_type(
    vm: &mut VM,
    _args: &[Handle],
) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionMethod::getReturnType()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionMethod::getReturnType() expects object context".into()),
    };

    let data = {
        let payload = vm.arena.get(payload_handle);
        if let Val::ObjPayload(obj_data) = &payload.value {
            if let Some(internal) = &obj_data.internal {
                internal.downcast_ref::<ReflectionMethodData>().cloned()
            } else {
                None
            }
        } else {
            None
        }
    }
    .ok_or_else(|| {
        "ReflectionMethod::getReturnType() called on invalid reflection object".to_string()
    })?;

    let class_def = vm
        .context
        .classes
        .get(&data.class_name)
        .ok_or("ReflectionMethod::getReturnType(): Class not found")?;
    let entry = class_def
        .methods
        .get(&data.method_lookup)
        .ok_or("ReflectionMethod::getReturnType(): Method not found")?;

    if let Some(hint) = &entry.signature.return_type {
        Ok(create_reflection_type(vm, hint.clone()))
    } else {
        Ok(vm.arena.alloc(Val::Null))
    }
}

pub fn reflection_property_construct(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("ReflectionProperty::__construct() expects 2 arguments".into());
    }
    let class_sym = class_arg(vm, args[0])?;
    let prop_bytes = vm.arena.get(args[1]).value.to_php_string_bytes();
    let prop_sym = vm.context.interner.intern(&prop_bytes);
    let Some((_, _, is_static)) = property_of(vm, class_sym, prop_sym) else {
        let msg = format!(
            "Property {}::${} does not exist",
            String::from_utf8_lossy(&name_of(vm, class_sym)),
            String::from_utf8_lossy(&prop_bytes)
        );
        return Err(reflection_error(vm, &msg));
    };

    let this_handle = get_this_handle(vm, "ReflectionProperty::__construct()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionProperty::__construct() expects object context".into()),
    };

    let data = ReflectionPropertyData {
        class_name: class_sym,
        prop_name: prop_sym,
        is_static,
    };
    let payload = vm.arena.get_mut(payload_handle);
    if let Val::ObjPayload(ref mut obj_data) = payload.value {
        obj_data.internal = Some(Rc::new(data));
    }
    let declaring = property_of(vm, class_sym, prop_sym).map_or(class_sym, |p| p.0);
    let (name, class) = (name_of(vm, prop_sym), name_of(vm, declaring));
    set_props(vm, payload_handle, &[(b"name", name), (b"class", class)]);

    Ok(vm.arena.alloc(Val::Null))
}

pub fn reflection_property_get_name(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionProperty::getName()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionProperty::getName() expects object context".into()),
    };
    let payload = vm.arena.get(payload_handle);
    if let Val::ObjPayload(obj_data) = &payload.value {
        if let Some(internal) = &obj_data.internal {
            if let Some(data) = internal.downcast_ref::<ReflectionPropertyData>() {
                let name_bytes = vm.context.interner.lookup(data.prop_name).unwrap_or(b"");
                return Ok(vm.arena.alloc(Val::String(name_bytes.to_vec().into())));
            }
        }
    }
    Err("ReflectionProperty::getName() called on invalid reflection object".into())
}

pub fn reflection_property_get_attributes(
    vm: &mut VM,
    args: &[Handle],
) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionProperty::getAttributes()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionProperty::getAttributes() expects object context".into()),
    };

    let filter_name = if let Some(arg) = args.first() {
        if let Val::String(s) = &vm.arena.get(*arg).value {
            Some(s.to_vec())
        } else {
            return Err("ReflectionProperty::getAttributes() expects string name filter".into());
        }
    } else {
        None
    };

    let data = {
        let payload = vm.arena.get(payload_handle);
        if let Val::ObjPayload(obj_data) = &payload.value {
            if let Some(internal) = &obj_data.internal {
                internal.downcast_ref::<ReflectionPropertyData>().cloned()
            } else {
                None
            }
        } else {
            None
        }
    }
    .ok_or_else(|| "ReflectionProperty::getAttributes() called on invalid reflection object".to_string())?;

    let class_def = vm
        .context
        .classes
        .get(&data.class_name)
        .ok_or("ReflectionProperty::getAttributes(): Class not found")?;
    let attributes = if data.is_static {
        class_def
            .static_properties
            .get(&data.prop_name)
            .map(|entry| entry.attributes.clone())
            .unwrap_or_default()
    } else {
        class_def
            .properties
            .get(&data.prop_name)
            .map(|entry| entry.attributes.clone())
            .unwrap_or_default()
    };

    build_attribute_array(vm, &attributes, filter_name.as_deref(), VM::ATTR_TARGET_PROPERTY)
}

pub fn reflection_property_has_type(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionProperty::hasType()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionProperty::hasType() expects object context".into()),
    };

    let data = {
        let payload = vm.arena.get(payload_handle);
        if let Val::ObjPayload(obj_data) = &payload.value {
            if let Some(internal) = &obj_data.internal {
                internal.downcast_ref::<ReflectionPropertyData>().cloned()
            } else {
                None
            }
        } else {
            None
        }
    }
    .ok_or_else(|| {
        "ReflectionProperty::hasType() called on invalid reflection object".to_string()
    })?;

    let class_def = vm
        .context
        .classes
        .get(&data.class_name)
        .ok_or("ReflectionProperty::hasType(): Class not found")?;
    let has_type = if data.is_static {
        class_def
            .static_properties
            .get(&data.prop_name)
            .and_then(|entry| entry.type_hint.clone())
            .is_some()
    } else {
        class_def
            .properties
            .get(&data.prop_name)
            .and_then(|entry| entry.type_hint.clone())
            .is_some()
    };

    Ok(vm.arena.alloc(Val::Bool(has_type)))
}

pub fn reflection_property_get_type(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionProperty::getType()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionProperty::getType() expects object context".into()),
    };

    let data = {
        let payload = vm.arena.get(payload_handle);
        if let Val::ObjPayload(obj_data) = &payload.value {
            if let Some(internal) = &obj_data.internal {
                internal.downcast_ref::<ReflectionPropertyData>().cloned()
            } else {
                None
            }
        } else {
            None
        }
    }
    .ok_or_else(|| {
        "ReflectionProperty::getType() called on invalid reflection object".to_string()
    })?;

    let class_def = vm
        .context
        .classes
        .get(&data.class_name)
        .ok_or("ReflectionProperty::getType(): Class not found")?;
    let hint = if data.is_static {
        class_def
            .static_properties
            .get(&data.prop_name)
            .and_then(|entry| entry.type_hint.clone())
    } else {
        class_def
            .properties
            .get(&data.prop_name)
            .and_then(|entry| entry.type_hint.clone())
    };

    if let Some(hint) = hint {
        Ok(create_reflection_type(vm, hint))
    } else {
        Ok(vm.arena.alloc(Val::Null))
    }
}

pub fn reflection_class_const_construct(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("ReflectionClassConstant::__construct() expects 2 arguments".into());
    }
    let class_sym = resolve_class_symbol(vm, args[0])?;
    let const_bytes = match &vm.arena.get(args[1]).value {
        Val::String(s) => s.clone(),
        _ => {
            return Err(
                "ReflectionClassConstant::__construct() expects constant name string".into(),
            )
        }
    };
    let const_sym = vm.context.interner.intern(&const_bytes);

    let class_def = vm
        .context
        .classes
        .get(&class_sym)
        .ok_or("ReflectionClassConstant::__construct(): Class not found")?;
    if !class_def.constants.contains_key(&const_sym) {
        return Err("ReflectionClassConstant::__construct(): Constant not found".into());
    }

    let this_handle = get_this_handle(vm, "ReflectionClassConstant::__construct()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionClassConstant::__construct() expects object context".into()),
    };

    let data = ReflectionClassConstData {
        class_name: class_sym,
        const_name: const_sym,
    };
    let payload = vm.arena.get_mut(payload_handle);
    if let Val::ObjPayload(ref mut obj_data) = payload.value {
        obj_data.internal = Some(Rc::new(data));
    }
    let (name, class) = (name_of(vm, const_sym), name_of(vm, class_sym));
    set_props(vm, payload_handle, &[(b"name", name), (b"class", class)]);

    Ok(vm.arena.alloc(Val::Null))
}

pub fn reflection_class_const_get_name(
    vm: &mut VM,
    _args: &[Handle],
) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionClassConstant::getName()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionClassConstant::getName() expects object context".into()),
    };
    let payload = vm.arena.get(payload_handle);
    if let Val::ObjPayload(obj_data) = &payload.value {
        if let Some(internal) = &obj_data.internal {
            if let Some(data) = internal.downcast_ref::<ReflectionClassConstData>() {
                let name_bytes = vm.context.interner.lookup(data.const_name).unwrap_or(b"");
                return Ok(vm.arena.alloc(Val::String(name_bytes.to_vec().into())));
            }
        }
    }
    Err("ReflectionClassConstant::getName() called on invalid reflection object".into())
}

pub fn reflection_class_const_get_attributes(
    vm: &mut VM,
    args: &[Handle],
) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionClassConstant::getAttributes()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionClassConstant::getAttributes() expects object context".into()),
    };

    let filter_name = if let Some(arg) = args.first() {
        if let Val::String(s) = &vm.arena.get(*arg).value {
            Some(s.to_vec())
        } else {
            return Err("ReflectionClassConstant::getAttributes() expects string name filter".into());
        }
    } else {
        None
    };

    let data = {
        let payload = vm.arena.get(payload_handle);
        if let Val::ObjPayload(obj_data) = &payload.value {
            if let Some(internal) = &obj_data.internal {
                internal.downcast_ref::<ReflectionClassConstData>().cloned()
            } else {
                None
            }
        } else {
            None
        }
    }
    .ok_or_else(|| {
        "ReflectionClassConstant::getAttributes() called on invalid reflection object".to_string()
    })?;

    let class_def = vm
        .context
        .classes
        .get(&data.class_name)
        .ok_or("ReflectionClassConstant::getAttributes(): Class not found")?;
    let attributes = class_def
        .constants
        .get(&data.const_name)
        .map(|entry| entry.attributes.clone())
        .unwrap_or_default();

    build_attribute_array(vm, &attributes, filter_name.as_deref(), VM::ATTR_TARGET_CLASS_CONSTANT)
}

pub fn reflection_parameter_get_name(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionParameter::getName()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionParameter::getName() expects object context".into()),
    };

    let payload = vm.arena.get(payload_handle);
    if let Val::ObjPayload(obj_data) = &payload.value {
        if let Some(internal) = &obj_data.internal {
            if let Some(data) = internal.downcast_ref::<ReflectionParameterData>() {
                let name_bytes = vm.context.interner.lookup(data.name).unwrap_or(b"");
                return Ok(vm.arena.alloc(Val::String(name_bytes.to_vec().into())));
            }
        }
    }
    Err("ReflectionParameter::getName() called on invalid reflection object".into())
}

pub fn reflection_parameter_has_type(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionParameter::hasType()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionParameter::hasType() expects object context".into()),
    };

    let payload = vm.arena.get(payload_handle);
    if let Val::ObjPayload(obj_data) = &payload.value {
        if let Some(internal) = &obj_data.internal {
            if let Some(data) = internal.downcast_ref::<ReflectionParameterData>() {
                return Ok(vm
                    .arena
                    .alloc(Val::Bool(data.type_hint.is_some())));
            }
        }
    }
    Err("ReflectionParameter::hasType() called on invalid reflection object".into())
}

pub fn reflection_parameter_get_type(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionParameter::getType()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionParameter::getType() expects object context".into()),
    };

    let payload = vm.arena.get(payload_handle);
    if let Val::ObjPayload(obj_data) = &payload.value {
        if let Some(internal) = &obj_data.internal {
            if let Some(data) = internal.downcast_ref::<ReflectionParameterData>() {
                if let Some(hint) = &data.type_hint {
                    return Ok(create_reflection_type(vm, hint.clone()));
                }
                return Ok(vm.arena.alloc(Val::Null));
            }
        }
    }
    Err("ReflectionParameter::getType() called on invalid reflection object".into())
}

pub fn reflection_parameter_is_variadic(
    vm: &mut VM,
    _args: &[Handle],
) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionParameter::isVariadic()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionParameter::isVariadic() expects object context".into()),
    };

    let payload = vm.arena.get(payload_handle);
    if let Val::ObjPayload(obj_data) = &payload.value {
        if let Some(internal) = &obj_data.internal {
            if let Some(data) = internal.downcast_ref::<ReflectionParameterData>() {
                return Ok(vm.arena.alloc(Val::Bool(data.is_variadic)));
            }
        }
    }
    Err("ReflectionParameter::isVariadic() called on invalid reflection object".into())
}

pub fn reflection_parameter_is_passed_by_reference(
    vm: &mut VM,
    _args: &[Handle],
) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionParameter::isPassedByReference()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionParameter::isPassedByReference() expects object context".into()),
    };

    let payload = vm.arena.get(payload_handle);
    if let Val::ObjPayload(obj_data) = &payload.value {
        if let Some(internal) = &obj_data.internal {
            if let Some(data) = internal.downcast_ref::<ReflectionParameterData>() {
                return Ok(vm.arena.alloc(Val::Bool(data.is_by_ref)));
            }
        }
    }
    Err(
        "ReflectionParameter::isPassedByReference() called on invalid reflection object".into(),
    )
}

pub fn reflection_named_type_get_name(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionNamedType::getName()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionNamedType::getName() expects object context".into()),
    };

    let payload = vm.arena.get(payload_handle);
    if let Val::ObjPayload(obj_data) = &payload.value {
        if let Some(internal) = &obj_data.internal {
            if let Some(data) = internal.downcast_ref::<ReflectionTypeData>() {
                if let ReflectionTypeKind::Named(hint) = &data.kind {
                    let name_bytes = type_hint_name_bytes(vm, hint);
                    return Ok(vm.arena.alloc(Val::String(name_bytes.into())));
                }
            }
        }
    }
    Err("ReflectionNamedType::getName() called on invalid reflection object".into())
}

pub fn reflection_named_type_allows_null(
    vm: &mut VM,
    _args: &[Handle],
) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionNamedType::allowsNull()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionNamedType::allowsNull() expects object context".into()),
    };

    let payload = vm.arena.get(payload_handle);
    if let Val::ObjPayload(obj_data) = &payload.value {
        if let Some(internal) = &obj_data.internal {
            if let Some(data) = internal.downcast_ref::<ReflectionTypeData>() {
                if let ReflectionTypeKind::Named(hint) = &data.kind {
                    return Ok(vm.arena.alloc(Val::Bool(type_hint_allows_null(hint))));
                }
            }
        }
    }
    Err("ReflectionNamedType::allowsNull() called on invalid reflection object".into())
}

pub fn reflection_named_type_is_builtin(
    vm: &mut VM,
    _args: &[Handle],
) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionNamedType::isBuiltin()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionNamedType::isBuiltin() expects object context".into()),
    };

    let payload = vm.arena.get(payload_handle);
    if let Val::ObjPayload(obj_data) = &payload.value {
        if let Some(internal) = &obj_data.internal {
            if let Some(data) = internal.downcast_ref::<ReflectionTypeData>() {
                if let ReflectionTypeKind::Named(hint) = &data.kind {
                    return Ok(vm.arena.alloc(Val::Bool(type_hint_is_builtin(hint))));
                }
            }
        }
    }
    Err("ReflectionNamedType::isBuiltin() called on invalid reflection object".into())
}

pub fn reflection_union_type_get_types(
    vm: &mut VM,
    _args: &[Handle],
) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionUnionType::getTypes()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionUnionType::getTypes() expects object context".into()),
    };

    let types = {
        let payload = vm.arena.get(payload_handle);
        if let Val::ObjPayload(obj_data) = &payload.value {
            if let Some(internal) = &obj_data.internal {
                if let Some(data) = internal.downcast_ref::<ReflectionTypeData>() {
                    if let ReflectionTypeKind::Union(types) = &data.kind {
                        Some(types.clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    };

    if let Some(types) = types {
        let mut array = ArrayData::new();
        for (idx, hint) in types.iter().enumerate() {
            let type_obj = create_reflection_type(vm, hint.clone());
            array.insert(ArrayKey::Int(idx as i64), type_obj);
        }
        return Ok(vm.arena.alloc(Val::Array(Rc::new(array))));
    }
    Err("ReflectionUnionType::getTypes() called on invalid reflection object".into())
}

pub fn reflection_union_type_allows_null(
    vm: &mut VM,
    _args: &[Handle],
) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionUnionType::allowsNull()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionUnionType::allowsNull() expects object context".into()),
    };

    let payload = vm.arena.get(payload_handle);
    if let Val::ObjPayload(obj_data) = &payload.value {
        if let Some(internal) = &obj_data.internal {
            if let Some(data) = internal.downcast_ref::<ReflectionTypeData>() {
                if let ReflectionTypeKind::Union(types) = &data.kind {
                    let allows_null = types.iter().any(type_hint_allows_null);
                    return Ok(vm.arena.alloc(Val::Bool(allows_null)));
                }
            }
        }
    }
    Err("ReflectionUnionType::allowsNull() called on invalid reflection object".into())
}

pub fn reflection_intersection_type_get_types(
    vm: &mut VM,
    _args: &[Handle],
) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionIntersectionType::getTypes()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionIntersectionType::getTypes() expects object context".into()),
    };

    let types = {
        let payload = vm.arena.get(payload_handle);
        if let Val::ObjPayload(obj_data) = &payload.value {
            if let Some(internal) = &obj_data.internal {
                if let Some(data) = internal.downcast_ref::<ReflectionTypeData>() {
                    if let ReflectionTypeKind::Intersection(types) = &data.kind {
                        Some(types.clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    };

    if let Some(types) = types {
        let mut array = ArrayData::new();
        for (idx, hint) in types.iter().enumerate() {
            let type_obj = create_reflection_type(vm, hint.clone());
            array.insert(ArrayKey::Int(idx as i64), type_obj);
        }
        return Ok(vm.arena.alloc(Val::Array(Rc::new(array))));
    }
    Err("ReflectionIntersectionType::getTypes() called on invalid reflection object".into())
}

pub fn reflection_intersection_type_allows_null(
    vm: &mut VM,
    _args: &[Handle],
) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionIntersectionType::allowsNull()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionIntersectionType::allowsNull() expects object context".into()),
    };

    let payload = vm.arena.get(payload_handle);
    if let Val::ObjPayload(obj_data) = &payload.value {
        if let Some(internal) = &obj_data.internal {
            if let Some(data) = internal.downcast_ref::<ReflectionTypeData>() {
                if let ReflectionTypeKind::Intersection(types) = &data.kind {
                    let allows_null = types.iter().any(type_hint_allows_null);
                    return Ok(vm.arena.alloc(Val::Bool(allows_null)));
                }
            }
        }
    }
    Err("ReflectionIntersectionType::allowsNull() called on invalid reflection object".into())
}

fn create_reflection_parameter(
    vm: &mut VM,
    params: &[crate::runtime::context::ParameterInfo],
    position: usize,
) -> Result<Handle, String> {
    let param = &params[position];
    let is_optional = params[position..]
        .iter()
        .all(|p| p.default_value.is_some() || p.is_variadic);
    let data = ReflectionParameterData {
        name: param.name,
        type_hint: param.type_hint.clone(),
        is_variadic: param.is_variadic,
        is_by_ref: param.is_reference,
        position,
        is_optional,
        default_value: param.default_value.clone(),
    };

    let class_sym = vm.context.interner.intern(b"ReflectionParameter");
    let obj_data = ObjectData {
        class: class_sym,
        properties: IndexMap::new(),
        internal: Some(Rc::new(data)),
        dynamic_properties: std::collections::HashSet::new(),
        id: 0,
    };
    let payload_handle = vm.arena.alloc(Val::ObjPayload(obj_data));
    Ok(vm.arena.alloc(Val::Object(payload_handle)))
}

fn find_user_function_symbol(vm: &VM, name: &[u8]) -> Option<crate::core::value::Symbol> {
    let name_lower = name.to_ascii_lowercase();
    for (&sym, _func) in vm.context.user_functions.iter() {
        if let Some(stored) = vm.context.interner.lookup(sym) {
            if stored.eq(name) || stored.to_ascii_lowercase() == name_lower {
                return Some(sym);
            }
        }
    }
    None
}

pub fn reflection_function_construct(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("ReflectionFunction::__construct() expects 1 argument".into());
    }
    let name_bytes = match &vm.arena.get(args[0]).value {
        Val::String(s) => s.clone(),
        _ => return Err("ReflectionFunction::__construct() expects function name string".into()),
    };

    let func_sym = find_user_function_symbol(vm, &name_bytes)
        .ok_or_else(|| "ReflectionFunction::__construct(): Function not found".to_string())?;

    let this_handle = get_this_handle(vm, "ReflectionFunction::__construct()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionFunction::__construct() expects object context".into()),
    };

    let data = ReflectionFunctionData { func_name: func_sym };
    let payload = vm.arena.get_mut(payload_handle);
    if let Val::ObjPayload(ref mut obj_data) = payload.value {
        obj_data.internal = Some(Rc::new(data));
    }
    let name = name_of(vm, func_sym);
    set_props(vm, payload_handle, &[(b"name", name)]);

    Ok(vm.arena.alloc(Val::Null))
}

pub fn reflection_function_get_name(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionFunction::getName()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionFunction::getName() expects object context".into()),
    };
    let payload = vm.arena.get(payload_handle);
    if let Val::ObjPayload(obj_data) = &payload.value {
        if let Some(internal) = &obj_data.internal {
            if let Some(data) = internal.downcast_ref::<ReflectionFunctionData>() {
                let name_bytes = vm.context.interner.lookup(data.func_name).unwrap_or(b"");
                return Ok(vm.arena.alloc(Val::String(name_bytes.to_vec().into())));
            }
        }
    }
    Err("ReflectionFunction::getName() called on invalid reflection object".into())
}

pub fn reflection_function_get_attributes(
    vm: &mut VM,
    args: &[Handle],
) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionFunction::getAttributes()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionFunction::getAttributes() expects object context".into()),
    };

    let filter_name = if let Some(arg) = args.first() {
        if let Val::String(s) = &vm.arena.get(*arg).value {
            Some(s.to_vec())
        } else {
            return Err("ReflectionFunction::getAttributes() expects string name filter".into());
        }
    } else {
        None
    };

    let func_sym = {
        let payload = vm.arena.get(payload_handle);
        if let Val::ObjPayload(obj_data) = &payload.value {
            if let Some(internal) = &obj_data.internal {
                internal.downcast_ref::<ReflectionFunctionData>().map(|d| d.func_name)
            } else {
                None
            }
        } else {
            None
        }
    }
    .ok_or_else(|| "ReflectionFunction::getAttributes() called on invalid reflection object".to_string())?;

    let attrs = vm
        .context
        .user_functions
        .get(&func_sym)
        .ok_or("ReflectionFunction::getAttributes(): Function not found")?
        .attributes
        .clone();

    build_attribute_array(vm, &attrs, filter_name.as_deref(), VM::ATTR_TARGET_FUNCTION)
}

pub fn reflection_function_get_parameters(
    vm: &mut VM,
    _args: &[Handle],
) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionFunction::getParameters()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionFunction::getParameters() expects object context".into()),
    };

    let func_sym = {
        let payload = vm.arena.get(payload_handle);
        if let Val::ObjPayload(obj_data) = &payload.value {
            if let Some(internal) = &obj_data.internal {
                internal.downcast_ref::<ReflectionFunctionData>().map(|d| d.func_name)
            } else {
                None
            }
        } else {
            None
        }
    }
    .ok_or_else(|| "ReflectionFunction::getParameters() called on invalid reflection object".to_string())?;

    let params = vm
        .context
        .user_functions
        .get(&func_sym)
        .ok_or("ReflectionFunction::getParameters(): Function not found")?
        .params
        .clone();

    let infos: Vec<crate::runtime::context::ParameterInfo> = params
        .iter()
        .map(|param| crate::runtime::context::ParameterInfo {
            name: param.name,
            type_hint: param.param_type.as_ref().and_then(return_type_to_type_hint),
            is_reference: param.by_ref,
            is_variadic: param.is_variadic,
            default_value: param.default_value.clone(),
            attributes: param.attributes.clone(),
        })
        .collect();
    let mut array = ArrayData::new();
    for idx in 0..infos.len() {
        let param_obj = create_reflection_parameter(vm, &infos, idx)?;
        array.insert(ArrayKey::Int(idx as i64), param_obj);
    }

    Ok(vm.arena.alloc(Val::Array(Rc::new(array))))
}

pub fn reflection_function_get_return_type(
    vm: &mut VM,
    _args: &[Handle],
) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionFunction::getReturnType()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionFunction::getReturnType() expects object context".into()),
    };

    let func_sym = {
        let payload = vm.arena.get(payload_handle);
        if let Val::ObjPayload(obj_data) = &payload.value {
            if let Some(internal) = &obj_data.internal {
                internal.downcast_ref::<ReflectionFunctionData>().map(|d| d.func_name)
            } else {
                None
            }
        } else {
            None
        }
    }
    .ok_or_else(|| "ReflectionFunction::getReturnType() called on invalid reflection object".to_string())?;

    let func = vm
        .context
        .user_functions
        .get(&func_sym)
        .ok_or("ReflectionFunction::getReturnType(): Function not found")?;

    if let Some(rt) = &func.return_type {
        if let Some(hint) = return_type_to_type_hint(rt) {
            return Ok(create_reflection_type(vm, hint));
        }
    }

    Ok(vm.arena.alloc(Val::Null))
}
fn return_type_to_type_hint(rt: &ReturnType) -> Option<TypeHint> {
    match rt {
        ReturnType::Int => Some(TypeHint::Int),
        ReturnType::Float => Some(TypeHint::Float),
        ReturnType::String => Some(TypeHint::String),
        ReturnType::Bool => Some(TypeHint::Bool),
        ReturnType::Array => Some(TypeHint::Array),
        ReturnType::Object => Some(TypeHint::Object),
        ReturnType::Callable => Some(TypeHint::Callable),
        ReturnType::Iterable => Some(TypeHint::Iterable),
        ReturnType::Mixed => Some(TypeHint::Mixed),
        ReturnType::Void => Some(TypeHint::Void),
        ReturnType::Never => Some(TypeHint::Never),
        ReturnType::Null => Some(TypeHint::Null),
        ReturnType::Named(sym) => Some(TypeHint::Class(*sym)),
        ReturnType::Union(types) => {
            let hints: Vec<_> = types.iter().filter_map(return_type_to_type_hint).collect();
            if hints.is_empty() {
                None
            } else {
                Some(TypeHint::Union(hints))
            }
        }
        ReturnType::Intersection(types) => {
            let hints: Vec<_> = types.iter().filter_map(return_type_to_type_hint).collect();
            if hints.is_empty() {
                None
            } else {
                Some(TypeHint::Intersection(hints))
            }
        }
        ReturnType::Nullable(inner) => return_type_to_type_hint(inner).map(|hint| {
            TypeHint::Union(vec![hint, TypeHint::Null])
        }),
        _ => None,
    }
}

// ============================================================================
// Core ReflectionClass / ReflectionObject / ReflectionProperty /
// ReflectionMethod / ReflectionFunction / ReflectionParameter API: the methods
// php-src's reflection tests use most, on this module's data types.
// Reference: $PHP_SRC_PATH/ext/reflection/php_reflection.c
// ============================================================================

use crate::core::value::{Symbol, Visibility};
use crate::runtime::context::ClassDef;

// ReflectionMethod / ReflectionProperty / ReflectionClass modifier bits.
const IS_STATIC: i64 = 16;
const IS_ABSTRACT: i64 = 64;
const IS_FINAL: i64 = 32;
const IS_PUBLIC: i64 = 1;
const IS_PROTECTED: i64 = 2;
const IS_PRIVATE: i64 = 4;
const IS_READONLY: i64 = 128;

/// The internal data of `$this` for a reflection method.
fn this_data<T: Clone + 'static>(vm: &VM, method: &str) -> Result<T, String> {
    let this_handle = get_this_handle(vm, method)?;
    if let Val::Object(payload) = &vm.arena.get(this_handle).value
        && let Val::ObjPayload(obj) = &vm.arena.get(*payload).value
        && let Some(data) = obj.internal.as_ref().and_then(|i| i.downcast_ref::<T>())
    {
        return Ok(data.clone());
    }
    Err(format!("{method} called on an uninitialized reflection object"))
}

/// A new reflection object of `class` carrying `data`.
fn new_reflection<T: 'static>(vm: &mut VM, class: &[u8], data: T) -> Handle {
    let class_sym = vm.context.interner.intern(class);
    let payload = vm.arena.alloc(Val::ObjPayload(ObjectData {
        class: class_sym,
        properties: IndexMap::new(),
        internal: Some(Rc::new(data)),
        dynamic_properties: std::collections::HashSet::new(),
        id: 0,
    }));
    vm.arena.alloc(Val::Object(payload))
}

/// Set the internal data of `$this` (used by constructors).
fn set_this_data<T: 'static>(vm: &mut VM, method: &str, data: T) -> Result<(), String> {
    let this_handle = get_this_handle(vm, method)?;
    if let Val::Object(payload) = vm.arena.get(this_handle).value
        && let Val::ObjPayload(obj) = &mut vm.arena.get_mut(payload).value
    {
        obj.internal = Some(Rc::new(data));
    }
    Ok(())
}

fn name_of(vm: &VM, sym: Symbol) -> Vec<u8> {
    vm.context.interner.lookup(sym).unwrap_or(b"").to_vec()
}

fn string_handle(vm: &mut VM, bytes: &[u8]) -> Handle {
    vm.arena.alloc(Val::String(Rc::new(bytes.to_vec())))
}

fn bool_handle(vm: &mut VM, b: bool) -> Handle {
    vm.arena.alloc(Val::Bool(b))
}

fn reflection_error(vm: &mut VM, msg: &str) -> String {
    vm.throw_from_native(b"ReflectionException", msg)
}

fn class_of(vm: &VM, class: Symbol) -> Result<&ClassDef, String> {
    vm.context
        .classes
        .get(&class)
        .ok_or_else(|| format!("Class \"{}\" does not exist", String::from_utf8_lossy(&name_of(vm, class))))
}

/// Look a class up by name, autoloading it, case-insensitively.
fn find_class(vm: &mut VM, name: &[u8]) -> Option<Symbol> {
    let name = name.strip_prefix(b"\\").unwrap_or(name);
    let sym = vm.context.interner.intern(name);
    if vm.context.classes.contains_key(&sym) {
        return Some(sym);
    }
    let _ = vm.trigger_autoload(sym);
    if vm.context.classes.contains_key(&sym) {
        return Some(sym);
    }
    let lower = name.to_ascii_lowercase();
    vm.context
        .classes
        .keys()
        .copied()
        .find(|k| vm.context.interner.lookup(*k).is_some_and(|n| n.to_ascii_lowercase() == lower))
}

/// A class named by a string or given as an object (or a ReflectionClass).
fn class_arg(vm: &mut VM, arg: Handle) -> Result<Symbol, String> {
    match vm.arena.get(arg).value.clone() {
        Val::String(s) => find_class(vm, &s).ok_or_else(|| {
            reflection_error(vm, &format!("Class \"{}\" does not exist", String::from_utf8_lossy(&s)))
        }),
        Val::Object(payload) => match &vm.arena.get(payload).value {
            Val::ObjPayload(obj) => match obj.internal.as_ref().and_then(|i| i.downcast_ref::<ReflectionClassData>()) {
                Some(data) => Ok(data.class_name),
                None => Ok(obj.class),
            },
            _ => Err("invalid object".into()),
        },
        other => Err(vm.throw_from_native(
            b"TypeError",
            &format!("Argument #1 ($objectOrClass) must be of type object|string, {} given", other.type_name()),
        )),
    }
}

/// The class of an object handle.
fn object_class(vm: &VM, obj: Handle) -> Option<(Symbol, Handle)> {
    if let Val::Object(payload) = vm.arena.get(obj).value
        && let Val::ObjPayload(data) = &vm.arena.get(payload).value
    {
        return Some((data.class, payload));
    }
    None
}

fn reflection_class_for(vm: &mut VM, class: Symbol) -> Handle {
    let obj = new_reflection(vm, b"ReflectionClass", ReflectionClassData { class_name: class });
    let name = name_of(vm, class);
    set_props_on(vm, obj, &[(b"name", name)]);
    obj
}

/// A method of `class` by (any-case) name: (class it was asked on, entry).
fn method_of(vm: &VM, class: Symbol, name: &[u8]) -> Option<crate::runtime::context::MethodEntry> {
    let lower = vm.context.interner.find(&name.to_ascii_lowercase())?;
    vm.walk_inheritance_chain(class, |def, _| def.methods.get(&lower).cloned())
}

fn reflection_method_for(vm: &mut VM, class: Symbol, entry: &crate::runtime::context::MethodEntry) -> Handle {
    let lower = name_of(vm, entry.name).to_ascii_lowercase();
    let method_lookup = vm.context.interner.intern(&lower);
    let obj = new_reflection(
        vm,
        b"ReflectionMethod",
        ReflectionMethodData {
            class_name: class,
            method_name: entry.name,
            method_lookup,
        },
    );
    let (name, class) = (name_of(vm, entry.name), name_of(vm, entry.declaring_class));
    set_props_on(vm, obj, &[(b"name", name), (b"class", class)]);
    obj
}

/// A declared property of `class` or an ancestor: (declaring class, visibility, is_static).
fn property_of(vm: &VM, class: Symbol, prop: Symbol) -> Option<(Symbol, Visibility, bool)> {
    vm.walk_inheritance_chain(class, |def, cls| {
        if let Some(entry) = def.properties.get(&prop) {
            return Some((cls, entry.visibility, false));
        }
        def.static_properties.get(&prop).map(|entry| (cls, entry.visibility, true))
    })
}

fn reflection_property_for(vm: &mut VM, class: Symbol, prop: Symbol, is_static: bool) -> Handle {
    let obj = new_reflection(
        vm,
        b"ReflectionProperty",
        ReflectionPropertyData {
            class_name: class,
            prop_name: prop,
            is_static,
        },
    );
    let (name, class) = (name_of(vm, prop), name_of(vm, class));
    set_props_on(vm, obj, &[(b"name", name), (b"class", class)]);
    obj
}

/// Set the public string properties (`name`, `class`) a reflection object exposes.
fn set_props(vm: &mut VM, payload: Handle, props: &[(&[u8], Vec<u8>)]) {
    for (key, value) in props {
        let key = vm.context.interner.intern(key);
        let value = vm.arena.alloc(Val::String(Rc::new(value.clone())));
        if let Val::ObjPayload(obj) = &mut vm.arena.get_mut(payload).value {
            obj.properties.insert(key, value);
        }
    }
}

fn set_props_on(vm: &mut VM, obj: Handle, props: &[(&[u8], Vec<u8>)]) {
    if let Val::Object(payload) = vm.arena.get(obj).value {
        set_props(vm, payload, props);
    }
}

fn visibility_bits(vis: Visibility) -> i64 {
    match vis {
        Visibility::Public => IS_PUBLIC,
        Visibility::Protected => IS_PROTECTED,
        Visibility::Private => IS_PRIVATE,
    }
}

fn array_of(vm: &mut VM, items: Vec<Handle>) -> Handle {
    let mut arr = ArrayData::new();
    for item in items {
        arr.push(item);
    }
    vm.arena.alloc(Val::Array(Rc::new(arr)))
}

fn callback_err(vm: &mut VM, e: crate::vm::engine::VmError) -> String {
    vm.callback_error(e)
}

// ---------------------------------------------------------------- ReflectionClass

macro_rules! class_data {
    ($vm:expr, $method:expr) => {
        this_data::<ReflectionClassData>($vm, $method)?.class_name
    };
}

pub fn reflection_object_construct(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let obj = *args.first().ok_or("ReflectionObject::__construct() expects exactly 1 argument")?;
    let Some((class, _)) = object_class(vm, obj) else {
        return Err(vm.throw_from_native(
            b"TypeError",
            "ReflectionObject::__construct(): Argument #1 ($object) must be of type object",
        ));
    };
    set_this_data(vm, "ReflectionObject::__construct()", ReflectionClassData { class_name: class })?;
    let this = get_this_handle(vm, "ReflectionObject::__construct()")?;
    let name = name_of(vm, class);
    set_props_on(vm, this, &[(b"name", name)]);
    Ok(vm.arena.alloc(Val::Null))
}

pub fn reflection_class_get_short_name(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let class = class_data!(vm, "ReflectionClass::getShortName()");
    let name = name_of(vm, class);
    let short = name.rsplit(|b| *b == b'\\').next().unwrap_or(&name).to_vec();
    Ok(string_handle(vm, &short))
}

pub fn reflection_class_get_namespace_name(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let class = class_data!(vm, "ReflectionClass::getNamespaceName()");
    let name = name_of(vm, class);
    let ns = name.iter().rposition(|b| *b == b'\\').map(|i| name[..i].to_vec()).unwrap_or_default();
    Ok(string_handle(vm, &ns))
}

pub fn reflection_class_in_namespace(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let class = class_data!(vm, "ReflectionClass::inNamespace()");
    let has = name_of(vm, class).contains(&b'\\');
    Ok(bool_handle(vm, has))
}

fn class_flag(vm: &mut VM, method: &str, f: impl Fn(&VM, &ClassDef) -> bool) -> Result<Handle, String> {
    let class = this_data::<ReflectionClassData>(vm, method)?.class_name;
    let value = f(vm, class_of(vm, class)?);
    Ok(bool_handle(vm, value))
}

pub fn reflection_class_is_interface(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    class_flag(vm, "ReflectionClass::isInterface()", |_, d| d.is_interface)
}

pub fn reflection_class_is_trait(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    class_flag(vm, "ReflectionClass::isTrait()", |_, d| d.is_trait)
}

pub fn reflection_class_is_enum(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    class_flag(vm, "ReflectionClass::isEnum()", |_, d| d.is_enum)
}

pub fn reflection_class_is_abstract(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    class_flag(vm, "ReflectionClass::isAbstract()", |_, d| {
        d.is_abstract || (d.is_interface && !d.methods.is_empty()) || !d.abstract_methods.is_empty()
    })
}

pub fn reflection_class_is_final(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    class_flag(vm, "ReflectionClass::isFinal()", |_, d| d.is_final)
}

pub fn reflection_class_is_internal(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    class_flag(vm, "ReflectionClass::isInternal()", |vm, d| {
        vm.context.engine.registry.get_class(&name_of(vm, d.name)).is_some()
    })
}

pub fn reflection_class_is_user_defined(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    class_flag(vm, "ReflectionClass::isUserDefined()", |vm, d| {
        vm.context.engine.registry.get_class(&name_of(vm, d.name)).is_none()
    })
}

pub fn reflection_class_is_anonymous(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    class_flag(vm, "ReflectionClass::isAnonymous()", |vm, d| {
        name_of(vm, d.name).windows(10).any(|w| w == b"@anonymous")
    })
}

pub fn reflection_class_is_instantiable(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    class_flag(vm, "ReflectionClass::isInstantiable()", |vm, d| {
        if d.is_interface || d.is_trait || d.is_enum || d.is_abstract || !d.abstract_methods.is_empty() {
            return false;
        }
        let ctor = vm.context.interner.find(b"__construct");
        match ctor.and_then(|c| vm.walk_inheritance_chain(d.name, |def, _| def.methods.get(&c).cloned())) {
            Some(entry) => entry.visibility == Visibility::Public,
            None => true,
        }
    })
}

pub fn reflection_class_is_cloneable(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    class_flag(vm, "ReflectionClass::isCloneable()", |vm, d| {
        if d.is_interface || d.is_trait || d.is_enum || d.is_abstract {
            return false;
        }
        let clone = vm.context.interner.find(b"__clone");
        match clone.and_then(|c| vm.walk_inheritance_chain(d.name, |def, _| def.methods.get(&c).cloned())) {
            Some(entry) => entry.visibility == Visibility::Public,
            None => true,
        }
    })
}

pub fn reflection_class_get_modifiers(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let class = class_data!(vm, "ReflectionClass::getModifiers()");
    let def = class_of(vm, class)?;
    let mut bits = 0;
    if d_abstract(def) {
        bits |= IS_ABSTRACT;
    }
    if def.is_final {
        bits |= IS_FINAL;
    }
    Ok(vm.arena.alloc(Val::Int(bits)))
}

fn d_abstract(def: &ClassDef) -> bool {
    def.is_abstract && !def.is_interface
}

pub fn reflection_class_get_parent_class(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let class = class_data!(vm, "ReflectionClass::getParentClass()");
    match class_of(vm, class)?.parent {
        Some(parent) => Ok(reflection_class_for(vm, parent)),
        None => Ok(bool_handle(vm, false)),
    }
}

pub fn reflection_class_is_subclass_of(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let class = class_data!(vm, "ReflectionClass::isSubclassOf()");
    let other = class_arg(vm, *args.first().ok_or("ReflectionClass::isSubclassOf() expects 1 argument")?)?;
    let result = class != other && vm.is_subclass_of(class, other);
    Ok(bool_handle(vm, result))
}

pub fn reflection_class_implements_interface(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let class = class_data!(vm, "ReflectionClass::implementsInterface()");
    let iface = class_arg(vm, *args.first().ok_or("ReflectionClass::implementsInterface() expects 1 argument")?)?;
    if !class_of(vm, iface)?.is_interface {
        let msg = format!("{} is not an interface", String::from_utf8_lossy(&name_of(vm, iface)));
        return Err(reflection_error(vm, &msg));
    }
    let result = vm.is_subclass_of(class, iface);
    Ok(bool_handle(vm, result))
}

pub fn reflection_class_is_instance(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let class = class_data!(vm, "ReflectionClass::isInstance()");
    let obj = *args.first().ok_or("ReflectionClass::isInstance() expects 1 argument")?;
    let result = object_class(vm, obj).is_some_and(|(c, _)| vm.is_subclass_of(c, class));
    Ok(bool_handle(vm, result))
}

/// All interfaces `class` implements, including inherited ones, each once.
fn interfaces_of(vm: &VM, class: Symbol) -> Vec<Symbol> {
    fn walk(vm: &VM, class: Symbol, out: &mut Vec<Symbol>) {
        let Some(def) = vm.context.classes.get(&class) else { return };
        if let Some(parent) = def.parent {
            walk(vm, parent, out);
        }
        for &iface in &def.interfaces {
            walk(vm, iface, out);
            if !out.contains(&iface) {
                out.push(iface);
            }
        }
    }
    let mut out = Vec::new();
    walk(vm, class, &mut out);
    out
}

pub fn reflection_class_get_interface_names(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let class = class_data!(vm, "ReflectionClass::getInterfaceNames()");
    let names: Vec<Vec<u8>> = interfaces_of(vm, class).into_iter().map(|i| name_of(vm, i)).collect();
    let items = names.iter().map(|n| string_handle(vm, n)).collect();
    Ok(array_of(vm, items))
}

pub fn reflection_class_get_interfaces(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let class = class_data!(vm, "ReflectionClass::getInterfaces()");
    let mut arr = ArrayData::new();
    for iface in interfaces_of(vm, class) {
        let name = name_of(vm, iface);
        let refl = reflection_class_for(vm, iface);
        arr.insert(ArrayKey::Str(Rc::new(name)), refl);
    }
    Ok(vm.arena.alloc(Val::Array(Rc::new(arr))))
}

/// Methods of `class` in PHP's order: declared in the class first (source
/// order), then inherited ones.
fn methods_in_order(vm: &VM, class: Symbol) -> Vec<crate::runtime::context::MethodEntry> {
    let mut chain = Vec::new();
    let mut cur = Some(class);
    while let Some(c) = cur {
        chain.push(c);
        cur = vm.context.classes.get(&c).and_then(|d| d.parent);
    }
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (depth, cls) in chain.iter().enumerate() {
        let Some(def) = vm.context.classes.get(cls) else { continue };
        let mut mine: Vec<_> = def
            .methods
            .iter()
            .filter(|(_, m)| depth == 0 || m.declaring_class == *cls || m.visibility != Visibility::Private)
            .map(|(k, m)| (*k, m.clone()))
            .collect();
        // methods declared by this class first, each group in declaration order
        mine.sort_by_key(|(_, m)| (m.declaring_class != *cls, m.decl_order));
        for (key, m) in mine {
            if seen.insert(key) {
                out.push(m);
            }
        }
    }
    out
}

pub fn reflection_class_get_methods(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let class = class_data!(vm, "ReflectionClass::getMethods()");
    let filter = args.first().map(|h| vm.arena.get(*h).value.to_int());
    let mut items = Vec::new();
    for m in methods_in_order(vm, class) {
        let mut bits = visibility_bits(m.visibility);
        if m.is_static {
            bits |= IS_STATIC;
        }
        if m.is_abstract {
            bits |= IS_ABSTRACT;
        }
        if filter.is_none_or(|f| f & bits != 0) {
            let refl = reflection_method_for(vm, m.declaring_class, &m);
            items.push(refl);
        }
    }
    Ok(array_of(vm, items))
}

pub fn reflection_class_get_method(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let class = class_data!(vm, "ReflectionClass::getMethod()");
    let name = vm.arena.get(*args.first().ok_or("ReflectionClass::getMethod() expects 1 argument")?).value.to_php_string_bytes();
    match method_of(vm, class, &name) {
        Some(entry) => Ok(reflection_method_for(vm, entry.declaring_class, &entry)),
        None => {
            let msg = format!(
                "Method {}::{}() does not exist",
                String::from_utf8_lossy(&name_of(vm, class)),
                String::from_utf8_lossy(&name)
            );
            Err(reflection_error(vm, &msg))
        }
    }
}

pub fn reflection_class_has_method(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let class = class_data!(vm, "ReflectionClass::hasMethod()");
    let name = vm.arena.get(*args.first().ok_or("ReflectionClass::hasMethod() expects 1 argument")?).value.to_php_string_bytes();
    let has = method_of(vm, class, &name).is_some();
    Ok(bool_handle(vm, has))
}

pub fn reflection_class_get_constructor(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let class = class_data!(vm, "ReflectionClass::getConstructor()");
    match method_of(vm, class, b"__construct") {
        Some(entry) => Ok(reflection_method_for(vm, entry.declaring_class, &entry)),
        None => Ok(vm.arena.alloc(Val::Null)),
    }
}

/// Declared properties of `class`, own first, then inherited non-private ones.
fn properties_in_order(vm: &VM, class: Symbol) -> Vec<(Symbol, Symbol, Visibility, bool, bool)> {
    let mut out: Vec<(Symbol, Symbol, Visibility, bool, bool)> = Vec::new();
    let mut cur = Some(class);
    let mut depth = 0;
    while let Some(c) = cur {
        let Some(def) = vm.context.classes.get(&c) else { break };
        // instance and static properties together, in declaration order
        let mut props: Vec<(u64, Symbol, Visibility, bool, bool)> = def
            .properties
            .iter()
            .map(|(k, e)| (e.decl_order, *k, e.visibility, false, e.is_readonly))
            .chain(def.static_properties.iter().map(|(k, e)| (e.decl_order, *k, e.visibility, true, false)))
            .collect();
        props.sort_by_key(|p| p.0);
        for (_, name, vis, is_static, readonly) in props {
            if (depth == 0 || vis != Visibility::Private) && !out.iter().any(|p| p.0 == name) {
                out.push((name, c, vis, is_static, readonly));
            }
        }
        cur = def.parent;
        depth += 1;
    }
    out
}

pub fn reflection_class_get_properties(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let class = class_data!(vm, "ReflectionClass::getProperties()");
    let filter = args.first().map(|h| vm.arena.get(*h).value.to_int());
    let mut items = Vec::new();
    for (name, declaring, vis, is_static, readonly) in properties_in_order(vm, class) {
        let mut bits = visibility_bits(vis);
        if is_static {
            bits |= IS_STATIC;
        }
        if readonly {
            bits |= IS_READONLY;
        }
        if filter.is_none_or(|f| f & bits != 0) {
            let refl = reflection_property_for(vm, declaring, name, is_static);
            items.push(refl);
        }
    }
    Ok(array_of(vm, items))
}

pub fn reflection_class_get_property(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let class = class_data!(vm, "ReflectionClass::getProperty()");
    let name = vm.arena.get(*args.first().ok_or("ReflectionClass::getProperty() expects 1 argument")?).value.to_php_string_bytes();
    let prop = vm.context.interner.intern(&name);
    match property_of(vm, class, prop) {
        Some((declaring, _, is_static)) => Ok(reflection_property_for(vm, declaring, prop, is_static)),
        None => {
            let msg = format!(
                "Property {}::${} does not exist",
                String::from_utf8_lossy(&name_of(vm, class)),
                String::from_utf8_lossy(&name)
            );
            Err(reflection_error(vm, &msg))
        }
    }
}

pub fn reflection_class_has_property(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let class = class_data!(vm, "ReflectionClass::hasProperty()");
    let name = vm.arena.get(*args.first().ok_or("ReflectionClass::hasProperty() expects 1 argument")?).value.to_php_string_bytes();
    let prop = vm.context.interner.intern(&name);
    let has = property_of(vm, class, prop).is_some();
    Ok(bool_handle(vm, has))
}

/// A class constant's value, walking parents and interfaces.
fn constant_of(vm: &VM, class: Symbol, name: Symbol) -> Option<Val> {
    let def = vm.context.classes.get(&class)?;
    if let Some(entry) = def.constants.get(&name) {
        return Some(entry.value.clone());
    }
    if let Some(parent) = def.parent
        && let Some(v) = constant_of(vm, parent, name)
    {
        return Some(v);
    }
    def.interfaces.iter().find_map(|i| constant_of(vm, *i, name))
}

pub fn reflection_class_has_constant(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let class = class_data!(vm, "ReflectionClass::hasConstant()");
    let name = vm.arena.get(*args.first().ok_or("ReflectionClass::hasConstant() expects 1 argument")?).value.to_php_string_bytes();
    let sym = vm.context.interner.intern(&name);
    let has = constant_of(vm, class, sym).is_some();
    Ok(bool_handle(vm, has))
}

pub fn reflection_class_get_constant(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let class = class_data!(vm, "ReflectionClass::getConstant()");
    let name = vm.arena.get(*args.first().ok_or("ReflectionClass::getConstant() expects 1 argument")?).value.to_php_string_bytes();
    let sym = vm.context.interner.intern(&name);
    match constant_of(vm, class, sym) {
        Some(v) => Ok(alloc_val(vm, &v)),
        None => Ok(bool_handle(vm, false)),
    }
}

pub fn reflection_class_get_constants(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let class = class_data!(vm, "ReflectionClass::getConstants()");
    let mut names: Vec<Symbol> = Vec::new();
    let mut cur = Some(class);
    while let Some(c) = cur {
        let Some(def) = vm.context.classes.get(&c) else { break };
        let mut own: Vec<Symbol> = def.constants.keys().copied().collect();
        own.sort_by_key(|k| k.0);
        for k in own {
            if !names.contains(&k) {
                names.push(k);
            }
        }
        cur = def.parent;
    }
    for iface in interfaces_of(vm, class) {
        if let Some(def) = vm.context.classes.get(&iface) {
            let mut own: Vec<Symbol> = def.constants.keys().copied().collect();
            own.sort_by_key(|k| k.0);
            for k in own {
                if !names.contains(&k) {
                    names.push(k);
                }
            }
        }
    }
    let mut arr = ArrayData::new();
    for name in names {
        if let Some(v) = constant_of(vm, class, name) {
            let key = name_of(vm, name);
            let h = alloc_val(vm, &v);
            arr.insert(ArrayKey::Str(Rc::new(key)), h);
        }
    }
    Ok(vm.arena.alloc(Val::Array(Rc::new(arr))))
}

pub fn reflection_class_new_instance(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let class = class_data!(vm, "ReflectionClass::newInstance()");
    vm.instantiate_class(class, args)
}

pub fn reflection_class_new_instance_args(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let class = class_data!(vm, "ReflectionClass::newInstanceArgs()");
    let ctor_args: Vec<Handle> = match args.first().map(|h| vm.arena.get(*h).value.clone()) {
        Some(Val::Array(arr)) => arr.map.values().copied().collect(),
        _ => Vec::new(),
    };
    vm.instantiate_class(class, &ctor_args)
}

pub fn reflection_class_new_instance_without_constructor(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let class = class_data!(vm, "ReflectionClass::newInstanceWithoutConstructor()");
    let properties = vm.collect_properties(class, crate::vm::engine::PropertyCollectionMode::All);
    let payload = vm.arena.alloc(Val::ObjPayload(ObjectData {
        class,
        properties,
        internal: None,
        dynamic_properties: std::collections::HashSet::new(),
        id: 0,
    }));
    Ok(vm.arena.alloc(Val::Object(payload)))
}

pub fn reflection_class_get_static_properties(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let class = class_data!(vm, "ReflectionClass::getStaticProperties()");
    let mut arr = ArrayData::new();
    for (name, _, _, is_static, _) in properties_in_order(vm, class) {
        if is_static
            && let Ok((v, _, _)) = vm.find_static_prop(class, name)
        {
            let key = name_of(vm, name);
            let h = alloc_val(vm, &v);
            arr.insert(ArrayKey::Str(Rc::new(key)), h);
        }
    }
    Ok(vm.arena.alloc(Val::Array(Rc::new(arr))))
}

pub fn reflection_class_get_static_property_value(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let class = class_data!(vm, "ReflectionClass::getStaticPropertyValue()");
    let name = vm.arena.get(*args.first().ok_or("ReflectionClass::getStaticPropertyValue() expects 1 argument")?).value.to_php_string_bytes();
    let prop = vm.context.interner.intern(&name);
    let found = vm.walk_inheritance_chain(class, |def, cls| def.static_properties.get(&prop).map(|e| (cls, e.handle)));
    match (found, args.get(1)) {
        (Some((_, h)), _) => {
            let v = vm.arena.get(h).value.clone();
            Ok(vm.arena.alloc(v))
        }
        (None, Some(default)) => Ok(*default),
        (None, None) => {
            let msg = format!(
                "Property {}::${} does not exist",
                String::from_utf8_lossy(&name_of(vm, class)),
                String::from_utf8_lossy(&name)
            );
            Err(reflection_error(vm, &msg))
        }
    }
}

pub fn reflection_class_set_static_property_value(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let class = class_data!(vm, "ReflectionClass::setStaticPropertyValue()");
    if args.len() < 2 {
        return Err("ReflectionClass::setStaticPropertyValue() expects 2 arguments".into());
    }
    let name = vm.arena.get(args[0]).value.to_php_string_bytes();
    let prop = vm.context.interner.intern(&name);
    let found = vm.walk_inheritance_chain(class, |def, cls| def.static_properties.get(&prop).map(|_| cls));
    match found {
        Some(declaring) => {
            let v = vm.arena.get(args[1]).value.clone();
            vm.write_static_prop(declaring, prop, v);
            Ok(vm.arena.alloc(Val::Null))
        }
        None => {
            let msg = format!(
                "Class {} does not have a property named {}",
                String::from_utf8_lossy(&name_of(vm, class)),
                String::from_utf8_lossy(&name)
            );
            Err(reflection_error(vm, &msg))
        }
    }
}

pub fn reflection_class_get_doc_comment(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    // Doc comments are not kept by the compiler yet.
    Ok(bool_handle(vm, false))
}

// ------------------------------------------------------------- ReflectionProperty

fn property_data(vm: &VM, method: &str) -> Result<ReflectionPropertyData, String> {
    this_data::<ReflectionPropertyData>(vm, method)
}

/// Visibility and readonly flag of the property `data` refers to.
fn property_info(vm: &VM, data: &ReflectionPropertyData) -> (Visibility, bool) {
    vm.walk_inheritance_chain(data.class_name, |def, _| {
        if let Some(e) = def.properties.get(&data.prop_name) {
            return Some((e.visibility, e.is_readonly));
        }
        def.static_properties.get(&data.prop_name).map(|e| (e.visibility, false))
    })
    .unwrap_or((Visibility::Public, false))
}

macro_rules! property_flag {
    ($fn:ident, $method:expr, $pred:expr) => {
        pub fn $fn(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
            let data = property_data(vm, $method)?;
            let (vis, readonly) = property_info(vm, &data);
            let result = $pred(vis, readonly, data.is_static);
            Ok(bool_handle(vm, result))
        }
    };
}

property_flag!(reflection_property_is_public, "ReflectionProperty::isPublic()", |v, _, _| v == Visibility::Public);
property_flag!(reflection_property_is_protected, "ReflectionProperty::isProtected()", |v, _, _| v == Visibility::Protected);
property_flag!(reflection_property_is_private, "ReflectionProperty::isPrivate()", |v, _, _| v == Visibility::Private);
property_flag!(reflection_property_is_static, "ReflectionProperty::isStatic()", |_, _, s| s);
property_flag!(reflection_property_is_readonly, "ReflectionProperty::isReadOnly()", |_, r, _| r);
property_flag!(reflection_property_is_default, "ReflectionProperty::isDefault()", |_, _, _| true);

pub fn reflection_property_get_modifiers(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let data = property_data(vm, "ReflectionProperty::getModifiers()")?;
    let (vis, readonly) = property_info(vm, &data);
    let mut bits = visibility_bits(vis);
    if data.is_static {
        bits |= IS_STATIC;
    }
    if readonly {
        bits |= IS_READONLY;
    }
    Ok(vm.arena.alloc(Val::Int(bits)))
}

pub fn reflection_property_get_declaring_class(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let data = property_data(vm, "ReflectionProperty::getDeclaringClass()")?;
    let declaring = property_of(vm, data.class_name, data.prop_name).map_or(data.class_name, |p| p.0);
    Ok(reflection_class_for(vm, declaring))
}

pub fn reflection_property_set_accessible(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    // A no-op since PHP 8.1: reflection can always access the property.
    Ok(vm.arena.alloc(Val::Null))
}

pub fn reflection_property_get_value(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let data = property_data(vm, "ReflectionProperty::getValue()")?;
    if data.is_static {
        let (v, _, _) = vm.find_static_prop(data.class_name, data.prop_name).map_err(|e| format!("{:?}", e))?;
        return Ok(vm.arena.alloc(v));
    }
    let obj = *args.first().ok_or("ReflectionProperty::getValue() expects an object")?;
    let Some((_, payload)) = object_class(vm, obj) else {
        return Err(vm.throw_from_native(b"TypeError", "ReflectionProperty::getValue(): Argument #1 ($object) must be provided for instance properties"));
    };
    let handle = match &vm.arena.get(payload).value {
        Val::ObjPayload(o) => o.properties.get(&data.prop_name).copied(),
        _ => None,
    };
    match handle {
        Some(h) if matches!(vm.arena.get(h).value, Val::Uninitialized) => {
            let msg = format!(
                "Typed property {}::${} must not be accessed before initialization",
                String::from_utf8_lossy(&name_of(vm, data.class_name)),
                String::from_utf8_lossy(&name_of(vm, data.prop_name))
            );
            Err(vm.throw_from_native(b"Error", &msg))
        }
        Some(h) => {
            let v = vm.arena.get(h).value.clone();
            Ok(vm.arena.alloc(v))
        }
        None => Ok(vm.arena.alloc(Val::Null)),
    }
}

pub fn reflection_property_set_value(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let data = property_data(vm, "ReflectionProperty::setValue()")?;
    if data.is_static {
        let value = *args.last().ok_or("ReflectionProperty::setValue() expects a value")?;
        let declaring = property_of(vm, data.class_name, data.prop_name).map_or(data.class_name, |p| p.0);
        let v = vm.arena.get(value).value.clone();
        vm.write_static_prop(declaring, data.prop_name, v);
        return Ok(vm.arena.alloc(Val::Null));
    }
    if args.len() < 2 {
        return Err("ReflectionProperty::setValue() expects 2 arguments".into());
    }
    let Some((_, payload)) = object_class(vm, args[0]) else {
        return Err(vm.throw_from_native(b"TypeError", "ReflectionProperty::setValue(): Argument #1 ($objectOrValue) must be of type object"));
    };
    let v = vm.arena.get(args[1]).value.clone();
    let h = vm.arena.alloc(v);
    if let Val::ObjPayload(o) = &mut vm.arena.get_mut(payload).value {
        o.properties.insert(data.prop_name, h);
    }
    Ok(vm.arena.alloc(Val::Null))
}

pub fn reflection_property_is_initialized(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let data = property_data(vm, "ReflectionProperty::isInitialized()")?;
    if data.is_static {
        return Ok(bool_handle(vm, true));
    }
    let obj = *args.first().ok_or("ReflectionProperty::isInitialized() expects an object")?;
    let init = object_class(vm, obj)
        .and_then(|(_, payload)| match &vm.arena.get(payload).value {
            Val::ObjPayload(o) => o.properties.get(&data.prop_name).copied(),
            _ => None,
        })
        .is_some_and(|h| !matches!(vm.arena.get(h).value, Val::Uninitialized));
    Ok(bool_handle(vm, init))
}

pub fn reflection_property_has_default_value(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let data = property_data(vm, "ReflectionProperty::hasDefaultValue()")?;
    let has = vm
        .walk_inheritance_chain(data.class_name, |def, _| def.properties.get(&data.prop_name).map(|e| e.default_value.clone()))
        .is_some_and(|v| !matches!(v, Val::Uninitialized));
    let has = has || data.is_static;
    Ok(bool_handle(vm, has))
}

pub fn reflection_property_get_default_value(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let data = property_data(vm, "ReflectionProperty::getDefaultValue()")?;
    let v = vm
        .walk_inheritance_chain(data.class_name, |def, _| def.properties.get(&data.prop_name).map(|e| e.default_value.clone()))
        .filter(|v| !matches!(v, Val::Uninitialized))
        .unwrap_or(Val::Null);
    Ok(alloc_val(vm, &v))
}

pub fn reflection_property_get_doc_comment(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(bool_handle(vm, false))
}

// --------------------------------------------------------------- ReflectionMethod

fn method_data(vm: &VM, method: &str) -> Result<(ReflectionMethodData, crate::runtime::context::MethodEntry), String> {
    let data = this_data::<ReflectionMethodData>(vm, method)?;
    let name = name_of(vm, data.method_name);
    let entry = method_of(vm, data.class_name, &name).ok_or_else(|| format!("{method}: method not found"))?;
    Ok((data, entry))
}

macro_rules! method_flag {
    ($fn:ident, $method:expr, $pred:expr) => {
        pub fn $fn(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
            let (_, entry) = method_data(vm, $method)?;
            let result = $pred(vm, &entry);
            Ok(bool_handle(vm, result))
        }
    };
}

method_flag!(reflection_method_is_public, "ReflectionMethod::isPublic()", |_: &VM, e: &crate::runtime::context::MethodEntry| e.visibility == Visibility::Public);
method_flag!(reflection_method_is_protected, "ReflectionMethod::isProtected()", |_: &VM, e: &crate::runtime::context::MethodEntry| e.visibility == Visibility::Protected);
method_flag!(reflection_method_is_private, "ReflectionMethod::isPrivate()", |_: &VM, e: &crate::runtime::context::MethodEntry| e.visibility == Visibility::Private);
method_flag!(reflection_method_is_static, "ReflectionMethod::isStatic()", |_: &VM, e: &crate::runtime::context::MethodEntry| e.is_static);
method_flag!(reflection_method_is_abstract, "ReflectionMethod::isAbstract()", |_: &VM, e: &crate::runtime::context::MethodEntry| e.is_abstract);
method_flag!(reflection_method_is_constructor, "ReflectionMethod::isConstructor()", |vm: &VM, e: &crate::runtime::context::MethodEntry| name_of(vm, e.name).eq_ignore_ascii_case(b"__construct"));
method_flag!(reflection_method_is_destructor, "ReflectionMethod::isDestructor()", |vm: &VM, e: &crate::runtime::context::MethodEntry| name_of(vm, e.name).eq_ignore_ascii_case(b"__destruct"));
method_flag!(reflection_method_is_user_defined, "ReflectionMethod::isUserDefined()", |_: &VM, _e: &crate::runtime::context::MethodEntry| true);
method_flag!(reflection_method_is_internal, "ReflectionMethod::isInternal()", |_: &VM, _e: &crate::runtime::context::MethodEntry| false);
method_flag!(reflection_method_is_variadic, "ReflectionMethod::isVariadic()", |_: &VM, e: &crate::runtime::context::MethodEntry| e.func.params.iter().any(|p| p.is_variadic));
method_flag!(reflection_method_is_generator, "ReflectionMethod::isGenerator()", |_: &VM, e: &crate::runtime::context::MethodEntry| e.func.is_generator);

pub fn reflection_method_get_modifiers(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let (_, entry) = method_data(vm, "ReflectionMethod::getModifiers()")?;
    let mut bits = visibility_bits(entry.visibility);
    if entry.is_static {
        bits |= IS_STATIC;
    }
    if entry.is_abstract {
        bits |= IS_ABSTRACT;
    }
    Ok(vm.arena.alloc(Val::Int(bits)))
}

pub fn reflection_method_get_declaring_class(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let (_, entry) = method_data(vm, "ReflectionMethod::getDeclaringClass()")?;
    Ok(reflection_class_for(vm, entry.declaring_class))
}

pub fn reflection_method_get_number_of_parameters(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let (_, entry) = method_data(vm, "ReflectionMethod::getNumberOfParameters()")?;
    Ok(vm.arena.alloc(Val::Int(entry.func.params.len() as i64)))
}

pub fn reflection_method_get_number_of_required_parameters(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let (_, entry) = method_data(vm, "ReflectionMethod::getNumberOfRequiredParameters()")?;
    Ok(vm.arena.alloc(Val::Int(required_params(&entry.func.params) as i64)))
}

/// Parameters before the last one without a default (variadics are optional).
fn required_params(params: &[crate::compiler::chunk::FuncParam]) -> usize {
    params
        .iter()
        .rposition(|p| p.default_value.is_none() && !p.is_variadic)
        .map_or(0, |i| i + 1)
}

pub fn reflection_method_invoke(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let (data, entry) = method_data(vm, "ReflectionMethod::invoke()")?;
    let obj = args.first().copied();
    invoke_method(vm, &data, &entry, obj, args.get(1..).unwrap_or(&[]))
}

pub fn reflection_method_invoke_args(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let (data, entry) = method_data(vm, "ReflectionMethod::invokeArgs()")?;
    let obj = args.first().copied();
    let call_args: Vec<Handle> = match args.get(1).map(|h| vm.arena.get(*h).value.clone()) {
        Some(Val::Array(arr)) => arr.map.values().copied().collect(),
        _ => Vec::new(),
    };
    invoke_method(vm, &data, &entry, obj, &call_args)
}

fn invoke_method(
    vm: &mut VM,
    data: &ReflectionMethodData,
    entry: &crate::runtime::context::MethodEntry,
    obj: Option<Handle>,
    args: &[Handle],
) -> Result<Handle, String> {
    if !entry.is_static {
        let Some(obj) = obj.filter(|o| object_class(vm, *o).is_some()) else {
            let msg = format!(
                "Trying to invoke non static method {}::{}() without an object",
                String::from_utf8_lossy(&name_of(vm, entry.declaring_class)),
                String::from_utf8_lossy(&name_of(vm, entry.name))
            );
            return Err(reflection_error(vm, &msg));
        };
        return vm.call_method(obj, entry.name, args).map_err(|e| callback_err(vm, e));
    }
    // Static: call through a [Class, method] callable.
    let (class, method) = (name_of(vm, data.class_name), name_of(vm, entry.name));
    let class = string_handle(vm, &class);
    let method = string_handle(vm, &method);
    let callable = array_of(vm, vec![class, method]);
    vm.call_callable(callable, args.iter().copied().collect()).map_err(|e| callback_err(vm, e))
}

pub fn reflection_method_set_accessible(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(vm.arena.alloc(Val::Null))
}

pub fn reflection_method_get_doc_comment(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(bool_handle(vm, false))
}

// ------------------------------------------------------------- ReflectionFunction

fn function_of(vm: &VM, method: &str) -> Result<Rc<crate::compiler::chunk::UserFunc>, String> {
    let data = this_data::<ReflectionFunctionData>(vm, method)?;
    vm.context
        .user_functions
        .get(&data.func_name)
        .cloned()
        .ok_or_else(|| format!("{method}: function not found"))
}

pub fn reflection_function_get_number_of_parameters(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let func = function_of(vm, "ReflectionFunction::getNumberOfParameters()")?;
    Ok(vm.arena.alloc(Val::Int(func.params.len() as i64)))
}

pub fn reflection_function_get_number_of_required_parameters(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let func = function_of(vm, "ReflectionFunction::getNumberOfRequiredParameters()")?;
    Ok(vm.arena.alloc(Val::Int(required_params(&func.params) as i64)))
}

pub fn reflection_function_is_variadic(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let func = function_of(vm, "ReflectionFunction::isVariadic()")?;
    let v = func.params.iter().any(|p| p.is_variadic);
    Ok(bool_handle(vm, v))
}

pub fn reflection_function_is_generator(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let func = function_of(vm, "ReflectionFunction::isGenerator()")?;
    Ok(bool_handle(vm, func.is_generator))
}

pub fn reflection_function_is_user_defined(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let user = function_of(vm, "ReflectionFunction::isUserDefined()").is_ok();
    Ok(bool_handle(vm, user))
}

pub fn reflection_function_is_internal(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let user = function_of(vm, "ReflectionFunction::isInternal()").is_ok();
    Ok(bool_handle(vm, !user))
}

pub fn reflection_function_invoke(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let data = this_data::<ReflectionFunctionData>(vm, "ReflectionFunction::invoke()")?;
    let name = name_of(vm, data.func_name);
    let callable = string_handle(vm, &name);
    vm.call_callable(callable, args.iter().copied().collect()).map_err(|e| callback_err(vm, e))
}

pub fn reflection_function_invoke_args(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let data = this_data::<ReflectionFunctionData>(vm, "ReflectionFunction::invokeArgs()")?;
    let call_args: smallvec::SmallVec<[Handle; 8]> = match args.first().map(|h| vm.arena.get(*h).value.clone()) {
        Some(Val::Array(arr)) => arr.map.values().copied().collect(),
        _ => smallvec::SmallVec::new(),
    };
    let name = name_of(vm, data.func_name);
    let callable = string_handle(vm, &name);
    vm.call_callable(callable, call_args).map_err(|e| callback_err(vm, e))
}

pub fn reflection_function_get_doc_comment(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(bool_handle(vm, false))
}

// ------------------------------------------------------------ ReflectionParameter

pub fn reflection_parameter_get_position(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let data = this_data::<ReflectionParameterData>(vm, "ReflectionParameter::getPosition()")?;
    Ok(vm.arena.alloc(Val::Int(data.position as i64)))
}

pub fn reflection_parameter_is_optional(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let data = this_data::<ReflectionParameterData>(vm, "ReflectionParameter::isOptional()")?;
    Ok(bool_handle(vm, data.is_optional))
}

pub fn reflection_parameter_is_default_value_available(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let data = this_data::<ReflectionParameterData>(vm, "ReflectionParameter::isDefaultValueAvailable()")?;
    Ok(bool_handle(vm, data.default_value.is_some()))
}

pub fn reflection_parameter_get_default_value(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let data = this_data::<ReflectionParameterData>(vm, "ReflectionParameter::getDefaultValue()")?;
    match data.default_value {
        Some(v) => Ok(alloc_val(vm, &v)),
        None => Err(reflection_error(vm, "Internal error: Failed to retrieve the default value")),
    }
}

pub fn reflection_parameter_allows_null(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let data = this_data::<ReflectionParameterData>(vm, "ReflectionParameter::allowsNull()")?;
    let allows = data.type_hint.as_ref().is_none_or(type_hint_allows_null);
    Ok(bool_handle(vm, allows))
}

// --------------------------------------------------------------------- Reflection

/// Reflection::getModifierNames(int $modifiers): the keywords for the bits set.
pub fn reflection_get_modifier_names(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let bits = args.first().map_or(0, |h| vm.arena.get(*h).value.to_int());
    let mut names: Vec<&[u8]> = Vec::new();
    if bits & IS_ABSTRACT != 0 {
        names.push(b"abstract");
    }
    if bits & IS_FINAL != 0 {
        names.push(b"final");
    }
    if bits & IS_PUBLIC != 0 {
        names.push(b"public");
    } else if bits & IS_PRIVATE != 0 {
        names.push(b"private");
    } else if bits & IS_PROTECTED != 0 {
        names.push(b"protected");
    }
    if bits & IS_STATIC != 0 {
        names.push(b"static");
    }
    if bits & IS_READONLY != 0 {
        names.push(b"readonly");
    }
    let items = names.iter().map(|n| string_handle(vm, n)).collect();
    Ok(array_of(vm, items))
}
