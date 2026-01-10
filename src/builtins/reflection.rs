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

fn create_reflection_attribute(vm: &mut VM, attr: &AttributeInstance) -> Result<Handle, String> {
    let class_sym = vm.context.interner.intern(b"ReflectionAttribute");
    let data = ReflectionAttributeData {
        name: attr.name,
        args: attr.args.clone(),
    };

    let obj_data = ObjectData {
        class: class_sym,
        properties: IndexMap::new(),
        internal: Some(Rc::new(data)),
        dynamic_properties: std::collections::HashSet::new(),
    };
    let payload_handle = vm.arena.alloc(Val::ObjPayload(obj_data));
    Ok(vm.arena.alloc(Val::Object(payload_handle)))
}

fn build_attribute_array(
    vm: &mut VM,
    attrs: &[AttributeInstance],
    filter: Option<&[u8]>,
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

        let attr_obj = create_reflection_attribute(vm, attr)?;
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
            };
            let payload_handle = vm.arena.alloc(Val::ObjPayload(obj_data));
            vm.arena.alloc(Val::Object(payload_handle))
        }
        TypeHint::Intersection(types) => {
            let data = ReflectionTypeData {
                kind: ReflectionTypeKind::Intersection(types),
            };
            let class_sym = vm.context.interner.intern(b"ReflectionIntersectionType");
            let obj_data = ObjectData {
                class: class_sym,
                properties: IndexMap::new(),
                internal: Some(Rc::new(data)),
                dynamic_properties: std::collections::HashSet::new(),
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

    let class_sym = resolve_class_symbol(vm, args[0])?;
    if !vm.context.classes.contains_key(&class_sym) {
        return Err("ReflectionClass::__construct(): Class does not exist".into());
    }

    let this_handle = get_this_handle(vm, "ReflectionClass::__construct()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionClass::__construct() expects object context".into()),
    };

    let data = ReflectionClassData {
        class_name: class_sym,
    };
    let payload = vm.arena.get_mut(payload_handle);
    if let Val::ObjPayload(ref mut obj_data) = payload.value {
        obj_data.internal = Some(Rc::new(data));
    }

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
    build_attribute_array(vm, &attributes, filter_name.as_deref())
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

pub fn reflection_attribute_get_arguments(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionAttribute::getArguments()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionAttribute::getArguments() expects object context".into()),
    };

    let payload = vm.arena.get(payload_handle);
    let data = if let Val::ObjPayload(obj_data) = &payload.value {
        if let Some(internal) = &obj_data.internal {
            internal.downcast_ref::<ReflectionAttributeData>().cloned()
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

pub fn reflection_attribute_new_instance(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionAttribute::newInstance()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionAttribute::newInstance() expects object context".into()),
    };

    let payload = vm.arena.get(payload_handle);
    let data = if let Val::ObjPayload(obj_data) = &payload.value {
        if let Some(internal) = &obj_data.internal {
            internal.downcast_ref::<ReflectionAttributeData>().cloned()
        } else {
            None
        }
    } else {
        None
    }
    .ok_or_else(|| {
        "ReflectionAttribute::newInstance() called on invalid reflection object".to_string()
    })?;

    let mut args = Vec::new();
    for arg in data.args {
        args.push(alloc_val(vm, &arg.value));
    }

    vm.instantiate_class(data.name, &args)
        .map_err(|err| format!("ReflectionAttribute::newInstance(): {err}"))
}

pub fn reflection_method_construct(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("ReflectionMethod::__construct() expects 2 arguments".into());
    }
    let class_sym = resolve_class_symbol(vm, args[0])?;
    let method_bytes = match &vm.arena.get(args[1]).value {
        Val::String(s) => s.clone(),
        _ => return Err("ReflectionMethod::__construct() expects method name string".into()),
    };
    let method_sym = vm.context.interner.intern(&method_bytes);
    let method_lookup = vm
        .context
        .interner
        .intern(&method_bytes.to_ascii_lowercase());

    let class_def = vm
        .context
        .classes
        .get(&class_sym)
        .ok_or("ReflectionMethod::__construct(): Class not found")?;
    if !class_def.methods.contains_key(&method_lookup) {
        return Err("ReflectionMethod::__construct(): Method not found".into());
    }

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
    .ok_or_else(|| {
        "ReflectionMethod::getAttributes() called on invalid reflection object".to_string()
    })?;

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

    build_attribute_array(vm, &attributes, filter_name.as_deref())
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
    for (idx, param) in params.iter().enumerate() {
        let param_obj = create_reflection_parameter(vm, param)?;
        array.insert(ArrayKey::Int(idx as i64), param_obj);
    }

    Ok(vm.arena.alloc(Val::Array(Rc::new(array))))
}

pub fn reflection_method_get_return_type(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
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
    let class_sym = resolve_class_symbol(vm, args[0])?;
    let prop_bytes = match &vm.arena.get(args[1]).value {
        Val::String(s) => s.clone(),
        _ => return Err("ReflectionProperty::__construct() expects property name string".into()),
    };
    let prop_bytes = if prop_bytes.starts_with(b"$") {
        prop_bytes[1..].to_vec()
    } else {
        prop_bytes.to_vec()
    };
    let prop_sym = vm.context.interner.intern(&prop_bytes);

    let class_def = vm
        .context
        .classes
        .get(&class_sym)
        .ok_or("ReflectionProperty::__construct(): Class not found")?;
    let is_static = if class_def.properties.contains_key(&prop_sym) {
        false
    } else if class_def.static_properties.contains_key(&prop_sym) {
        true
    } else {
        return Err("ReflectionProperty::__construct(): Property not found".into());
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

pub fn reflection_property_get_attributes(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
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
    .ok_or_else(|| {
        "ReflectionProperty::getAttributes() called on invalid reflection object".to_string()
    })?;

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

    build_attribute_array(vm, &attributes, filter_name.as_deref())
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
            );
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

    Ok(vm.arena.alloc(Val::Null))
}

pub fn reflection_class_const_get_name(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
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
            return Err(
                "ReflectionClassConstant::getAttributes() expects string name filter".into(),
            );
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

    build_attribute_array(vm, &attributes, filter_name.as_deref())
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
                return Ok(vm.arena.alloc(Val::Bool(data.type_hint.is_some())));
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

pub fn reflection_parameter_is_variadic(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
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
        _ => {
            return Err("ReflectionParameter::isPassedByReference() expects object context".into());
        }
    };

    let payload = vm.arena.get(payload_handle);
    if let Val::ObjPayload(obj_data) = &payload.value {
        if let Some(internal) = &obj_data.internal {
            if let Some(data) = internal.downcast_ref::<ReflectionParameterData>() {
                return Ok(vm.arena.alloc(Val::Bool(data.is_by_ref)));
            }
        }
    }
    Err("ReflectionParameter::isPassedByReference() called on invalid reflection object".into())
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

pub fn reflection_named_type_allows_null(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
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

pub fn reflection_named_type_is_builtin(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
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

pub fn reflection_union_type_get_types(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
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

pub fn reflection_union_type_allows_null(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
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
    param: &crate::runtime::context::ParameterInfo,
) -> Result<Handle, String> {
    let data = ReflectionParameterData {
        name: param.name,
        type_hint: param.type_hint.clone(),
        is_variadic: param.is_variadic,
        is_by_ref: param.is_reference,
    };

    let class_sym = vm.context.interner.intern(b"ReflectionParameter");
    let obj_data = ObjectData {
        class: class_sym,
        properties: IndexMap::new(),
        internal: Some(Rc::new(data)),
        dynamic_properties: std::collections::HashSet::new(),
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

    let data = ReflectionFunctionData {
        func_name: func_sym,
    };
    let payload = vm.arena.get_mut(payload_handle);
    if let Val::ObjPayload(ref mut obj_data) = payload.value {
        obj_data.internal = Some(Rc::new(data));
    }

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

pub fn reflection_function_get_attributes(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
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
                internal
                    .downcast_ref::<ReflectionFunctionData>()
                    .map(|d| d.func_name)
            } else {
                None
            }
        } else {
            None
        }
    }
    .ok_or_else(|| {
        "ReflectionFunction::getAttributes() called on invalid reflection object".to_string()
    })?;

    let attrs = vm
        .context
        .user_functions
        .get(&func_sym)
        .ok_or("ReflectionFunction::getAttributes(): Function not found")?
        .attributes
        .clone();

    build_attribute_array(vm, &attrs, filter_name.as_deref())
}

pub fn reflection_function_get_parameters(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = get_this_handle(vm, "ReflectionFunction::getParameters()")?;
    let payload_handle = match &vm.arena.get(this_handle).value {
        Val::Object(payload_handle) => *payload_handle,
        _ => return Err("ReflectionFunction::getParameters() expects object context".into()),
    };

    let func_sym = {
        let payload = vm.arena.get(payload_handle);
        if let Val::ObjPayload(obj_data) = &payload.value {
            if let Some(internal) = &obj_data.internal {
                internal
                    .downcast_ref::<ReflectionFunctionData>()
                    .map(|d| d.func_name)
            } else {
                None
            }
        } else {
            None
        }
    }
    .ok_or_else(|| {
        "ReflectionFunction::getParameters() called on invalid reflection object".to_string()
    })?;

    let params = vm
        .context
        .user_functions
        .get(&func_sym)
        .ok_or("ReflectionFunction::getParameters(): Function not found")?
        .params
        .clone();

    let mut array = ArrayData::new();
    for (idx, param) in params.iter().enumerate() {
        let info = crate::runtime::context::ParameterInfo {
            name: param.name,
            type_hint: param.param_type.as_ref().and_then(return_type_to_type_hint),
            is_reference: param.by_ref,
            is_variadic: param.is_variadic,
            default_value: param.default_value.clone(),
            attributes: param.attributes.clone(),
        };
        let param_obj = create_reflection_parameter(vm, &info)?;
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
                internal
                    .downcast_ref::<ReflectionFunctionData>()
                    .map(|d| d.func_name)
            } else {
                None
            }
        } else {
            None
        }
    }
    .ok_or_else(|| {
        "ReflectionFunction::getReturnType() called on invalid reflection object".to_string()
    })?;

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
        ReturnType::Nullable(inner) => {
            return_type_to_type_hint(inner).map(|hint| TypeHint::Union(vec![hint, TypeHint::Null]))
        }
        _ => None,
    }
}
