//! Stack traces as PHP shows them (getTraceAsString,
//! debug_print_backtrace): a trace array is the engine's
//! (Throwable::getTrace, debug_backtrace).
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_exceptions.c (_build_trace_string)

use crate::array::Key;
use crate::number::{Precision, float_to_string};
use crate::string::PhpStr;
use crate::value::{Kind, Value};

fn text(v: &Value) -> String {
    match v.to_str_view() {
        Some(s) => String::from_utf8_lossy(s.as_bytes()).into_owned(),
        None => String::new(),
    }
}

/// getTraceAsString: one line per frame, then {main}.
pub fn trace_string(trace: &Value) -> String {
    trace_string_within(trace, usize::MAX).unwrap_or_default()
}

/// trace_string(), in at most `room` bytes of text: past them, the size
/// the text grew to (an out-of-memory error's trace, made under the hard
/// limit).
pub fn trace_string_within(trace: &Value, room: usize) -> Result<String, usize> {
    let mut out = String::new();
    let mut n = 0;
    if let Some(frames) = trace.as_array() {
        for (_, frame) in frames.iter() {
            let frame = frame.deref();
            let Some(f) = frame.as_array() else { continue };
            let get = |k: &str| f.get(&Key::from_string(PhpStr::from(k))).map(Value::deref);
            out.push_str(&format!("#{n} "));
            match (get("file"), get("line")) {
                (Some(file), Some(line)) => out.push_str(&format!("{}({}): ", text(&file), line.to_int())),
                _ => out.push_str("[internal function]: "),
            }
            if let Some(class) = get("class") {
                out.push_str(&text(&class));
                out.push_str(&get("type").map(|t| text(&t)).unwrap_or_default());
            }
            out.push_str(&get("function").map(|f| text(&f)).unwrap_or_default());
            out.push('(');
            if let Some(args) = get("args").as_ref().and_then(Value::as_array) {
                let shown: Vec<String> = args.iter().map(|(_, a)| trace_arg(&a.deref())).collect();
                out.push_str(&shown.join(", "));
            }
            out.push_str(")\n");
            n += 1;
            if out.capacity() > room {
                return Err(out.capacity());
            }
        }
    }
    out.push_str(&format!("#{n} {{main}}"));
    if out.capacity() > room {
        return Err(out.capacity());
    }
    Ok(out)
}

/// An argument as a trace shows it (_build_trace_args): strings cut at
/// 15 bytes.
pub fn trace_arg(v: &Value) -> String {
    match v.kind() {
        Kind::Null | Kind::Undef => "NULL".into(),
        Kind::Bool => if v.as_bool().expect("a bool") { "true" } else { "false" }.into(),
        Kind::Int => v.as_int().expect("an int").to_string(),
        Kind::Float => float_to_string(v.as_float().expect("a float"), Precision::ECHO, 'E'),
        Kind::Str => {
            let bytes = v.as_str().expect("a string").as_bytes();
            let mut out = String::from("'");
            for &c in &bytes[..bytes.len().min(15)] {
                match c {
                    b'\n' => out.push_str("\\n"),
                    b'\r' => out.push_str("\\r"),
                    b'\t' => out.push_str("\\t"),
                    0x0c => out.push_str("\\f"),
                    0x0b => out.push_str("\\v"),
                    b'\\' => out.push_str("\\\\"),
                    0x1b => out.push_str("\\e"),
                    c if !(0x20..0x7f).contains(&c) => out.push_str(&format!("\\x{c:02X}")),
                    c => out.push(c as char),
                }
            }
            out.push_str(if bytes.len() > 15 { "...'" } else { "'" });
            out
        }
        Kind::Array => "Array".into(),
        Kind::Object => format!("Object({})", String::from_utf8_lossy(v.as_object().expect("an object").class.name.as_bytes())),
        Kind::Ref => trace_arg(&v.as_reference().expect("a reference").borrow()),
    }
}
