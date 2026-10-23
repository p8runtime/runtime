use crate::core::value::{Handle, Val};
use crate::vm::engine::VM;
use std::rc::Rc;

pub fn php_var_dump(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    for &arg in args {
        let mut out = Vec::new();
        let mut dumper = Dumper { out: &mut out, visiting: Vec::new() };
        dumper.value(vm, arg, 0)?;
        vm.print_bytes(&out)?;
    }
    Ok(vm.arena.alloc(Val::Null))
}

/// var_dump's writer.
/// Reference: $PHP_SRC_PATH/ext/standard/var.c - php_var_dump
struct Dumper<'a> {
    out: &'a mut Vec<u8>,
    /// Arrays and objects being dumped, to print *RECURSION* on a cycle.
    visiting: Vec<Handle>,
}

impl Dumper<'_> {
    fn line(&mut self, indent: usize, text: &[u8]) {
        self.out.extend(std::iter::repeat_n(b' ', indent));
        self.out.extend_from_slice(text);
        self.out.push(b'\n');
    }

    fn key(&mut self, indent: usize, key: &crate::core::value::ArrayKey) {
        let mut text = Vec::new();
        match key {
            crate::core::value::ArrayKey::Int(i) => text.extend_from_slice(format!("[{}]=>", i).as_bytes()),
            crate::core::value::ArrayKey::Str(s) => {
                text.extend_from_slice(b"[\"");
                text.extend_from_slice(s);
                text.extend_from_slice(b"\"]=>");
            }
        }
        self.line(indent, &text);
    }

    fn value(&mut self, vm: &mut VM, handle: Handle, indent: usize) -> Result<(), String> {
        let val = vm.arena.get(handle).value.clone();
        match val {
            Val::Null | Val::AppendPlaceholder => self.line(indent, b"NULL"),
            Val::Uninitialized => self.line(indent, b"uninitialized"),
            Val::Bool(b) => self.line(indent, if b { b"bool(true)" } else { b"bool(false)" }),
            Val::Int(i) => self.line(indent, format!("int({})", i).as_bytes()),
            Val::Float(f) => {
                let text = crate::core::value::php_float_to_string(
                    f,
                    crate::core::value::FloatPrecision::Shortest,
                    'E',
                );
                self.line(indent, format!("float({})", text).as_bytes());
            }
            Val::String(s) => {
                let mut text = format!("string({}) \"", s.len()).into_bytes();
                text.extend_from_slice(&s);
                text.push(b'"');
                self.line(indent, &text);
            }
            Val::Resource(_) => self.line(indent, b"resource(0) of type (Unknown)"),
            Val::ConstArray(_) | Val::ObjPayload(_) => self.line(indent, b"NULL"),
            Val::Array(arr) => {
                if self.visiting.contains(&handle) {
                    self.line(indent, b"*RECURSION*");
                    return Ok(());
                }
                self.line(indent, format!("array({}) {{", arr.map.len()).as_bytes());
                self.visiting.push(handle);
                for (key, &element) in arr.map.iter() {
                    self.key(indent + 2, key);
                    self.value(vm, element, indent + 2)?;
                }
                self.visiting.pop();
                self.line(indent, b"}");
            }
            Val::Object(payload) => self.object(vm, handle, payload, indent)?,
        }
        Ok(())
    }

    fn object(&mut self, vm: &mut VM, handle: Handle, payload: Handle, indent: usize) -> Result<(), String> {
        let (class, props) = match &vm.arena.get(payload).value {
            Val::ObjPayload(obj) => (obj.class, obj.properties.clone()),
            _ => {
                self.line(indent, b"NULL");
                return Ok(());
            }
        };
        let class_name = vm.context.interner.lookup(class).unwrap_or(b"").to_vec();
        let shown = display_class_name(&class_name).to_vec();

        if let Some(def) = vm.context.classes.get(&class).filter(|d| d.is_enum) {
            let case = def
                .enum_cases
                .iter()
                .find(|c| vm.arena.get(c.handle).value == Val::Object(payload))
                .and_then(|c| vm.context.interner.lookup(c.name))
                .unwrap_or(b"")
                .to_vec();
            let mut text = b"enum(".to_vec();
            text.extend_from_slice(&shown);
            text.extend_from_slice(b"::");
            text.extend_from_slice(&case);
            text.push(b')');
            self.line(indent, &text);
            return Ok(());
        }
        if self.visiting.contains(&payload) {
            self.line(indent, b"*RECURSION*");
            return Ok(());
        }

        // __debugInfo() replaces the property table.
        let debug_info = vm.context.interner.intern(b"__debugInfo");
        let entries: Vec<(Vec<u8>, Handle, Option<crate::core::value::Symbol>)> = if vm.find_method(class, debug_info).is_some() {
            let res = vm
                .call_method(handle, debug_info, &[])
                .map_err(|e| format!("{:?}", e))?;
            match vm.arena.get(res).value.clone() {
                Val::Array(arr) => arr
                    .map
                    .iter()
                    .map(|(k, v)| {
                        let name = match k {
                            crate::core::value::ArrayKey::Int(i) => format!("[{}]=>", i).into_bytes(),
                            crate::core::value::ArrayKey::Str(s) => {
                                [b"[\"".as_slice(), s, b"\"]=>"].concat()
                            }
                        };
                        (name, *v, None)
                    })
                    .collect(),
                _ => {
                    let msg = "__debuginfo() must return an array";
                    return Err(vm.throw_from_native(b"Error", msg));
                }
            }
        } else {
            props
                .iter()
                .map(|(prop, v)| {
                    let name = vm.context.interner.lookup(*prop).unwrap_or(b"").to_vec();
                    let owner = vm.walk_inheritance_chain(class, |def, cls| {
                        def.properties.get(prop).map(|entry| (cls, entry.visibility))
                    });
                    let label = match owner {
                        Some((_, crate::core::value::Visibility::Protected)) => {
                            [b"[\"".as_slice(), &name, b"\":protected]=>"].concat()
                        }
                        Some((cls, crate::core::value::Visibility::Private)) => {
                            let cls = vm.context.interner.lookup(cls).unwrap_or(b"");
                            [b"[\"".as_slice(), &name, b"\":\"", display_class_name(cls), b"\":private]=>"].concat()
                        }
                        _ => [b"[\"".as_slice(), &name, b"\"]=>"].concat(),
                    };
                    (label, *v, Some(*prop))
                })
                .collect()
        };

        let initialized = entries
            .iter()
            .filter(|(_, v, _)| !matches!(vm.arena.get(*v).value, Val::Uninitialized))
            .count();
        let mut header = b"object(".to_vec();
        header.extend_from_slice(&shown);
        header.extend_from_slice(format!(")#{} ({}) {{", payload.index(), initialized).as_bytes());
        self.line(indent, &header);
        self.visiting.push(payload);
        for (label, value, prop) in entries {
            self.line(indent + 2, &label);
            if let (Val::Uninitialized, Some(prop)) = (&vm.arena.get(value).value, prop) {
                let ty = vm
                    .walk_inheritance_chain(class, |def, _| {
                        def.properties.get(&prop).and_then(|e| e.type_hint.clone())
                    })
                    .map(|t| vm.type_hint_to_string(&t))
                    .unwrap_or_else(|| "mixed".into());
                self.line(indent + 2, format!("uninitialized({})", ty).as_bytes());
            } else {
                self.value(vm, value, indent + 2)?;
            }
        }
        self.visiting.pop();
        self.line(indent, b"}");
        Ok(())
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
        vm.print_bytes(output.as_bytes())?;
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
            let repr = crate::core::value::php_float_to_string(*f, crate::core::value::FloatPrecision::Shortest, 'E');
            let integral = repr.bytes().all(|b| b.is_ascii_digit() || b == b'-');
            output.push_str(&repr);
            if integral {
                output.push_str(".0");
            }
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
                // Nested arrays and objects start on their own line.
                if matches!(vm.arena.get(*val_handle).value, Val::Array(_) | Val::Object(_)) {
                    output.push('\n');
                    output.push_str(&indent);
                    output.push_str("  ");
                }
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
            output.push_str(&crate::core::value::php_float_to_string(*f, crate::core::value::FloatPrecision::ECHO, 'E'));
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
                output.push_str(&String::from_utf8_lossy(display_class_name(class_name)));
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

/// getenv(?string $name = null, bool $local_only = false): array|string|false
pub fn php_getenv(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let name = match args.first().map(|h| vm.arena.get(*h).value.clone()) {
        None | Some(Val::Null) => None,
        Some(v) => Some(v.to_php_string_bytes()),
    };
    match name {
        Some(name) => match vm.context.env_var(&name) {
            Some(value) => Ok(vm.arena.alloc(Val::String(Rc::new(value)))),
            None => Ok(vm.arena.alloc(Val::Bool(false))),
        },
        None => {
            let mut arr = crate::core::value::ArrayData::new();
            for (key, value) in vm.context.env_vars() {
                let h = vm.arena.alloc(Val::String(Rc::new(value)));
                arr.insert(crate::core::value::ArrayKey::Str(Rc::new(key)), h);
            }
            Ok(vm.arena.alloc(Val::Array(Rc::new(arr))))
        }
    }
}

/// putenv(string $assignment): bool
///
/// "NAME=value" sets (an empty value included), "NAME" unsets. The change is
/// kept per request and passed to child processes; the process environment
/// itself is never written.
pub fn php_putenv(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let Some(&arg) = args.first() else {
        return Err("putenv() expects exactly 1 parameter".into());
    };
    let setting = vm.arena.get(arg).value.to_php_string_bytes();
    let (key, value) = match setting.iter().position(|b| *b == b'=') {
        Some(eq) => (setting[..eq].to_vec(), Some(setting[eq + 1..].to_vec())),
        None => (setting.clone(), None),
    };
    if key.is_empty() {
        return Err(vm.throw_from_native(
            b"ValueError",
            "putenv(): Argument #1 ($assignment) must have a valid syntax",
        ));
    }
    vm.context.env_overrides.insert(key, value);
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


/// A class name as var_dump and print_r show it: an anonymous class's name
/// ends at the NUL that precedes its file and line.
fn display_class_name(name: &[u8]) -> &[u8] {
    name.split(|b| *b == 0).next().unwrap_or(name)
}
