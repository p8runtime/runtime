//! v2 builtins: `fn(&mut Vm, &mut [Value]) -> Result<Value, Throw>`.
//! Arguments arrive by value (moved out of the caller's registers); a
//! by-reference parameter receives the variable's reference cell.
//!
//! This is the start of the port (milestone M5 moves the rest, module by
//! module, each with its php-src directories).

use super::array::{Key, PhpArray};
use super::string::PhpStr;
use super::value::Value;
use super::vm::{Throw, Vm};
use std::collections::HashMap;
use std::rc::Rc;

pub type Run = fn(&mut Vm, &mut [Value]) -> Result<Value, Throw>;

pub struct Builtin {
    pub name: &'static str,
    pub run: Run,
    /// Parameters passed by reference.
    pub by_ref: &'static [usize],
}

static BUILTINS: &[Builtin] = &[
    Builtin { name: "strlen", run: strlen, by_ref: &[] },
    Builtin { name: "str_repeat", run: str_repeat, by_ref: &[] },
    Builtin { name: "count", run: count, by_ref: &[] },
    Builtin { name: "dechex", run: dechex, by_ref: &[] },
    Builtin { name: "hrtime", run: hrtime, by_ref: &[] },
    Builtin { name: "microtime", run: microtime, by_ref: &[] },
    Builtin { name: "number_format", run: number_format, by_ref: &[] },
    Builtin { name: "printf", run: printf, by_ref: &[] },
    Builtin { name: "sprintf", run: sprintf, by_ref: &[] },
    Builtin { name: "ob_start", run: ob_start, by_ref: &[] },
    Builtin { name: "ob_end_clean", run: ob_end_clean, by_ref: &[] },
    Builtin { name: "flush", run: flush, by_ref: &[] },
    Builtin { name: "function_exists", run: function_exists, by_ref: &[] },
    Builtin { name: "define", run: define, by_ref: &[] },
    Builtin { name: "defined", run: defined, by_ref: &[] },
    Builtin { name: "date_default_timezone_set", run: date_default_timezone_set, by_ref: &[] },
    Builtin { name: "func_get_args", run: func_get_args, by_ref: &[] },
    Builtin { name: "var_dump", run: var_dump, by_ref: &[] },
    Builtin { name: "array_keys", run: array_keys, by_ref: &[] },
    Builtin { name: "range", run: range, by_ref: &[] },
];

thread_local! {
    static INDEX: HashMap<&'static str, usize> =
        BUILTINS.iter().enumerate().map(|(i, b)| (b.name, i)).collect();
}

/// The builtin named `name` (lowercase).
pub fn lookup(name: &[u8]) -> Option<usize> {
    let name = std::str::from_utf8(name).ok()?;
    INDEX.with(|index| index.get(name).copied())
}

pub fn entry(i: usize) -> &'static Builtin {
    &BUILTINS[i]
}

pub fn by_ref(i: usize, n: usize) -> bool {
    BUILTINS[i].by_ref.contains(&n)
}

/// The constants a request starts with: real PHP 8.4's (the table
/// examples/gen_defaults.php generates).
pub fn constants() -> HashMap<Vec<u8>, Value> {
    use crate::builtins::defaults::{CONSTANTS, ConstValue};
    let mut c = HashMap::with_capacity(CONSTANTS.len());
    for (_, name, value, _) in CONSTANTS {
        let value = match value {
            ConstValue::Int(i) => Value::Int(*i),
            ConstValue::Float(bits) => Value::Float(f64::from_bits(*bits)),
            ConstValue::Str(s) => Value::Str(PhpStr::from(*s)),
            ConstValue::Bool(b) => Value::Bool(*b),
            ConstValue::Null => Value::Null,
        };
        c.insert(name.to_vec(), value);
    }
    c
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(Value::deref).unwrap_or(Value::Null)
}

fn str_arg(args: &[Value], i: usize) -> PhpStr {
    arg(args, i).to_php_string().unwrap_or_else(PhpStr::empty)
}

fn strlen(_: &mut Vm, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(str_arg(args, 0).len() as i64))
}

fn str_repeat(_: &mut Vm, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let n = arg(args, 1).to_int().max(0) as usize;
    Ok(Value::Str(PhpStr::new(s.as_bytes().repeat(n))))
}

fn count(_: &mut Vm, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(match arg(args, 0) {
        Value::Array(a) => a.len() as i64,
        _ => 1,
    }))
}

fn dechex(_: &mut Vm, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::str(format!("{:x}", arg(args, 0).to_int() as u64)))
}

fn hrtime(_: &mut Vm, args: &mut [Value]) -> Result<Value, Throw> {
    thread_local!(static START: std::time::Instant = std::time::Instant::now());
    let ns = START.with(|s| s.elapsed().as_nanos()) as i64;
    if arg(args, 0).to_bool() {
        return Ok(Value::Int(ns));
    }
    Ok(Value::Array(Rc::new(PhpArray::from_list(vec![
        Value::Int(ns / 1_000_000_000),
        Value::Int(ns % 1_000_000_000),
    ]))))
}

fn microtime(_: &mut Vm, args: &mut [Value]) -> Result<Value, Throw> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    if arg(args, 0).to_bool() {
        return Ok(Value::Float(now.as_secs_f64()));
    }
    Ok(Value::str(format!("{:.8} {}", now.subsec_nanos() as f64 / 1e9, now.as_secs())))
}

/// number_format(float $num, int $decimals = 0, ?string $decimal_separator = ".", ?string $thousands_separator = ",")
fn number_format(_: &mut Vm, args: &mut [Value]) -> Result<Value, Throw> {
    let num = arg(args, 0).to_float();
    let decimals = arg(args, 1).to_int().max(0) as usize;
    let point = if args.len() > 2 { str_arg(args, 2).as_bytes().to_vec() } else { b".".to_vec() };
    let sep = if args.len() > 3 { str_arg(args, 3).as_bytes().to_vec() } else { b",".to_vec() };
    // _php_math_round: round half away from zero at `decimals` places
    let factor = 10f64.powi(decimals as i32);
    let rounded = (num.abs() * factor).round() / factor;
    let text = format!("{:.*}", decimals, rounded);
    let (int_part, frac) = text.split_once('.').unwrap_or((&text, ""));
    let mut out = Vec::new();
    if num < 0.0 && rounded != 0.0 {
        out.push(b'-');
    }
    for (i, d) in int_part.bytes().enumerate() {
        if i > 0 && (int_part.len() - i) % 3 == 0 {
            out.extend_from_slice(&sep);
        }
        out.push(d);
    }
    if decimals > 0 {
        out.extend_from_slice(&point);
        out.extend_from_slice(frac.as_bytes());
    }
    Ok(Value::Str(PhpStr::new(out)))
}

/// The common subset of sprintf: %s %d %u %x %X %o %b %c %e %f %F %%, with
/// flags, width and precision.
fn format(args: &[Value]) -> Vec<u8> {
    let fmt = str_arg(args, 0);
    let fmt = fmt.as_bytes();
    let mut out = Vec::new();
    let mut next = 1;
    let mut i = 0;
    while i < fmt.len() {
        if fmt[i] != b'%' {
            out.push(fmt[i]);
            i += 1;
            continue;
        }
        i += 1;
        if fmt.get(i) == Some(&b'%') {
            out.push(b'%');
            i += 1;
            continue;
        }
        let (mut left, mut plus, mut pad) = (false, false, b' ');
        while let Some(&c) = fmt.get(i) {
            match c {
                b'-' => left = true,
                b'+' => plus = true,
                b'0' => pad = b'0',
                b' ' => pad = b' ',
                b'\'' => {
                    i += 1;
                    pad = fmt.get(i).copied().unwrap_or(b' ');
                }
                _ => break,
            }
            i += 1;
        }
        let digits = |i: &mut usize| {
            let start = *i;
            while fmt.get(*i).is_some_and(u8::is_ascii_digit) {
                *i += 1;
            }
            std::str::from_utf8(&fmt[start..*i]).ok().and_then(|t| t.parse::<usize>().ok())
        };
        let width = digits(&mut i).unwrap_or(0);
        let precision = if fmt.get(i) == Some(&b'.') {
            i += 1;
            Some(digits(&mut i).unwrap_or(0))
        } else {
            None
        };
        let conv = fmt.get(i).copied().unwrap_or(b's');
        i += 1;
        let v = arg(args, next);
        next += 1;
        let mut body: Vec<u8> = match conv {
            b'd' | b'i' => {
                let n = v.to_int();
                if plus && n >= 0 { format!("+{n}") } else { n.to_string() }.into_bytes()
            }
            b'u' => (v.to_int() as u64).to_string().into_bytes(),
            b'x' => format!("{:x}", v.to_int() as u64).into_bytes(),
            b'X' => format!("{:X}", v.to_int() as u64).into_bytes(),
            b'o' => format!("{:o}", v.to_int() as u64).into_bytes(),
            b'b' => format!("{:b}", v.to_int() as u64).into_bytes(),
            b'c' => vec![v.to_int() as u8],
            b'f' | b'F' => {
                let f = v.to_float();
                let s = format!("{:.*}", precision.unwrap_or(6), f);
                if plus && f >= 0.0 { format!("+{s}") } else { s }.into_bytes()
            }
            b'e' | b'E' => {
                let s = format!("{:.*e}", precision.unwrap_or(6), v.to_float());
                let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
                let e: i32 = e.parse().unwrap_or(0);
                let mut t = format!("{m}e{}{}", if e < 0 { '-' } else { '+' }, e.abs());
                if conv == b'E' {
                    t = t.to_uppercase();
                }
                t.into_bytes()
            }
            _ => {
                let s = v.to_php_string().unwrap_or_else(|| PhpStr::from("Array"));
                let mut b = s.as_bytes().to_vec();
                if let Some(p) = precision {
                    b.truncate(p);
                }
                b
            }
        };
        if body.len() < width {
            let fill = vec![pad; width - body.len()];
            if left {
                body.extend_from_slice(&vec![b' '; width - body.len()]);
            } else if pad == b'0' && matches!(body.first(), Some(b'-' | b'+')) && matches!(conv, b'd' | b'f' | b'F' | b'e' | b'E') {
                let sign = body.remove(0);
                body = [vec![sign], fill, body].concat();
            } else {
                body = [fill, body].concat();
            }
        }
        out.extend_from_slice(&body);
    }
    out
}

fn printf(vm: &mut Vm, args: &mut [Value]) -> Result<Value, Throw> {
    let out = format(args);
    vm.out.write(&out);
    Ok(Value::Int(out.len() as i64))
}

fn sprintf(_: &mut Vm, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Str(PhpStr::new(format(args))))
}

fn ob_start(vm: &mut Vm, _: &mut [Value]) -> Result<Value, Throw> {
    vm.out.start_buffer();
    Ok(Value::Bool(true))
}

fn ob_end_clean(vm: &mut Vm, _: &mut [Value]) -> Result<Value, Throw> {
    if vm.out.end_clean() {
        return Ok(Value::Bool(true));
    }
    vm.warn("ob_end_clean(): Failed to delete buffer. No buffer to delete");
    Ok(Value::Bool(false))
}

fn flush(_: &mut Vm, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Null)
}

fn function_exists(vm: &mut Vm, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(vm.function_exists(str_arg(args, 0).as_bytes())))
}

fn define(vm: &mut Vm, args: &mut [Value]) -> Result<Value, Throw> {
    let name = str_arg(args, 0).as_bytes().to_vec();
    if vm.constants.contains_key(&name) {
        vm.warn(format!("Constant {} already defined", String::from_utf8_lossy(&name)));
        return Ok(Value::Bool(false));
    }
    vm.constants.insert(name, arg(args, 1));
    Ok(Value::Bool(true))
}

fn defined(vm: &mut Vm, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(vm.constants.contains_key(str_arg(args, 0).as_bytes())))
}

fn date_default_timezone_set(_: &mut Vm, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Bool(true))
}

fn func_get_args(vm: &mut Vm, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Array(Rc::new(PhpArray::from_list(vm.extra_args().to_vec()))))
}

/// range() for int bounds and step (the float and character forms come with the port).
fn range(vm: &mut Vm, args: &mut [Value]) -> Result<Value, Throw> {
    let (start, end) = (arg(args, 0).to_int(), arg(args, 1).to_int());
    let step = if args.len() > 2 { arg(args, 2).to_int().abs() } else { 1 };
    if step == 0 {
        let _ = vm;
        return Ok(Value::Array(Rc::new(PhpArray::from_list(vec![Value::Int(start)]))));
    }
    let values: Vec<Value> = if start <= end {
        (start..=end).step_by(step as usize).map(Value::Int).collect()
    } else {
        (end..=start).rev().step_by(step as usize).map(Value::Int).collect()
    };
    Ok(Value::Array(Rc::new(PhpArray::from_list(values))))
}

fn array_keys(_: &mut Vm, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(match arg(args, 0) {
        Value::Array(a) => Value::Array(Rc::new(PhpArray::from_list(a.iter().map(|(k, _)| k.to_value()).collect()))),
        _ => Value::Null,
    })
}

/// var_dump, for the types v2 has so far.
fn var_dump(vm: &mut Vm, args: &mut [Value]) -> Result<Value, Throw> {
    fn dump(v: &Value, indent: usize, out: &mut Vec<u8>) {
        let pad = " ".repeat(indent);
        match v {
            Value::Null | Value::Undef => out.extend_from_slice(format!("{pad}NULL\n").as_bytes()),
            Value::Bool(b) => out.extend_from_slice(format!("{pad}bool({b})\n").as_bytes()),
            Value::Int(i) => out.extend_from_slice(format!("{pad}int({i})\n").as_bytes()),
            Value::Float(f) => {
                let s = crate::core::value::php_float_to_string(*f, crate::core::value::FloatPrecision::serialize(), 'E');
                out.extend_from_slice(format!("{pad}float({s})\n").as_bytes());
            }
            Value::Str(s) => {
                out.extend_from_slice(format!("{pad}string({}) \"", s.len()).as_bytes());
                out.extend_from_slice(s.as_bytes());
                out.extend_from_slice(b"\"\n");
            }
            Value::Array(a) => {
                out.extend_from_slice(format!("{pad}array({}) {{\n", a.len()).as_bytes());
                for (k, v) in a.iter() {
                    match k {
                        Key::Int(i) => out.extend_from_slice(format!("{pad}  [{i}]=>\n").as_bytes()),
                        Key::Str(s) => {
                            out.extend_from_slice(format!("{pad}  [\"").as_bytes());
                            out.extend_from_slice(s.as_bytes());
                            out.extend_from_slice(b"\"]=>\n");
                        }
                    }
                    dump(v, indent + 2, out);
                }
                out.extend_from_slice(format!("{pad}}}\n").as_bytes());
            }
            Value::Ref(cell) => dump(&cell.borrow(), indent, out),
        }
    }
    let mut out = Vec::new();
    for v in args.iter() {
        dump(v, 0, &mut out);
    }
    vm.out.write(&out);
    Ok(Value::Null)
}
