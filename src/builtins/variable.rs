use crate::core::value::{ArrayData, ArrayKey, Handle, ObjectData, ObjectMapData, Val};
use crate::vm::engine::{ErrorLevel, PropertyCollectionMode, VM};
use std::fmt::Write;
use std::rc::Rc;
use std::collections::HashSet;
use indexmap::IndexMap;

pub fn php_deka_symbol_get(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() || args.len() > 2 {
        return Err("__deka_symbol_get() expects 1 or 2 parameters".into());
    }

    let name = vm
        .value_to_string_bytes(args[0])
        .map_err(|e| format!("__deka_symbol_get(): {}", e))?;
    let sym = vm.context.interner.intern(&name);
    let depth_raw = if args.len() == 2 {
        vm.value_to_int(args[1]) as usize
    } else {
        0
    };
    if vm.frames.is_empty() {
        return Ok(vm.arena.alloc(Val::Null));
    }
    let max_depth = vm.frames.len() - 1;
    let depth = depth_raw.min(max_depth);
    let frame_index = vm.frames.len() - 1 - depth;
    if let Some(frame) = vm.frames.get(frame_index) {
        if let Some(handle) = frame.locals.get(&sym).copied() {
            return Ok(handle);
        }
    }
    Ok(vm.arena.alloc(Val::Null))
}

pub fn php_deka_symbol_set(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 || args.len() > 3 {
        return Err("__deka_symbol_set() expects 2 or 3 parameters".into());
    }

    let name = vm
        .value_to_string_bytes(args[0])
        .map_err(|e| format!("__deka_symbol_set(): {}", e))?;
    let sym = vm.context.interner.intern(&name);
    let depth_raw = if args.len() == 3 {
        vm.value_to_int(args[2]) as usize
    } else {
        0
    };
    if vm.frames.is_empty() {
        return Ok(vm.arena.alloc(Val::Bool(false)));
    }
    let max_depth = vm.frames.len() - 1;
    let depth = depth_raw.min(max_depth);
    let frame_index = vm.frames.len() - 1 - depth;
    let frame = vm
        .frames
        .get_mut(frame_index)
        .ok_or_else(|| "__deka_symbol_set(): Invalid frame".to_string())?;

    if let Some(&old_handle) = frame.locals.get(&sym) {
        if vm.arena.get(old_handle).is_ref {
            let new_val = vm.arena.get(args[1]).value.clone();
            vm.arena.get_mut(old_handle).value = new_val;
            return Ok(vm.arena.alloc(Val::Bool(true)));
        }
    }

    let val = vm.arena.get(args[1]).value.clone();
    let final_handle = vm.arena.alloc(val);
    frame.locals.insert(sym, final_handle);
    Ok(vm.arena.alloc(Val::Bool(true)))
}

pub fn php_deka_symbol_exists(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() || args.len() > 2 {
        return Err("__deka_symbol_exists() expects 1 or 2 parameters".into());
    }

    let name = vm
        .value_to_string_bytes(args[0])
        .map_err(|e| format!("__deka_symbol_exists(): {}", e))?;
    let sym = vm.context.interner.intern(&name);
    let depth_raw = if args.len() == 2 {
        vm.value_to_int(args[1]) as usize
    } else {
        0
    };
    if vm.frames.is_empty() {
        return Ok(vm.arena.alloc(Val::Bool(false)));
    }
    let max_depth = vm.frames.len() - 1;
    let depth = depth_raw.min(max_depth);
    let frame_index = vm.frames.len() - 1 - depth;
    let exists = vm
        .frames
        .get(frame_index)
        .and_then(|frame| frame.locals.get(&sym))
        .is_some();
    Ok(vm.arena.alloc(Val::Bool(exists)))
}

pub fn php_deka_object_set(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 3 {
        return Err("__deka_object_set() expects exactly 3 parameters".into());
    }

    let key = vm
        .value_to_string_bytes(args[1])
        .map_err(|e| format!("__deka_object_set(): {}", e))?;
    let sym = vm.context.interner.intern(&key);
    let value_handle = args[2];

    match vm.arena.get(args[0]).value.clone() {
        Val::ObjectMap(_) => {
            let obj_zval = vm.arena.get_mut(args[0]);
            if let Val::ObjectMap(map_rc) = &mut obj_zval.value {
                let map = Rc::make_mut(map_rc);
                map.map.insert(sym, value_handle);
            }
            Ok(value_handle)
        }
        Val::Struct(obj_data) => {
            let class_name = obj_data.class;
            vm.assign_struct_property(args[0], class_name, sym, value_handle)
                .map_err(|e| format!("__deka_object_set(): {}", e))?;
            Ok(value_handle)
        }
        _ => Err("__deka_object_set() expects a PHPX object literal or struct".into()),
    }
}

pub fn php_phpx_object_new(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if !args.is_empty() {
        return Err("__phpx_object_new() expects no parameters".into());
    }
    let map = ObjectMapData::new();
    Ok(vm.arena.alloc(Val::ObjectMap(Rc::new(map))))
}

pub fn php_phpx_struct_new(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("__phpx_struct_new() expects exactly 1 parameter".into());
    }

    let name_bytes = vm
        .value_to_string_bytes(args[0])
        .map_err(|e| format!("__phpx_struct_new(): {}", e))?;
    let class_sym = vm.context.interner.intern(&name_bytes);
    let resolved = vm
        .resolve_class_name(class_sym)
        .map_err(|e| format!("__phpx_struct_new(): {}", e))?;

    if !vm.context.classes.contains_key(&resolved) {
        vm.trigger_autoload(resolved)
            .map_err(|e| format!("__phpx_struct_new(): {}", e))?;
    }

    let class_def = vm
        .context
        .classes
        .get(&resolved)
        .ok_or_else(|| "__phpx_struct_new(): Unknown struct".to_string())?;

    if class_def.is_interface {
        let class_name_str = vm
            .context
            .interner
            .lookup(resolved)
            .map(|b| String::from_utf8_lossy(b).to_string())
            .unwrap_or_else(|| format!("{:?}", resolved));
        return Err(format!(
            "__phpx_struct_new(): Cannot instantiate interface {}",
            class_name_str
        ));
    }

    if class_def.is_abstract && !class_def.is_interface {
        let class_name_str = vm
            .context
            .interner
            .lookup(resolved)
            .map(|b| String::from_utf8_lossy(b).to_string())
            .unwrap_or_else(|| format!("{:?}", resolved));
        return Err(format!(
            "__phpx_struct_new(): Cannot instantiate abstract class {}",
            class_name_str
        ));
    }

    if !class_def.is_struct {
        let class_name_str = vm
            .context
            .interner
            .lookup(resolved)
            .map(|b| String::from_utf8_lossy(b).to_string())
            .unwrap_or_else(|| format!("{:?}", resolved));
        return Err(format!(
            "__phpx_struct_new(): Struct literal requires a struct, got {}",
            class_name_str
        ));
    }

    let properties = vm.collect_properties(resolved, PropertyCollectionMode::All);
    let obj_data = ObjectData {
        class: resolved,
        properties,
        internal: None,
        dynamic_properties: HashSet::new(),
    };

    Ok(vm.arena.alloc(Val::Struct(Rc::new(obj_data))))
}

pub fn php_phpx_struct_set(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 3 {
        return Err("__phpx_struct_set() expects exactly 3 parameters".into());
    }

    let prop_name = vm
        .value_to_string_bytes(args[1])
        .map_err(|e| format!("__phpx_struct_set(): {}", e))?;
    let prop_sym = vm.context.interner.intern(&prop_name);
    let value_handle = args[2];

    match vm.arena.get(args[0]).value.clone() {
        Val::Struct(obj_data) => {
            let class_name = obj_data.class;
            vm.assign_struct_property(args[0], class_name, prop_sym, value_handle)
                .map_err(|e| format!("__phpx_struct_set(): {}", e))?;
            Ok(value_handle)
        }
        _ => Err("__phpx_struct_set() expects a PHPX struct".into()),
    }
}

fn phpx_to_php_value(vm: &mut VM, handle: Handle) -> Handle {
    match vm.arena.get(handle).value.clone() {
        Val::ObjectMap(map_rc) => {
            let mut props = IndexMap::new();
            for (prop_sym, val_handle) in map_rc.map.iter() {
                let converted = phpx_to_php_value(vm, *val_handle);
                props.insert(*prop_sym, converted);
            }
            let obj_data = ObjectData {
                class: vm.context.interner.intern(b"stdClass"),
                properties: props,
                internal: None,
                dynamic_properties: HashSet::new(),
            };
            let payload = vm.arena.alloc(Val::ObjPayload(obj_data));
            vm.arena.alloc(Val::Object(payload))
        }
        Val::Array(arr_rc) => {
            let mut array = ArrayData::new();
            for (key, val_handle) in arr_rc.map.iter() {
                let converted = phpx_to_php_value(vm, *val_handle);
                array.insert(key.clone(), converted);
            }
            vm.arena.alloc(Val::Array(Rc::new(array)))
        }
        Val::ConstArray(const_arr) => {
            let mut array = ArrayData::new();
            for (key, val) in const_arr.iter() {
                let runtime_key = match key {
                    crate::core::value::ConstArrayKey::Int(i) => ArrayKey::Int(*i),
                    crate::core::value::ConstArrayKey::Str(s) => ArrayKey::Str(s.clone()),
                };
                let val_handle = vm.arena.alloc(val.clone());
                let converted = phpx_to_php_value(vm, val_handle);
                array.insert(runtime_key, converted);
            }
            vm.arena.alloc(Val::Array(Rc::new(array)))
        }
        _ => handle,
    }
}

pub fn php_phpx_object_to_stdclass(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("__phpx_object_to_stdclass() expects exactly 1 parameter".into());
    }

    Ok(phpx_to_php_value(vm, args[0]))
}

pub fn php_var_dump(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let mut output = String::new();
    for arg in args {
        // Check for __debugInfo
        let class_sym = if let Val::Object(obj_handle) = vm.arena.get(*arg).value {
            if let Val::ObjPayload(obj_data) = &vm.arena.get(obj_handle).value {
                Some((obj_handle, obj_data.class))
            } else {
                None
            }
        } else {
            None
        };

        if let Some((obj_handle, class)) = class_sym {
            let debug_info_sym = vm.context.interner.intern(b"__debugInfo");
            if let Some((method, _, _, _)) = vm.find_method(class, debug_info_sym) {
                let mut frame = crate::vm::frame::CallFrame::new(method.chunk.clone());
                frame.func = Some(method.clone());
                frame.this = Some(obj_handle);
                frame.class_scope = Some(class);

                let res = vm.run_frame(frame);
                if let Ok(res_handle) = res {
                    let res_val = vm.arena.get(res_handle);
                    if let Val::Array(arr) = &res_val.value {
                        let class_name = String::from_utf8_lossy(
                            vm.context.interner.lookup(class).unwrap_or(b""),
                        );
                        let _ = writeln!(
                            output,
                            "object({}) ({}) {{",
                            class_name,
                            arr.map.len()
                        );
                        for (key, val_handle) in arr.map.iter() {
                            match key {
                                crate::core::value::ArrayKey::Int(i) => {
                                    let _ = writeln!(output, "  [{}]=>", i);
                                }
                                crate::core::value::ArrayKey::Str(s) => {
                                    let _ = writeln!(
                                        output,
                                        "  [\"{}\"]=>",
                                        String::from_utf8_lossy(s)
                                    );
                                }
                            }
                            dump_value(vm, *val_handle, 1, &mut output);
                        }
                        let _ = writeln!(output, "}}");
                        continue;
                    }
                }
            }
        }

        dump_value(vm, *arg, 0, &mut output);
    }
    vm.write_output(output.as_bytes())
        .map_err(|e| format!("{:?}", e))?;
    Ok(vm.arena.alloc(Val::Null))
}

fn dump_value(vm: &VM, handle: Handle, depth: usize, output: &mut String) {
    let val = vm.arena.get(handle);
    let indent = "  ".repeat(depth);

    match &val.value {
        Val::String(s) => {
            let _ = writeln!(
                output,
                "{}string({}) \"{}\"",
                indent,
                s.len(),
                String::from_utf8_lossy(s)
            );
        }
        Val::Int(i) => {
            let _ = writeln!(output, "{}int({})", indent, i);
        }
        Val::Float(f) => {
            let _ = writeln!(output, "{}float({})", indent, f);
        }
        Val::Bool(b) => {
            let _ = writeln!(output, "{}bool({})", indent, b);
        }
        Val::Null => {
            let _ = writeln!(output, "{}NULL", indent);
        }
        Val::ConstArray(arr) => {
            // ConstArray shouldn't appear at runtime, but handle it just in case
            let _ = writeln!(
                output,
                "{}array({}) {{ /* const array */ }}",
                indent,
                arr.len()
            );
        }
        Val::Array(arr) => {
            let _ = writeln!(output, "{}array({}) {{", indent, arr.map.len());
            for (key, val_handle) in arr.map.iter() {
                match key {
                    crate::core::value::ArrayKey::Int(i) => {
                        let _ = writeln!(output, "{}  [{}]=>", indent, i);
                    }
                    crate::core::value::ArrayKey::Str(s) => {
                        let _ = writeln!(
                            output,
                            "{}  [\"{}\"]=>",
                            indent,
                            String::from_utf8_lossy(s)
                        );
                    }
                }
                dump_value(vm, *val_handle, depth + 1, output);
            }
            let _ = writeln!(output, "{}}}", indent);
        }
        Val::Object(handle) => {
            // Dereference the object payload
            let payload_val = vm.arena.get(*handle);
            if let Val::ObjPayload(obj) = &payload_val.value {
                let class_name = vm
                    .context
                    .interner
                    .lookup(obj.class)
                    .unwrap_or(b"<unknown>");
                let _ = writeln!(
                    output,
                    "{}object({})#{} ({}) {{",
                    indent,
                    String::from_utf8_lossy(class_name),
                    handle.0,
                    obj.properties.len()
                );
                for (prop_sym, prop_handle) in &obj.properties {
                    let prop_name = vm
                        .context
                        .interner
                        .lookup(*prop_sym)
                        .unwrap_or(b"<unknown>");
                    let _ = writeln!(
                        output,
                        "{}  [\"{}\"]=>",
                        indent,
                        String::from_utf8_lossy(prop_name)
                    );
                    dump_value(vm, *prop_handle, depth + 1, output);
                }
                let _ = writeln!(output, "{}}}", indent);
            } else {
                let _ = writeln!(output, "{}object(INVALID)", indent);
            }
        }
        Val::Struct(obj_data) => {
            let class_name = vm
                .context
                .interner
                .lookup(obj_data.class)
                .unwrap_or(b"<unknown>");
            let _ = writeln!(
                output,
                "{}object({})#{} ({}) {{",
                indent,
                String::from_utf8_lossy(class_name),
                handle.0,
                obj_data.properties.len()
            );
            for (prop_sym, prop_handle) in obj_data.properties.iter() {
                let prop_name = vm
                    .context
                    .interner
                    .lookup(*prop_sym)
                    .unwrap_or(b"<unknown>");
                let _ = writeln!(
                    output,
                    "{}  [\"{}\"]=>",
                    indent,
                    String::from_utf8_lossy(prop_name)
                );
                dump_value(vm, *prop_handle, depth + 1, output);
            }
            let _ = writeln!(output, "{}}}", indent);
        }
        Val::ObjectMap(map_rc) => {
            let _ = writeln!(
                output,
                "{}object(Object)#{} ({}) {{",
                indent,
                handle.0,
                map_rc.map.len()
            );
            for (prop_sym, prop_handle) in map_rc.map.iter() {
                let prop_name = vm
                    .context
                    .interner
                    .lookup(*prop_sym)
                    .unwrap_or(b"<unknown>");
                let _ = writeln!(
                    output,
                    "{}  [\"{}\"]=>",
                    indent,
                    String::from_utf8_lossy(prop_name)
                );
                dump_value(vm, *prop_handle, depth + 1, output);
            }
            let _ = writeln!(output, "{}}}", indent);
        }
        Val::ObjPayload(_) => {
            let _ = writeln!(output, "{}ObjPayload(Internal)", indent);
        }
        Val::Resource(_) => {
            let _ = writeln!(output, "{}resource", indent);
        }
        Val::Promise(_) => {
            let _ = writeln!(output, "{}promise", indent);
        }
        Val::AppendPlaceholder => {
            let _ = writeln!(output, "{}AppendPlaceholder", indent);
        }
        Val::Uninitialized => {
            let _ = writeln!(output, "{}uninitialized", indent);
        }
    }
}

pub fn php_var_export(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 1 {
        return Err("var_export() expects at least 1 parameter".into());
    }

    let val_handle = args[0];
    let return_res = if args.len() > 1 {
        let ret_val = vm.arena.get(args[1]);
        match &ret_val.value {
            Val::Bool(b) => *b,
            _ => false,
        }
    } else {
        false
    };

    let mut output = String::new();
    export_value(vm, val_handle, 0, &mut output);

    if return_res {
        Ok(vm.arena.alloc(Val::String(output.into_bytes().into())))
    } else {
        vm.write_output(output.as_bytes())
            .map_err(|e| format!("{:?}", e))?;
        Ok(vm.arena.alloc(Val::Null))
    }
}

fn export_value(vm: &VM, handle: Handle, depth: usize, output: &mut String) {
    let val = vm.arena.get(handle);
    let indent = "  ".repeat(depth);

    match &val.value {
        Val::String(s) => {
            output.push('\'');
            output.push_str(
                &String::from_utf8_lossy(s)
                    .replace("\\", "\\\\")
                    .replace("'", "\\'"),
            );
            output.push('\'');
        }
        Val::Int(i) => {
            output.push_str(&i.to_string());
        }
        Val::Float(f) => {
            output.push_str(&f.to_string());
        }
        Val::Bool(b) => {
            output.push_str(if *b { "true" } else { "false" });
        }
        Val::Null => {
            output.push_str("NULL");
        }
        Val::Array(arr) => {
            output.push_str("array (\n");
            for (key, val_handle) in arr.map.iter() {
                output.push_str(&indent);
                output.push_str("  ");
                match key {
                    crate::core::value::ArrayKey::Int(i) => output.push_str(&i.to_string()),
                    crate::core::value::ArrayKey::Str(s) => {
                        output.push('\'');
                        output.push_str(
                            &String::from_utf8_lossy(s)
                                .replace("\\", "\\\\")
                                .replace("'", "\\'"),
                        );
                        output.push('\'');
                    }
                }
                output.push_str(" => ");
                export_value(vm, *val_handle, depth + 1, output);
                output.push_str(",\n");
            }
            output.push_str(&indent);
            output.push(')');
        }
        Val::Object(handle) => {
            let payload_val = vm.arena.get(*handle);
            if let Val::ObjPayload(obj) = &payload_val.value {
                let class_name = vm
                    .context
                    .interner
                    .lookup(obj.class)
                    .unwrap_or(b"<unknown>");
                output.push('\\');
                output.push_str(&String::from_utf8_lossy(class_name));
                output.push_str("::__set_state(array(\n");

                for (prop_sym, val_handle) in &obj.properties {
                    output.push_str(&indent);
                    output.push_str("  ");
                    let prop_name = vm.context.interner.lookup(*prop_sym).unwrap_or(b"");
                    output.push('\'');
                    output.push_str(
                        &String::from_utf8_lossy(prop_name)
                            .replace("\\", "\\\\")
                            .replace("'", "\\'"),
                    );
                    output.push('\'');
                    output.push_str(" => ");
                    export_value(vm, *val_handle, depth + 1, output);
                    output.push_str(",\n");
                }

                output.push_str(&indent);
                output.push_str("))");
            } else {
                output.push_str("NULL");
            }
        }
        _ => output.push_str("NULL"),
    }
}

pub fn php_print_r(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("print_r() expects at least 1 parameter".into());
    }

    let val_handle = args[0];
    let return_res = if args.len() > 1 {
        let ret_val = vm.arena.get(args[1]);
        match &ret_val.value {
            Val::Bool(b) => *b,
            _ => false,
        }
    } else {
        false
    };

    let mut output = String::new();
    print_r_value(vm, val_handle, 0, &mut output);

    if return_res {
        Ok(vm.arena.alloc(Val::String(output.into_bytes().into())))
    } else {
        vm.print_bytes(output.as_bytes())?;
        Ok(vm.arena.alloc(Val::Bool(true)))
    }
}

fn print_r_value(vm: &VM, handle: Handle, depth: usize, output: &mut String) {
    let val = vm.arena.get(handle);
    let indent = "    ".repeat(depth);

    match &val.value {
        Val::String(s) => {
            output.push_str(&String::from_utf8_lossy(s));
        }
        Val::Int(i) => {
            output.push_str(&i.to_string());
        }
        Val::Float(f) => {
            output.push_str(&f.to_string());
        }
        Val::Bool(b) => {
            output.push_str(if *b { "1" } else { "" });
        }
        Val::Null => {
            // print_r outputs nothing for null
        }
        Val::Array(arr) => {
            output.push_str("Array\n");
            output.push_str(&indent);
            output.push_str("(\n");
            for (key, val_handle) in arr.map.iter() {
                output.push_str(&indent);
                output.push_str("    ");
                match key {
                    crate::core::value::ArrayKey::Int(i) => {
                        output.push('[');
                        output.push_str(&i.to_string());
                        output.push_str("] => ");
                    }
                    crate::core::value::ArrayKey::Str(s) => {
                        output.push('[');
                        output.push_str(&String::from_utf8_lossy(s));
                        output.push_str("] => ");
                    }
                }

                // Check if value is array or object to put it on new line
                let val = vm.arena.get(*val_handle);
                match &val.value {
                    Val::Array(_) | Val::Object(_) | Val::ObjectMap(_) | Val::Struct(_) => {
                        print_r_value(vm, *val_handle, depth + 2, output);
                        output.push('\n');
                    }
                    _ => {
                        print_r_value(vm, *val_handle, depth + 1, output);
                        output.push('\n');
                    }
                }
            }
            output.push_str(&indent);
            output.push_str(")\n");
        }
        Val::Object(handle) => {
            let payload_val = vm.arena.get(*handle);
            if let Val::ObjPayload(obj) = &payload_val.value {
                let class_name = vm
                    .context
                    .interner
                    .lookup(obj.class)
                    .unwrap_or(b"<unknown>");
                output.push_str(&String::from_utf8_lossy(class_name));
                output.push_str(" Object\n");
                output.push_str(&indent);
                output.push_str("(\n");

                for (prop_sym, val_handle) in &obj.properties {
                    output.push_str(&indent);
                    output.push_str("    ");
                    let prop_name = vm.context.interner.lookup(*prop_sym).unwrap_or(b"");
                    output.push('[');
                    output.push_str(&String::from_utf8_lossy(prop_name));
                    output.push_str("] => ");

                    let val = vm.arena.get(*val_handle);
                    match &val.value {
                        Val::Array(_) | Val::Object(_) | Val::ObjectMap(_) | Val::Struct(_) => {
                            print_r_value(vm, *val_handle, depth + 2, output);
                            output.push('\n');
                        }
                        _ => {
                            print_r_value(vm, *val_handle, depth + 1, output);
                            output.push('\n');
                        }
                    }
                }

                output.push_str(&indent);
                output.push_str(")\n");
            } else {
                // shouldn't happen
            }
        }
        Val::Struct(obj_data) => {
            let class_name = vm
                .context
                .interner
                .lookup(obj_data.class)
                .unwrap_or(b"<unknown>");
            output.push_str(&String::from_utf8_lossy(class_name));
            output.push_str(" Object\n");
            output.push_str(&indent);
            output.push_str("(\n");

            for (prop_sym, val_handle) in obj_data.properties.iter() {
                output.push_str(&indent);
                output.push_str("    ");
                let prop_name = vm.context.interner.lookup(*prop_sym).unwrap_or(b"");
                output.push('[');
                output.push_str(&String::from_utf8_lossy(prop_name));
                output.push_str("] => ");

                let val = vm.arena.get(*val_handle);
                match &val.value {
                    Val::Array(_) | Val::Object(_) | Val::ObjectMap(_) | Val::Struct(_) => {
                        print_r_value(vm, *val_handle, depth + 2, output);
                        output.push('\n');
                    }
                    _ => {
                        print_r_value(vm, *val_handle, depth + 1, output);
                        output.push('\n');
                    }
                }
            }

            output.push_str(&indent);
            output.push_str(")\n");
        }
        Val::ObjectMap(map_rc) => {
            output.push_str("Object\n");
            output.push_str(&indent);
            output.push_str("(\n");

            for (prop_sym, val_handle) in map_rc.map.iter() {
                output.push_str(&indent);
                output.push_str("    ");
                let prop_name = vm.context.interner.lookup(*prop_sym).unwrap_or(b"");
                output.push('[');
                output.push_str(&String::from_utf8_lossy(prop_name));
                output.push_str("] => ");

                let val = vm.arena.get(*val_handle);
                match &val.value {
                    Val::Array(_) | Val::Object(_) | Val::ObjectMap(_) | Val::Struct(_) => {
                        print_r_value(vm, *val_handle, depth + 2, output);
                        output.push('\n');
                    }
                    _ => {
                        print_r_value(vm, *val_handle, depth + 1, output);
                        output.push('\n');
                    }
                }
            }

            output.push_str(&indent);
            output.push_str(")\n");
        }
        _ => {
            // For other types, just output empty or their representation
        }
    }
}

pub fn php_gettype(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("gettype() expects exactly 1 parameter".into());
    }

    let val = vm.arena.get(args[0]);
    let type_str = match &val.value {
        Val::Null => "NULL",
        Val::Bool(_) => "boolean",
        Val::Int(_) => "integer",
        Val::Float(_) => "double",
        Val::String(_) => "string",
        Val::Array(_) => "array",
        Val::Object(_) | Val::ObjectMap(_) | Val::Struct(_) => "object",
        Val::ObjPayload(_) => "object",
        Val::Resource(_) => "resource",
        _ => "unknown type",
    };

    Ok(vm
        .arena
        .alloc(Val::String(type_str.as_bytes().to_vec().into())))
}

pub fn php_define(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("define() expects at least 2 parameters".into());
    }

    let name_val = vm.arena.get(args[0]);
    let name = match &name_val.value {
        Val::String(s) => s.clone(),
        _ => return Err("define(): Parameter 1 must be string".into()),
    };

    let value_handle = args[1];
    let value = vm.arena.get(value_handle).value.clone();

    // Case insensitive? Third arg.
    let _case_insensitive = if args.len() > 2 {
        let ci_val = vm.arena.get(args[2]);
        match &ci_val.value {
            Val::Bool(b) => *b,
            _ => false,
        }
    } else {
        false
    };

    let sym = vm.context.interner.intern(&name);

    // Check if constant already defined (in request context or registry)
    if vm.context.constants.contains_key(&sym) {
        let name_str = String::from_utf8_lossy(&name);
        let message = format!(
            "Constant {} already defined, this will be an error in PHP 9",
            name_str
        );
        vm.report_error(ErrorLevel::Warning, &message);
        let _ = vm.write_output(format!("Warning: {}\n", message).as_bytes());
        return Ok(vm.arena.alloc(Val::Bool(false)));
    }
    if vm.context.engine.registry.get_constant(&name).is_some() {
        let name_str = String::from_utf8_lossy(&name);
        let message = format!(
            "Constant {} already defined, this will be an error in PHP 9",
            name_str
        );
        vm.report_error(ErrorLevel::Warning, &message);
        let _ = vm.write_output(format!("Warning: {}\n", message).as_bytes());
        return Ok(vm.arena.alloc(Val::Bool(false)));
    }

    vm.context.constants.insert(sym, value);

    Ok(vm.arena.alloc(Val::Bool(true)))
}

pub fn php_defined(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("defined() expects exactly 1 parameter".into());
    }

    let name_val = vm.arena.get(args[0]);
    let name = match &name_val.value {
        Val::String(s) => s.clone(),
        _ => return Err("defined(): Parameter 1 must be string".into()),
    };

    let sym = vm.context.interner.intern(&name);

    // Check if constant exists (in request context or registry)
    let exists = vm.context.constants.contains_key(&sym)
        || vm.context.engine.registry.get_constant(&name).is_some();

    Ok(vm.arena.alloc(Val::Bool(exists)))
}

pub fn php_constant(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("constant() expects exactly 1 parameter".into());
    }

    let name_val = vm.arena.get(args[0]);
    let name = match &name_val.value {
        Val::String(s) => s.clone(),
        _ => return Err("constant(): Parameter 1 must be string".into()),
    };

    let sym = vm.context.interner.intern(&name);

    // Check request context constants first
    if let Some(val) = vm.context.constants.get(&sym) {
        return Ok(vm.arena.alloc(val.clone()));
    }

    // Check registry constants
    if let Some(val) = vm.context.engine.registry.get_constant(&name) {
        return Ok(vm.arena.alloc(val.clone()));
    }

    // TODO: Warning
    Ok(vm.arena.alloc(Val::Null))
}

pub fn php_is_string(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("is_string() expects exactly 1 parameter".into());
    }
    let val = vm.arena.get(args[0]);
    let is = matches!(val.value, Val::String(_));
    Ok(vm.arena.alloc(Val::Bool(is)))
}

pub fn php_is_int(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("is_int() expects exactly 1 parameter".into());
    }
    let val = vm.arena.get(args[0]);
    let is = matches!(val.value, Val::Int(_));
    Ok(vm.arena.alloc(Val::Bool(is)))
}

pub fn php_is_array(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("is_array() expects exactly 1 parameter".into());
    }
    let val = vm.arena.get(args[0]);
    let is = matches!(val.value, Val::Array(_));
    Ok(vm.arena.alloc(Val::Bool(is)))
}

pub fn php_is_bool(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("is_bool() expects exactly 1 parameter".into());
    }
    let val = vm.arena.get(args[0]);
    let is = matches!(val.value, Val::Bool(_));
    Ok(vm.arena.alloc(Val::Bool(is)))
}

pub fn php_is_null(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("is_null() expects exactly 1 parameter".into());
    }
    let val = vm.arena.get(args[0]);
    let is = matches!(val.value, Val::Null);
    Ok(vm.arena.alloc(Val::Bool(is)))
}

pub fn php_is_object(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("is_object() expects exactly 1 parameter".into());
    }
    let val = vm.arena.get(args[0]);
    let is = matches!(val.value, Val::Object(_) | Val::ObjectMap(_) | Val::Struct(_));
    Ok(vm.arena.alloc(Val::Bool(is)))
}

pub fn php_is_float(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("is_float() expects exactly 1 parameter".into());
    }
    let val = vm.arena.get(args[0]);
    let is = matches!(val.value, Val::Float(_));
    Ok(vm.arena.alloc(Val::Bool(is)))
}

pub fn php_is_numeric(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("is_numeric() expects exactly 1 parameter".into());
    }
    let val = vm.arena.get(args[0]);
    let is = match &val.value {
        Val::Int(_) | Val::Float(_) => true,
        Val::String(s) => {
            // Simple check for numeric string
            let s = String::from_utf8_lossy(s);
            s.trim().parse::<f64>().is_ok()
        }
        _ => false,
    };
    Ok(vm.arena.alloc(Val::Bool(is)))
}

pub fn php_is_scalar(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("is_scalar() expects exactly 1 parameter".into());
    }
    let val = vm.arena.get(args[0]);
    let is = matches!(
        val.value,
        Val::Int(_) | Val::Float(_) | Val::String(_) | Val::Bool(_)
    );
    Ok(vm.arena.alloc(Val::Bool(is)))
}

pub fn php_getenv(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        // Validation: php_getenv without args returns array of all env vars (not implemented here yet)
        // or just returns false?
        // PHP documentation says: string|false getenv(( string $name = null [, bool $local_only = false ] ))
        // If name is null, returns array of all env vars.
        return Err("getenv() expects at least 1 parameter".into());
    }

    let name_val = vm.arena.get(args[0]);
    let name = match &name_val.value {
        Val::String(s) => String::from_utf8_lossy(s).to_string(),
        _ => return Err("getenv(): Parameter 1 must be string".into()),
    };

    match std::env::var(&name) {
        Ok(val) => Ok(vm.arena.alloc(Val::String(Rc::new(val.into_bytes())))),
        Err(_) => Ok(vm.arena.alloc(Val::Bool(false))),
    }
}

pub fn php_putenv(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("putenv() expects exactly 1 parameter".into());
    }

    let setting_val = vm.arena.get(args[0]);
    let setting = match &setting_val.value {
        Val::String(s) => String::from_utf8_lossy(s).to_string(),
        _ => return Err("putenv(): Parameter 1 must be string".into()),
    };

    if let Some((key, val)) = setting.split_once('=') {
        unsafe {
            if val.is_empty() {
                std::env::remove_var(key);
            } else {
                std::env::set_var(key, val);
            }
        }
    } else {
        // "KEY" without "=" -> unset? Or no-op?
        // PHP manual: "setting - The setting, like "FOO=BAR""
        // std implementation usually requires key=val.
        // If just "KEY", PHP might unset it.
        unsafe {
            std::env::remove_var(&setting);
        }
    }

    Ok(vm.arena.alloc(Val::Bool(true)))
}

pub fn php_getopt(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("getopt() expects at least 1 parameter".into());
    }

    // TODO: Implement proper getopt parsing using $argv
    // For now, return an empty array to prevent crashes
    let map = crate::core::value::ArrayData::new();
    Ok(vm.arena.alloc(Val::Array(map.into())))
}

pub fn php_ini_get(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("ini_get() expects exactly 1 parameter".into());
    }

    let option = match &vm.arena.get(args[0]).value {
        Val::String(s) => String::from_utf8_lossy(s).to_string(),
        _ => return Err("ini_get() expects string parameter".into()),
    };

    // Return commonly expected ini values
    let value = match option.as_str() {
        "display_errors" => "1".to_string(),
        "error_reporting" => "32767".to_string(), // E_ALL
        "memory_limit" => "128M".to_string(),
        "max_execution_time" => vm.context.config.max_execution_time.to_string(),
        "upload_max_filesize" => "2M".to_string(),
        "post_max_size" => "8M".to_string(),
        _ => "".to_string(), // Unknown settings return empty string
    };

    Ok(vm
        .arena
        .alloc(Val::String(Rc::new(value.as_bytes().to_vec()))))
}

pub fn php_ini_set(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("ini_set() expects exactly 2 parameters".into());
    }

    let _option = match &vm.arena.get(args[0]).value {
        Val::String(s) => String::from_utf8_lossy(s).to_string(),
        _ => return Err("ini_set() expects string parameter".into()),
    };

    let _new_value = match &vm.arena.get(args[1]).value {
        Val::String(s) => String::from_utf8_lossy(s).to_string(),
        Val::Int(i) => i.to_string(),
        _ => return Err("ini_set() value must be string or int".into()),
    };

    // TODO: Actually store ini settings in context
    // For now, just return false to indicate setting couldn't be changed
    Ok(vm.arena.alloc(Val::String(Rc::new(b"".to_vec()))))
}

pub fn php_error_reporting(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let old_level = vm.context.config.error_reporting as i64;

    if args.is_empty() {
        // No arguments: return current level
        return Ok(vm.arena.alloc(Val::Int(old_level)));
    }

    // Set new error reporting level
    let new_level = match &vm.arena.get(args[0]).value {
        Val::Int(i) => *i as u32,
        Val::Null => 0, // null means disable all errors
        _ => return Err("error_reporting() expects int parameter".into()),
    };

    vm.context.config.error_reporting = new_level;
    Ok(vm.arena.alloc(Val::Int(old_level)))
}

pub fn php_error_get_last(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if !args.is_empty() {
        return Err("error_get_last() expects no parameters".into());
    }

    if let Some(error_info) = &vm.context.last_error {
        // Build array with error information
        let mut map = crate::core::value::ArrayData::new();

        let type_key = crate::core::value::ArrayKey::Str(b"type".to_vec().into());
        let type_val = vm.arena.alloc(Val::Int(error_info.error_type));
        map.insert(type_key, type_val);

        let message_key = crate::core::value::ArrayKey::Str(b"message".to_vec().into());
        let message_val = vm
            .arena
            .alloc(Val::String(Rc::new(error_info.message.as_bytes().to_vec())));
        map.insert(message_key, message_val);

        let file_key = crate::core::value::ArrayKey::Str(b"file".to_vec().into());
        let file_val = vm
            .arena
            .alloc(Val::String(Rc::new(error_info.file.as_bytes().to_vec())));
        map.insert(file_key, file_val);

        let line_key = crate::core::value::ArrayKey::Str(b"line".to_vec().into());
        let line_val = vm.arena.alloc(Val::Int(error_info.line));
        map.insert(line_key, line_val);

        Ok(vm.arena.alloc(Val::Array(map.into())))
    } else {
        // No error recorded yet, return null
        Ok(vm.arena.alloc(Val::Null))
    }
}
