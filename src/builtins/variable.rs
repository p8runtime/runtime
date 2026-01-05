use crate::core::value::{Handle, Val};
use crate::vm::engine::VM;
use std::rc::Rc;

pub fn php_var_dump(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
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
                        println!(
                            "object({}) ({}) {{",
                            String::from_utf8_lossy(
                                vm.context.interner.lookup(class).unwrap_or(b"")
                            ),
                            arr.map.len()
                        );
                        for (key, val_handle) in arr.map.iter() {
                            match key {
                                crate::core::value::ArrayKey::Int(i) => print!("  [{}]=>\n", i),
                                crate::core::value::ArrayKey::Str(s) => {
                                    print!("  [\"{}\"]=>\n", String::from_utf8_lossy(s))
                                }
                            }
                            dump_value(vm, *val_handle, 1);
                        }
                        println!("}}");
                        continue;
                    }
                }
            }
        }

        dump_value(vm, *arg, 0);
    }
    Ok(vm.arena.alloc(Val::Null))
}

fn dump_value(vm: &VM, handle: Handle, depth: usize) {
    let val = vm.arena.get(handle);
    let indent = "  ".repeat(depth);

    match &val.value {
        Val::String(s) => {
            println!(
                "{}string({}) \"{}\"",
                indent,
                s.len(),
                String::from_utf8_lossy(s)
            );
        }
        Val::Int(i) => {
            println!("{}int({})", indent, i);
        }
        Val::Float(f) => {
            println!("{}float({})", indent, f);
        }
        Val::Bool(b) => {
            println!("{}bool({})", indent, b);
        }
        Val::Null => {
            println!("{}NULL", indent);
        }
        Val::ConstArray(arr) => {
            // ConstArray shouldn't appear at runtime, but handle it just in case
            println!("{}array({}) {{ /* const array */ }}", indent, arr.len());
        }
        Val::Array(arr) => {
            println!("{}array({}) {{", indent, arr.map.len());
            for (key, val_handle) in arr.map.iter() {
                match key {
                    crate::core::value::ArrayKey::Int(i) => print!("{}  [{}]=>\n", indent, i),
                    crate::core::value::ArrayKey::Str(s) => {
                        print!("{}  [\"{}\"]=>\n", indent, String::from_utf8_lossy(s))
                    }
                }
                dump_value(vm, *val_handle, depth + 1);
            }
            println!("{}}}", indent);
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
                println!(
                    "{}object({})#{} ({}) {{",
                    indent,
                    String::from_utf8_lossy(class_name),
                    handle.index(),
                    obj.properties.len()
                );
                for (prop_sym, prop_handle) in &obj.properties {
                    let prop_name = vm
                        .context
                        .interner
                        .lookup(*prop_sym)
                        .unwrap_or(b"<unknown>");
                    println!("{}  [\"{}\"]=>", indent, String::from_utf8_lossy(prop_name));
                    dump_value(vm, *prop_handle, depth + 1);
                }
                println!("{}}}", indent);
            } else {
                println!("{}object(INVALID)", indent);
            }
        }
        Val::ObjPayload(_) => {
            println!("{}ObjPayload(Internal)", indent);
        }
        Val::Resource(_) => {
            println!("{}resource", indent);
        }
        Val::AppendPlaceholder => {
            println!("{}AppendPlaceholder", indent);
        }
        Val::Uninitialized => {
            println!("{}uninitialized", indent);
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
        print!("{}", output);
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
                    Val::Array(_) | Val::Object(_) => {
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
                        Val::Array(_) | Val::Object(_) => {
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
        Val::Object(_) => "object",
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
        // Notice: Constant already defined
        return Ok(vm.arena.alloc(Val::Bool(false)));
    }
    if vm.context.engine.registry.get_constant(&name).is_some() {
        // Notice: Constant already defined
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
    let is = matches!(val.value, Val::Object(_));
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

pub fn php_is_resource(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("is_resource() expects exactly 1 parameter".into());
    }
    let is = matches!(vm.arena.get(args[0]).value, Val::Resource(_));
    Ok(vm.arena.alloc(Val::Bool(is)))
}

/// strval($value): string — same conversion as echo/string interpolation.
pub fn php_strval(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("strval() expects exactly 1 parameter".into());
    }
    let bytes = vm.convert_to_string(args[0]).map_err(|e| e.to_string())?;
    Ok(vm.arena.alloc(Val::String(Rc::new(bytes))))
}

/// intval($value, $base = 10): int
/// Reference: $PHP_SRC_PATH/ext/standard/type.c - PHP_FUNCTION(intval)
pub fn php_intval(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() || args.len() > 2 {
        return Err("intval() expects 1 or 2 parameters".into());
    }
    let base = match args.get(1) {
        Some(h) => vm.arena.get(*h).value.to_int(),
        None => 10,
    };
    let val = &vm.arena.get(args[0]).value;
    let int_val = match val {
        Val::String(s) if base != 10 => strtol(s, base),
        other => other.to_int(),
    };
    Ok(vm.arena.alloc(Val::Int(int_val)))
}

/// C strtol as PHP's intval() uses it for non-decimal bases: leading
/// whitespace, optional sign, a 0x/0o/0b prefix (detected when base is 0),
/// digits up to the first invalid one, saturating on overflow.
fn strtol(s: &[u8], base: i64) -> i64 {
    let mut i = s.iter().take_while(|b| b.is_ascii_whitespace()).count();
    let negative = match s.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let prefix = |c: u8| {
        s.get(i) == Some(&b'0') && s.get(i + 1).map(|b| b.to_ascii_lowercase()) == Some(c)
    };
    let mut base = base;
    if (base == 16 || base == 0) && prefix(b'x') {
        base = 16;
        i += 2;
    } else if (base == 8 || base == 0) && prefix(b'o') {
        base = 8;
        i += 2;
    } else if (base == 2 || base == 0) && prefix(b'b') {
        base = 2;
        i += 2;
    } else if base == 0 {
        base = if s.get(i) == Some(&b'0') { 8 } else { 10 };
    }
    if !(2..=36).contains(&base) {
        return 0;
    }
    let mut acc: i64 = 0;
    for &b in &s[i.min(s.len())..] {
        let Some(d) = (b as char).to_digit(36).filter(|d| (*d as i64) < base) else {
            break;
        };
        acc = match acc.checked_mul(base).and_then(|a| {
            if negative {
                a.checked_sub(d as i64)
            } else {
                a.checked_add(d as i64)
            }
        }) {
            Some(a) => a,
            None => return if negative { i64::MIN } else { i64::MAX },
        };
    }
    acc
}

pub fn php_floatval(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("floatval() expects exactly 1 parameter".into());
    }
    let f = vm.arena.get(args[0]).value.to_float();
    Ok(vm.arena.alloc(Val::Float(f)))
}

pub fn php_boolval(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("boolval() expects exactly 1 parameter".into());
    }
    let b = vm.arena.get(args[0]).value.to_bool();
    Ok(vm.arena.alloc(Val::Bool(b)))
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

pub fn php_serialize(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("serialize() expects exactly 1 argument, 0 given".into());
    }

    let value = args[0];
    let serialized = serialize_value(vm, value)?;
    Ok(vm.arena.alloc(Val::String(Rc::new(serialized))))
}

fn serialize_value(vm: &VM, handle: Handle) -> Result<Vec<u8>, String> {
    let val = vm.arena.get(handle);
    let mut result = Vec::new();

    match &val.value {
        Val::Null => {
            result.extend_from_slice(b"N;");
        }
        Val::Bool(b) => {
            if *b {
                result.extend_from_slice(b"b:1;");
            } else {
                result.extend_from_slice(b"b:0;");
            }
        }
        Val::Int(i) => {
            result.extend_from_slice(b"i:");
            result.extend_from_slice(i.to_string().as_bytes());
            result.push(b';');
        }
        Val::Float(f) => {
            result.extend_from_slice(b"d:");
            // Format float like PHP does
            let f_str = if f.is_infinite() {
                if f.is_sign_positive() {
                    "INF"
                } else {
                    "-INF"
                }
            } else if f.is_nan() {
                "NAN"
            } else {
                // PHP uses specific formatting for floats
                &format!("{}", f)
            };
            result.extend_from_slice(f_str.as_bytes());
            result.push(b';');
        }
        Val::String(s) => {
            result.extend_from_slice(b"s:");
            result.extend_from_slice(s.len().to_string().as_bytes());
            result.extend_from_slice(b":\"");
            result.extend_from_slice(s);
            result.extend_from_slice(b"\";");
        }
        Val::Array(arr) => {
            result.extend_from_slice(b"a:");
            result.extend_from_slice(arr.map.len().to_string().as_bytes());
            result.extend_from_slice(b":{");
            for (key, val_handle) in arr.map.iter() {
                match key {
                    crate::core::value::ArrayKey::Int(i) => {
                        result.extend_from_slice(b"i:");
                        result.extend_from_slice(i.to_string().as_bytes());
                        result.push(b';');
                    }
                    crate::core::value::ArrayKey::Str(s) => {
                        result.extend_from_slice(b"s:");
                        result.extend_from_slice(s.len().to_string().as_bytes());
                        result.extend_from_slice(b":\"");
                        result.extend_from_slice(s);
                        result.extend_from_slice(b"\";");
                    }
                }
                let val_serialized = serialize_value(vm, *val_handle)?;
                result.extend_from_slice(&val_serialized);
            }
            result.push(b'}');
        }
        Val::ConstArray(arr) => {
            result.extend_from_slice(b"a:");
            result.extend_from_slice(arr.len().to_string().as_bytes());
            result.extend_from_slice(b":{");
            for (key, val) in arr.iter() {
                match key {
                    crate::core::value::ConstArrayKey::Int(i) => {
                        result.extend_from_slice(b"i:");
                        result.extend_from_slice(i.to_string().as_bytes());
                        result.push(b';');
                    }
                    crate::core::value::ConstArrayKey::Str(s) => {
                        result.extend_from_slice(b"s:");
                        result.extend_from_slice(s.len().to_string().as_bytes());
                        result.extend_from_slice(b":\"");
                        result.extend_from_slice(s);
                        result.extend_from_slice(b"\";");
                    }
                }
                // ConstArray contains Val directly, serialize it recursively
                let val_serialized = serialize_val(vm, val)?;
                result.extend_from_slice(&val_serialized);
            }
            result.push(b'}');
        }
        Val::Object(obj_handle) => {
            if let Val::ObjPayload(obj_data) = &vm.arena.get(*obj_handle).value {
                let class_name = vm
                    .context
                    .interner
                    .lookup(obj_data.class)
                    .unwrap_or(b"stdClass");
                
                result.extend_from_slice(b"O:");
                result.extend_from_slice(class_name.len().to_string().as_bytes());
                result.extend_from_slice(b":\"");
                result.extend_from_slice(class_name);
                result.extend_from_slice(b"\":");
                result.extend_from_slice(obj_data.properties.len().to_string().as_bytes());
                result.extend_from_slice(b":{");
                
                for (prop_name, prop_handle) in obj_data.properties.iter() {
                    let prop_name_bytes = vm.context.interner.lookup(*prop_name).unwrap_or(b"");
                    result.extend_from_slice(b"s:");
                    result.extend_from_slice(prop_name_bytes.len().to_string().as_bytes());
                    result.extend_from_slice(b":\"");
                    result.extend_from_slice(prop_name_bytes);
                    result.extend_from_slice(b"\";");
                    let val_serialized = serialize_value(vm, *prop_handle)?;
                    result.extend_from_slice(&val_serialized);
                }
                result.push(b'}');
            } else {
                return Err("Invalid object payload".into());
            }
        }
        _ => {
            return Err(format!("serialize() does not support type: {:?}", val.value));
        }
    }

    Ok(result)
}

// Helper function to serialize Val directly (for ConstArray)
fn serialize_val(vm: &VM, val: &Val) -> Result<Vec<u8>, String> {
    let mut result = Vec::new();

    match val {
        Val::Null => {
            result.extend_from_slice(b"N;");
        }
        Val::Bool(b) => {
            if *b {
                result.extend_from_slice(b"b:1;");
            } else {
                result.extend_from_slice(b"b:0;");
            }
        }
        Val::Int(i) => {
            result.extend_from_slice(b"i:");
            result.extend_from_slice(i.to_string().as_bytes());
            result.push(b';');
        }
        Val::Float(f) => {
            result.extend_from_slice(b"d:");
            let f_str = if f.is_infinite() {
                if f.is_sign_positive() {
                    "INF"
                } else {
                    "-INF"
                }
            } else if f.is_nan() {
                "NAN"
            } else {
                &format!("{}", f)
            };
            result.extend_from_slice(f_str.as_bytes());
            result.push(b';');
        }
        Val::String(s) => {
            result.extend_from_slice(b"s:");
            result.extend_from_slice(s.len().to_string().as_bytes());
            result.extend_from_slice(b":\"");
            result.extend_from_slice(s);
            result.extend_from_slice(b"\";");
        }
        Val::ConstArray(arr) => {
            result.extend_from_slice(b"a:");
            result.extend_from_slice(arr.len().to_string().as_bytes());
            result.extend_from_slice(b":{");
            for (key, nested_val) in arr.iter() {
                match key {
                    crate::core::value::ConstArrayKey::Int(i) => {
                        result.extend_from_slice(b"i:");
                        result.extend_from_slice(i.to_string().as_bytes());
                        result.push(b';');
                    }
                    crate::core::value::ConstArrayKey::Str(s) => {
                        result.extend_from_slice(b"s:");
                        result.extend_from_slice(s.len().to_string().as_bytes());
                        result.extend_from_slice(b":\"");
                        result.extend_from_slice(s);
                        result.extend_from_slice(b"\";");
                    }
                }
                let val_serialized = serialize_val(vm, nested_val)?;
                result.extend_from_slice(&val_serialized);
            }
            result.push(b'}');
        }
        _ => {
            return Err(format!("serialize_val() does not support type: {:?}", val));
        }
    }

    Ok(result)
}

pub fn php_unserialize(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("unserialize() expects at least 1 argument, 0 given".into());
    }

    let data = match &vm.arena.get(args[0]).value {
        Val::String(s) => s.clone(),
        _ => return Err("unserialize() expects parameter 1 to be string".into()),
    };

    // Options parameter is optional (args[1]) but not implemented yet
    let mut parser = UnserializeParser::new(&data);
    match parser.parse(vm) {
        Ok(handle) => Ok(handle),
        Err(_e) => {
            // PHP returns false on unserialize errors
            Ok(vm.arena.alloc(Val::Bool(false)))
        }
    }
}

struct UnserializeParser<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> UnserializeParser<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn consume(&mut self) -> Option<u8> {
        if self.pos < self.data.len() {
            let byte = self.data[self.pos];
            self.pos += 1;
            Some(byte)
        } else {
            None
        }
    }

    fn expect(&mut self, expected: u8) -> Result<(), String> {
        match self.consume() {
            Some(byte) if byte == expected => Ok(()),
            Some(byte) => Err(format!(
                "Expected '{}' but got '{}'",
                expected as char, byte as char
            )),
            None => Err("Unexpected end of data".into()),
        }
    }

    fn read_until(&mut self, delimiter: u8) -> Result<Vec<u8>, String> {
        let mut result = Vec::new();
        loop {
            match self.consume() {
                Some(byte) if byte == delimiter => return Ok(result),
                Some(byte) => result.push(byte),
                None => return Err("Unexpected end of data".into()),
            }
        }
    }

    fn read_int(&mut self) -> Result<i64, String> {
        let bytes = self.read_until(b';')?;
        let s = String::from_utf8(bytes).map_err(|_| "Invalid UTF-8 in integer")?;
        s.parse::<i64>()
            .map_err(|_| format!("Invalid integer: {}", s))
    }

    fn read_length(&mut self) -> Result<usize, String> {
        let bytes = self.read_until(b':')?;
        let s = String::from_utf8(bytes).map_err(|_| "Invalid UTF-8 in length")?;
        s.parse::<usize>()
            .map_err(|_| format!("Invalid length: {}", s))
    }

    fn read_float(&mut self) -> Result<f64, String> {
        let bytes = self.read_until(b';')?;
        let s = String::from_utf8(bytes).map_err(|_| "Invalid UTF-8 in float")?;
        
        // Handle special values
        match s.as_str() {
            "INF" => Ok(f64::INFINITY),
            "-INF" => Ok(f64::NEG_INFINITY),
            "NAN" => Ok(f64::NAN),
            _ => s.parse::<f64>()
                .map_err(|_| format!("Invalid float: {}", s)),
        }
    }

    fn read_string(&mut self, len: usize) -> Result<Vec<u8>, String> {
        self.expect(b'"')?;
        if self.pos + len > self.data.len() {
            return Err("String length exceeds data".into());
        }
        let result = self.data[self.pos..self.pos + len].to_vec();
        self.pos += len;
        self.expect(b'"')?;
        self.expect(b';')?;
        Ok(result)
    }

    fn read_string_no_semicolon(&mut self, len: usize) -> Result<Vec<u8>, String> {
        self.expect(b'"')?;
        if self.pos + len > self.data.len() {
            return Err("String length exceeds data".into());
        }
        let result = self.data[self.pos..self.pos + len].to_vec();
        self.pos += len;
        self.expect(b'"')?;
        Ok(result)
    }

    fn parse(&mut self, vm: &mut VM) -> Result<Handle, String> {
        let type_char = self.consume().ok_or("Empty serialized data")?;
        
        match type_char {
            b'N' => {
                // NULL doesn't have a colon, just N;
                self.expect(b';')?;
                Ok(vm.arena.alloc(Val::Null))
            }
            _ => {
                // All other types have a colon after the type character
                self.expect(b':')?;
                match type_char {
                    b'b' => {
                        let val = self.consume().ok_or("Missing bool value")?;
                        self.expect(b';')?;
                        Ok(vm.arena.alloc(Val::Bool(val == b'1')))
                    }
                    b'i' => {
                        let i = self.read_int()?;
                        Ok(vm.arena.alloc(Val::Int(i)))
                    }
                    b'd' => {
                        let f = self.read_float()?;
                        Ok(vm.arena.alloc(Val::Float(f)))
                    }
                    b's' => {
                        let len = self.read_length()?;
                        let s = self.read_string(len)?;
                        Ok(vm.arena.alloc(Val::String(Rc::new(s))))
                    }
                    b'a' => {
                        let count = self.read_length()?;
                        self.expect(b'{')?;
                        
                        let mut map = crate::core::value::ArrayData::new();
                        for _ in 0..count {
                            // Parse key
                            let key_type = self.consume().ok_or("Missing array key type")?;
                            self.expect(b':')?;
                            
                            let key = match key_type {
                                b'i' => {
                                    let i = self.read_int()?;
                                    crate::core::value::ArrayKey::Int(i)
                                }
                                b's' => {
                                    let len = self.read_length()?;
                                    let s = self.read_string(len)?;
                                    crate::core::value::ArrayKey::Str(s.into())
                                }
                                _ => return Err(format!("Invalid array key type: {}", key_type as char)),
                            };
                            
                            // Parse value
                            let value = self.parse(vm)?;
                            map.insert(key, value);
                        }
                        
                        self.expect(b'}')?;
                        Ok(vm.arena.alloc(Val::Array(map.into())))
                    }
                    b'O' => {
                        let class_name_len = self.read_length()?;
                        let class_name = self.read_string_no_semicolon(class_name_len)?;
                        self.expect(b':')?;
                        let class_sym = vm.context.interner.intern(&class_name);
                        
                        // Look up the class
                        let class_info = vm.context.classes.get(&class_sym);
                        if class_info.is_none() {
                            return Err(format!(
                                "Class '{}' not found",
                                String::from_utf8_lossy(&class_name)
                            ));
                        }
                        
                        let prop_count = self.read_length()?;
                        self.expect(b'{')?;
                        
                        // Create object
                        let obj_payload = crate::core::value::ObjectData {
                            class: class_sym,
                            properties: indexmap::IndexMap::new(),
                            internal: None,
                            dynamic_properties: std::collections::HashSet::new(),
                        };
                        let obj_handle = vm.arena.alloc(Val::ObjPayload(obj_payload));
                        let obj_ref = vm.arena.alloc(Val::Object(obj_handle));
                        
                        // Parse properties
                        for _ in 0..prop_count {
                            // Parse property name (always string)
                            let prop_type = self.consume().ok_or("Missing property name type")?;
                            self.expect(b':')?;
                            
                            if prop_type != b's' {
                                return Err(format!("Expected string for property name, got {}", prop_type as char));
                            }
                            
                            let prop_name_len = self.read_length()?;
                            let prop_name = self.read_string(prop_name_len)?;
                            let prop_sym = vm.context.interner.intern(&prop_name);
                            
                            // Parse property value
                            let value = self.parse(vm)?;
                            
                            // Set property
                            if let Val::ObjPayload(obj_data) = &mut vm.arena.get_mut(obj_handle).value {
                                obj_data.properties.insert(prop_sym, value);
                            }
                        }
                        
                        self.expect(b'}')?;
                        Ok(obj_ref)
                    }
                    _ => Err(format!("Unknown serialization type: {}", type_char as char)),
                }
            }
        }
    }
}
