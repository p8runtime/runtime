use crate::core::value::{Handle, Val};
use crate::vm::engine::VM;
use std::rc::Rc;

/// Exception::__construct($message = "", $code = 0, Throwable $previous = null)
pub fn exception_construct(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    // Get $this from current frame
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or_else(|| "No $this in exception construct".to_string())?;

    // Get message (arg 0, default "")
    let message = if let Some(&msg_handle) = args.get(0) {
        if let Val::String(s) = &vm.arena.get(msg_handle).value {
            s.clone()
        } else {
            Rc::new(Vec::new())
        }
    } else {
        Rc::new(Vec::new())
    };

    // Get code (arg 1, default 0)
    let code = if let Some(&code_handle) = args.get(1) {
        if let Val::Int(c) = &vm.arena.get(code_handle).value {
            *c
        } else {
            0
        }
    } else {
        0
    };

    // Get previous (arg 2, default null)
    let previous = if let Some(&prev_handle) = args.get(2) {
        prev_handle
    } else {
        vm.arena.alloc(Val::Null)
    };

    // Set properties on the exception object
    let message_sym = vm.context.interner.intern(b"message");
    let code_sym = vm.context.interner.intern(b"code");
    let previous_sym = vm.context.interner.intern(b"previous");

    // Allocate property values
    let message_handle = vm.arena.alloc(Val::String(message));
    let code_handle = vm.arena.alloc(Val::Int(code));

    // Update object properties
    if let Val::Object(payload_handle) = &vm.arena.get(this_handle).value {
        let payload = vm.arena.get_mut(*payload_handle);
        if let Val::ObjPayload(ref mut obj_data) = payload.value {
            obj_data.properties.insert(message_sym, message_handle);
            obj_data.properties.insert(code_sym, code_handle);
            obj_data.properties.insert(previous_sym, previous);
        }
    }

    Ok(vm.arena.alloc(Val::Null))
}

/// Exception::getMessage() - Returns the exception message
pub fn exception_get_message(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("getMessage() called outside object context")?;

    let message_sym = vm.context.interner.intern(b"message");

    if let Val::Object(payload_handle) = &vm.arena.get(this_handle).value {
        if let Val::ObjPayload(obj_data) = &vm.arena.get(*payload_handle).value {
            if let Some(&msg_handle) = obj_data.properties.get(&message_sym) {
                return Ok(msg_handle);
            }
        }
    }

    // Return empty string if no message
    Ok(vm.arena.alloc(Val::String(Rc::new(Vec::new()))))
}

/// Exception::getCode() - Returns the exception code
pub fn exception_get_code(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("getCode() called outside object context")?;

    let code_sym = vm.context.interner.intern(b"code");

    if let Val::Object(payload_handle) = &vm.arena.get(this_handle).value {
        if let Val::ObjPayload(obj_data) = &vm.arena.get(*payload_handle).value {
            if let Some(&code_handle) = obj_data.properties.get(&code_sym) {
                return Ok(code_handle);
            }
        }
    }

    // Return 0 if no code
    Ok(vm.arena.alloc(Val::Int(0)))
}

/// Exception::getFile() - Returns the filename where the exception was created
pub fn exception_get_file(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("getFile() called outside object context")?;

    let file_sym = vm.context.interner.intern(b"file");

    if let Val::Object(payload_handle) = &vm.arena.get(this_handle).value {
        if let Val::ObjPayload(obj_data) = &vm.arena.get(*payload_handle).value {
            if let Some(&file_handle) = obj_data.properties.get(&file_sym) {
                return Ok(file_handle);
            }
        }
    }

    // Return "unknown" if no file
    Ok(vm.arena.alloc(Val::String(Rc::new(b"unknown".to_vec()))))
}

/// Exception::getLine() - Returns the line where the exception was created
pub fn exception_get_line(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("getLine() called outside object context")?;

    let line_sym = vm.context.interner.intern(b"line");

    if let Val::Object(payload_handle) = &vm.arena.get(this_handle).value {
        if let Val::ObjPayload(obj_data) = &vm.arena.get(*payload_handle).value {
            if let Some(&line_handle) = obj_data.properties.get(&line_sym) {
                return Ok(line_handle);
            }
        }
    }

    // Return 0 if no line
    Ok(vm.arena.alloc(Val::Int(0)))
}

/// Exception::getTrace() - Returns the stack trace as array
pub fn exception_get_trace(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("getTrace() called outside object context")?;

    let trace_sym = vm.context.interner.intern(b"trace");

    if let Val::Object(payload_handle) = &vm.arena.get(this_handle).value {
        if let Val::ObjPayload(obj_data) = &vm.arena.get(*payload_handle).value {
            if let Some(&trace_handle) = obj_data.properties.get(&trace_sym) {
                return Ok(trace_handle);
            }
        }
    }

    // Return empty array if no trace
    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::new().into())))
}

/// Exception::getTraceAsString(): "#0 file(line): Class->method(args)" per
/// frame, then "#N {main}".
/// Reference: $PHP_SRC_PATH/Zend/zend_exceptions.c - _build_trace_string
pub fn exception_get_trace_as_string(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("getTraceAsString() called outside object context")?;
    let text = trace_string(vm, this_handle);
    Ok(vm.arena.alloc(Val::String(Rc::new(text.into_bytes()))))
}

/// A property of an exception object.
fn prop(vm: &mut VM, ex: Handle, name: &[u8]) -> Option<Handle> {
    let sym = vm.context.interner.intern(name);
    if let Val::Object(payload) = vm.arena.get(ex).value
        && let Val::ObjPayload(obj) = &vm.arena.get(payload).value
    {
        return obj.properties.get(&sym).copied();
    }
    None
}

fn trace_string(vm: &mut VM, ex: Handle) -> String {
    let mut out = String::new();
    let mut n = 0;
    let frames: Vec<Handle> = match prop(vm, ex, b"trace").map(|h| vm.arena.get(h).value.clone()) {
        Some(Val::Array(arr)) => arr.map.values().copied().collect(),
        _ => Vec::new(),
    };
    for frame in frames {
        let Val::Array(entry) = vm.arena.get(frame).value.clone() else { continue };
        let get = |key: &[u8]| entry.map.get(&crate::core::value::ArrayKey::Str(Rc::new(key.to_vec()))).copied();
        let text = |vm: &VM, h: Option<Handle>| h.map(|h| String::from_utf8_lossy(&vm.arena.get(h).value.to_php_string_bytes()).into_owned());
        out.push_str(&format!("#{} ", n));
        match text(vm, get(b"file")) {
            Some(file) => out.push_str(&format!("{}({}): ", file, text(vm, get(b"line")).unwrap_or_default())),
            None => out.push_str("[internal function]: "),
        }
        if let Some(class) = text(vm, get(b"class")) {
            out.push_str(&class);
            out.push_str(&text(vm, get(b"type")).unwrap_or_default());
        }
        out.push_str(&text(vm, get(b"function")).unwrap_or_default());
        out.push('(');
        if let Some(args) = get(b"args")
            && let Val::Array(args) = vm.arena.get(args).value.clone()
        {
            let parts: Vec<String> = args.map.values().map(|h| trace_arg(vm, *h)).collect();
            out.push_str(&parts.join(", "));
        }
        out.push_str(")\n");
        n += 1;
    }
    out.push_str(&format!("#{} {{main}}", n));
    out
}

/// An argument as a stack trace shows it.
fn trace_arg(vm: &VM, h: Handle) -> String {
    match &vm.arena.get(h).value {
        Val::Null => "NULL".into(),
        Val::Bool(b) => if *b { "true" } else { "false" }.into(),
        Val::Int(i) => i.to_string(),
        Val::Float(f) => crate::core::value::php_float_to_string(*f, crate::core::value::FloatPrecision::ECHO, 'E'),
        Val::String(s) => {
            // zend.exception_string_param_max_len = 15
            let shown = String::from_utf8_lossy(&s[..s.len().min(15)]).into_owned();
            format!("'{}{}'", shown, if s.len() > 15 { "..." } else { "" })
        }
        Val::Array(_) | Val::ConstArray(_) => "Array".into(),
        Val::Object(p) => match &vm.arena.get(*p).value {
            Val::ObjPayload(o) => format!(
                "Object({})",
                String::from_utf8_lossy(vm.context.interner.lookup(o.class).unwrap_or(b""))
            ),
            _ => "Object".into(),
        },
        _ => String::new(),
    }
}

/// Exception::getPrevious() - Returns previous exception
pub fn exception_get_previous(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("getPrevious() called outside object context")?;

    let previous_sym = vm.context.interner.intern(b"previous");

    if let Val::Object(payload_handle) = &vm.arena.get(this_handle).value {
        if let Val::ObjPayload(obj_data) = &vm.arena.get(*payload_handle).value {
            if let Some(&prev_handle) = obj_data.properties.get(&previous_sym) {
                return Ok(prev_handle);
            }
        }
    }

    // Return null if no previous
    Ok(vm.arena.alloc(Val::Null))
}

/// Exception::__toString(): "Class: message in file:line" and the stack
/// trace, for this exception and each previous one ("Next ...").
/// Reference: $PHP_SRC_PATH/Zend/zend_exceptions.c - ZEND_METHOD(Exception, __toString)
pub fn exception_to_string(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    let this_handle = vm
        .frames
        .last()
        .and_then(|f| f.this)
        .ok_or("__toString() called outside object context")?;

    // Innermost (oldest previous) first, as PHP prints them.
    let mut chain = Vec::new();
    let mut current = Some(this_handle);
    while let Some(ex) = current {
        if chain.contains(&ex) || chain.len() > 64 {
            break;
        }
        chain.push(ex);
        current = prop(vm, ex, b"previous").filter(|h| matches!(vm.arena.get(*h).value, Val::Object(_)));
    }

    let mut out = String::new();
    for ex in chain.into_iter().rev() {
        let class = match vm.arena.get(ex).value {
            Val::Object(p) => match &vm.arena.get(p).value {
                Val::ObjPayload(o) => String::from_utf8_lossy(vm.context.interner.lookup(o.class).unwrap_or(b"")).into_owned(),
                _ => String::new(),
            },
            _ => String::new(),
        };
        let text = |vm: &mut VM, name: &[u8]| {
            prop(vm, ex, name)
                .map(|h| String::from_utf8_lossy(&vm.arena.get(h).value.to_php_string_bytes()).into_owned())
                .unwrap_or_default()
        };
        let (message, file, line) = (text(vm, b"message"), text(vm, b"file"), text(vm, b"line"));
        let trace = trace_string(vm, ex);
        let this = if message.is_empty() {
            format!("{} in {}:{}\nStack trace:\n{}", class, file, line, trace)
        } else {
            format!("{}: {} in {}:{}\nStack trace:\n{}", class, message, file, line, trace)
        };
        if out.is_empty() {
            out = this;
        } else {
            out = format!("{}\n\nNext {}", out, this);
        }
    }
    Ok(vm.arena.alloc(Val::String(Rc::new(out.into_bytes()))))
}

