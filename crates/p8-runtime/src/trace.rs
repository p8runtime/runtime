//! Stack traces as PHP shows them (getTraceAsString,
//! debug_print_backtrace): a trace array is the engine's
//! (Throwable::getTrace, debug_backtrace).
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_exceptions.c (_build_trace_string)

use crate::array::Key;
use crate::number::{Precision, float_to_string};
use crate::string::PhpStr;
use crate::value::Value;

fn text(v: &Value) -> String {
    String::from_utf8_lossy(v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes()).into_owned()
}

/// getTraceAsString: one line per frame, then {main}.
pub fn trace_string(trace: &Value) -> String {
    let mut out = String::new();
    let mut n = 0;
    if let Value::Array(frames) = trace {
        for (_, frame) in frames.iter() {
            let Value::Array(f) = frame.deref() else { continue };
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
            if let Some(Value::Array(args)) = get("args") {
                let shown: Vec<String> = args.iter().map(|(_, a)| trace_arg(&a.deref())).collect();
                out.push_str(&shown.join(", "));
            }
            out.push_str(")\n");
            n += 1;
        }
    }
    out.push_str(&format!("#{n} {{main}}"));
    out
}

/// An argument as a trace shows it (_build_trace_args): strings cut at
/// 15 bytes.
pub fn trace_arg(v: &Value) -> String {
    match v {
        Value::Null | Value::Undef => "NULL".into(),
        Value::Bool(b) => if *b { "true" } else { "false" }.into(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => float_to_string(*f, Precision::ECHO, 'E'),
        Value::Str(s) => {
            let bytes = s.as_bytes();
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
        Value::Array(_) => "Array".into(),
        Value::Object(o) => format!("Object({})", String::from_utf8_lossy(o.class.name.as_bytes())),
        Value::Ref(cell) => trace_arg(&cell.borrow()),
    }
}
