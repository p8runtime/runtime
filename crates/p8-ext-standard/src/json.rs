//! ext/json: json_encode, json_decode, json_last_error(_msg),
//! json_validate. JsonSerializable objects are asked for their value.
//!
//! Reference: $PHP_SRC_PATH/ext/json/json_encoder.c,
//! $PHP_SRC_PATH/ext/json/json_parser.y, $PHP_SRC_PATH/ext/json/json_scanner.re

use crate::{arg, str_arg};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::number::{Precision, float_to_string};
use p8_runtime::object::{CLASS_ENUM, CLASS_RESOURCE, Object, Visibility};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "json_encode", run: json_encode, by_ref: &[] },
    Builtin { name: "json_decode", run: json_decode, by_ref: &[] },
    Builtin { name: "json_last_error", run: json_last_error, by_ref: &[] },
    Builtin { name: "json_last_error_msg", run: json_last_error_msg, by_ref: &[] },
    Builtin { name: "json_validate", run: json_validate, by_ref: &[] },
];

const HEX_TAG: i64 = 1;
const HEX_AMP: i64 = 2;
const HEX_APOS: i64 = 4;
const HEX_QUOT: i64 = 8;
const FORCE_OBJECT: i64 = 16;
const NUMERIC_CHECK: i64 = 32;
const UNESCAPED_SLASHES: i64 = 64;
const PRETTY_PRINT: i64 = 128;
const UNESCAPED_UNICODE: i64 = 256;
const PARTIAL_OUTPUT_ON_ERROR: i64 = 512;
const PRESERVE_ZERO_FRACTION: i64 = 1024;
const UNESCAPED_LINE_TERMINATORS: i64 = 2048;
const OBJECT_AS_ARRAY: i64 = 1;
const BIGINT_AS_STRING: i64 = 2;
const INVALID_UTF8_IGNORE: i64 = 1_048_576;
const INVALID_UTF8_SUBSTITUTE: i64 = 2_097_152;
const THROW_ON_ERROR: i64 = 4_194_304;

/// json_last_error() codes and their messages.
fn message(code: i64) -> &'static str {
    match code {
        0 => "No error",
        1 => "Maximum stack depth exceeded",
        2 => "State mismatch (invalid or malformed JSON)",
        3 => "Control character error, possibly incorrectly encoded",
        4 => "Syntax error",
        5 => "Malformed UTF-8 characters, possibly incorrectly encoded",
        6 => "Recursion detected",
        7 => "Inf and NaN cannot be JSON encoded",
        8 => "Type is not supported",
        9 => "The decoded property name is invalid",
        10 => "Single unpaired UTF-16 surrogate in unicode escape",
        11 => "Non-backed enums have no default serialization",
        _ => "Unknown error",
    }
}

/// Record the outcome; with JSON_THROW_ON_ERROR an error is a JsonException
/// (and the last error is left as it was).
fn finish(ctx: &mut dyn Ctx, flags: i64, code: i64, ok: Value) -> Result<Value, Throw> {
    if flags & THROW_ON_ERROR != 0 {
        if code != 0 {
            return Err(json_exception(ctx, code));
        }
        return Ok(ok);
    }
    *ctx.state("json_error") = Value::Int(code);
    Ok(ok)
}

/// A JsonException (its code is not set yet: engine errors carry none).
fn json_exception(ctx: &mut dyn Ctx, code: i64) -> Throw {
    ctx.throw("JsonException", message(code).to_string())
}

struct Encoder<'c> {
    ctx: &'c mut dyn Ctx,
    flags: i64,
    depth: usize,
    max_depth: usize,
    error: i64,
    seen: Vec<*const Object>,
}

impl Encoder<'_> {
    fn fail(&mut self, code: i64) {
        if self.error == 0 {
            self.error = code;
        }
    }

    fn newline(&self, out: &mut Vec<u8>) {
        if self.flags & PRETTY_PRINT != 0 {
            out.push(b'\n');
            out.extend(std::iter::repeat_n(b' ', self.depth * 4));
        }
    }

    fn value(&mut self, v: &Value, out: &mut Vec<u8>) -> Result<(), Throw> {
        match v {
            Value::Null | Value::Undef => out.extend_from_slice(b"null"),
            Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
            Value::Int(i) => out.extend_from_slice(i.to_string().as_bytes()),
            Value::Float(f) => self.float(*f, out),
            Value::Str(s) => self.string(s.as_bytes(), out),
            Value::Ref(cell) => {
                let inner = cell.borrow().clone();
                self.value(&inner, out)?;
            }
            Value::Array(a) => {
                let list = a.is_list() && self.flags & FORCE_OBJECT == 0;
                let entries: Vec<(Key, Value)> = a.iter().map(|(k, v)| (k, v.clone())).collect();
                self.container(entries, list, out)?;
            }
            Value::Object(o) => self.object(o, out)?,
        }
        Ok(())
    }

    fn float(&mut self, f: f64, out: &mut Vec<u8>) {
        if !f.is_finite() {
            self.fail(7);
            out.push(b'0');
            return;
        }
        let text = float_to_string(f, Precision::SERIALIZE, 'e');
        out.extend_from_slice(text.as_bytes());
        if self.flags & PRESERVE_ZERO_FRACTION != 0 && !text.contains(['.', 'e']) {
            out.extend_from_slice(b".0");
        }
    }

    fn string(&mut self, s: &[u8], out: &mut Vec<u8>) {
        if self.flags & NUMERIC_CHECK != 0
            && let Some(n) = p8_runtime::numeric::numeric_string(s)
        {
            match n {
                p8_runtime::numeric::Number::Int(i) => out.extend_from_slice(i.to_string().as_bytes()),
                p8_runtime::numeric::Number::Float(f) => self.float(f, out),
            }
            return;
        }
        let text = match std::str::from_utf8(s) {
            Ok(t) => std::borrow::Cow::Borrowed(t),
            Err(_) if self.flags & INVALID_UTF8_SUBSTITUTE != 0 => String::from_utf8_lossy(s),
            Err(_) if self.flags & INVALID_UTF8_IGNORE != 0 => {
                std::borrow::Cow::Owned(String::from_utf8_lossy(s).replace('\u{FFFD}', ""))
            }
            Err(_) => {
                self.fail(5);
                out.extend_from_slice(b"null");
                return;
            }
        };
        out.push(b'"');
        for c in text.chars() {
            match c {
                '"' if self.flags & HEX_QUOT != 0 => out.extend_from_slice(b"\\u0022"),
                '"' => out.extend_from_slice(b"\\\""),
                '\\' => out.extend_from_slice(b"\\\\"),
                '/' if self.flags & UNESCAPED_SLASHES == 0 => out.extend_from_slice(b"\\/"),
                '\u{8}' => out.extend_from_slice(b"\\b"),
                '\u{c}' => out.extend_from_slice(b"\\f"),
                '\n' => out.extend_from_slice(b"\\n"),
                '\r' => out.extend_from_slice(b"\\r"),
                '\t' => out.extend_from_slice(b"\\t"),
                '<' if self.flags & HEX_TAG != 0 => out.extend_from_slice(b"\\u003C"),
                '>' if self.flags & HEX_TAG != 0 => out.extend_from_slice(b"\\u003E"),
                '&' if self.flags & HEX_AMP != 0 => out.extend_from_slice(b"\\u0026"),
                '\'' if self.flags & HEX_APOS != 0 => out.extend_from_slice(b"\\u0027"),
                '\u{2028}' | '\u{2029}' if self.flags & UNESCAPED_UNICODE != 0 && self.flags & UNESCAPED_LINE_TERMINATORS == 0 => {
                    out.extend_from_slice(format!("\\u{:04x}", c as u32).as_bytes());
                }
                c if (c as u32) < 0x20 => out.extend_from_slice(format!("\\u{:04x}", c as u32).as_bytes()),
                c if (c as u32) < 0x80 || self.flags & UNESCAPED_UNICODE != 0 => {
                    let mut buf = [0; 4];
                    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                }
                c => {
                    let mut units = [0u16; 2];
                    for u in c.encode_utf16(&mut units) {
                        out.extend_from_slice(format!("\\u{u:04x}").as_bytes());
                    }
                }
            }
        }
        out.push(b'"');
    }

    fn container(&mut self, entries: Vec<(Key, Value)>, list: bool, out: &mut Vec<u8>) -> Result<(), Throw> {
        if self.depth >= self.max_depth {
            self.fail(1);
        }
        let (open, close) = if list { (b'[', b']') } else { (b'{', b'}') };
        out.push(open);
        if entries.is_empty() {
            out.push(close);
            return Ok(());
        }
        self.depth += 1;
        for (i, (k, v)) in entries.iter().enumerate() {
            if i > 0 {
                out.push(b',');
            }
            self.newline(out);
            if !list {
                let key = match k {
                    Key::Int(n) => PhpStr::from(n.to_string()),
                    Key::Str(s) => s.clone(),
                };
                self.string_key(key.as_bytes(), out);
                out.push(b':');
                if self.flags & PRETTY_PRINT != 0 {
                    out.push(b' ');
                }
            }
            self.value(v, out)?;
        }
        self.depth -= 1;
        self.newline(out);
        out.push(close);
        Ok(())
    }

    fn string_key(&mut self, s: &[u8], out: &mut Vec<u8>) {
        let saved = self.flags;
        self.flags &= !NUMERIC_CHECK;
        self.string(s, out);
        self.flags = saved;
    }

    fn object(&mut self, o: &Rc<Object>, out: &mut Vec<u8>) -> Result<(), Throw> {
        let lazy;
        let o = if o.lazy.get() != 0 {
            lazy = self.ctx.lazy_initialize(o)?;
            &lazy
        } else {
            o
        };
        if o.class.flags & CLASS_RESOURCE != 0 {
            self.fail(8);
            out.extend_from_slice(b"null");
            return Ok(());
        }
        if o.class.flags & CLASS_ENUM != 0 {
            match &o.class.enum_backing {
                Some(_) => {
                    let v = o.class.read(o, b"value", &o.class.lower);
                    return self.value(&v, out);
                }
                None => {
                    self.fail(11);
                    out.push(b'0');
                    return Ok(());
                }
            }
        }
        let ptr = Rc::as_ptr(o);
        if self.seen.contains(&ptr) {
            self.fail(6);
            out.extend_from_slice(b"null");
            return Ok(());
        }
        if o.class.is_a(b"jsonserializable") {
            let v = self.ctx.call(&callable(o, b"jsonSerialize"), &mut [])?;
            self.seen.push(ptr);
            let r = self.value(&v, out);
            self.seen.pop();
            return r;
        }
        // A native class's own view of its properties (SimpleXML).
        if let Some(props) = o.class.get_properties {
            let entries = props(o).iter().map(|(k, v)| (k, v.clone())).collect();
            self.seen.push(ptr);
            let r = self.container(entries, false, out);
            self.seen.pop();
            return r;
        }
        // Public properties (initialized), then dynamic ones.
        let mut entries = Vec::new();
        for (i, p) in o.class.properties.iter().enumerate() {
            if p.visibility != Visibility::Public || p.is_internal() {
                continue;
            }
            // A hooked property through its get hook (a virtual one too).
            let v = if p.hooks == 0 { Some(o.slots.borrow()[i].clone()).filter(|v| !matches!(v, Value::Undef)) } else { self.ctx.prop_value(o, i)? };
            if let Some(v) = v {
                entries.push((Key::from_string(p.name.clone()), v));
            }
        }
        for (k, v) in o.dynamic.borrow().iter() {
            entries.push((k, v.clone()));
        }
        self.seen.push(ptr);
        let r = self.container(entries, false, out);
        self.seen.pop();
        r
    }
}

fn callable(o: &Rc<Object>, method: &[u8]) -> Value {
    Value::Array(Rc::new(PhpArray::from_list(vec![Value::Object(Rc::clone(o)), Value::Str(PhpStr::from(method))])))
}

/// json_encode(mixed $value, int $flags = 0, int $depth = 512)
fn json_encode(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let v = arg(args, 0);
    let flags = if args.len() > 1 { arg(args, 1).to_int() } else { 0 };
    let max_depth = if args.len() > 2 { arg(args, 2).to_int() } else { 512 };
    if max_depth <= 0 {
        return Err(ctx.throw("ValueError", "json_encode(): Argument #3 ($depth) must be greater than 0".into()));
    }
    let mut out = Vec::new();
    let mut e = Encoder { ctx, flags, depth: 0, max_depth: max_depth as usize, error: 0, seen: Vec::new() };
    e.value(&v, &mut out)?;
    let error = e.error;
    let ok = if error != 0 && flags & PARTIAL_OUTPUT_ON_ERROR == 0 { Value::Bool(false) } else { Value::Str(PhpStr::new(out)) };
    finish(ctx, flags, error, ok)
}

struct Parser<'a> {
    s: &'a [u8],
    at: usize,
    assoc: bool,
    bigint_string: bool,
    depth: usize,
    max_depth: usize,
}

enum Fail {
    Code(i64),
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.at < self.s.len() && matches!(self.s[self.at], b' ' | b'\t' | b'\n' | b'\r') {
            self.at += 1;
        }
    }

    fn value(&mut self, ctx: &mut dyn Ctx) -> Result<Value, Fail> {
        self.ws();
        let Some(&c) = self.s.get(self.at) else { return Err(Fail::Code(4)) };
        match c {
            b'{' => {
                self.depth += 1;
                if self.depth > self.max_depth {
                    return Err(Fail::Code(1));
                }
                self.at += 1;
                // The object is made before its members (PHP numbers it first).
                let object = if self.assoc {
                    None
                } else {
                    let Some(class) = ctx.class(b"stdClass") else { return Err(Fail::Code(8)) };
                    Some(ctx.new_object(&class))
                };
                let mut entries: Vec<(PhpStr, Value)> = Vec::new();
                self.ws();
                if self.s.get(self.at) == Some(&b'}') {
                    self.at += 1;
                } else {
                    loop {
                        self.ws();
                        if self.s.get(self.at) != Some(&b'"') {
                            return Err(Fail::Code(4));
                        }
                        let key = self.string()?;
                        self.ws();
                        if self.s.get(self.at) != Some(&b':') {
                            return Err(Fail::Code(4));
                        }
                        self.at += 1;
                        let v = self.value(ctx)?;
                        entries.push((key, v));
                        self.ws();
                        match self.s.get(self.at) {
                            Some(b',') => self.at += 1,
                            Some(b'}') => {
                                self.at += 1;
                                break;
                            }
                            _ => return Err(Fail::Code(4)),
                        }
                    }
                }
                self.depth -= 1;
                if self.assoc {
                    let mut a = PhpArray::new();
                    for (k, v) in entries {
                        a.insert(Key::from_string(k), v);
                    }
                    return Ok(Value::Array(Rc::new(a)));
                }
                let o = object.expect("made at the brace");
                for (k, v) in entries {
                    if k.as_bytes().first() == Some(&0) {
                        return Err(Fail::Code(9));
                    }
                    o.dynamic.borrow_mut().insert(Key::Str(k), v);
                }
                Ok(Value::Object(o))
            }
            b'[' => {
                self.depth += 1;
                if self.depth > self.max_depth {
                    return Err(Fail::Code(1));
                }
                self.at += 1;
                let mut items = Vec::new();
                self.ws();
                if self.s.get(self.at) == Some(&b']') {
                    self.at += 1;
                } else {
                    loop {
                        items.push(self.value(ctx)?);
                        self.ws();
                        match self.s.get(self.at) {
                            Some(b',') => self.at += 1,
                            Some(b']') => {
                                self.at += 1;
                                break;
                            }
                            _ => return Err(Fail::Code(4)),
                        }
                    }
                }
                self.depth -= 1;
                Ok(Value::Array(Rc::new(PhpArray::from_list(items))))
            }
            b'"' => Ok(Value::Str(self.string()?)),
            b't' if self.s[self.at..].starts_with(b"true") => {
                self.at += 4;
                Ok(Value::Bool(true))
            }
            b'f' if self.s[self.at..].starts_with(b"false") => {
                self.at += 5;
                Ok(Value::Bool(false))
            }
            b'n' if self.s[self.at..].starts_with(b"null") => {
                self.at += 4;
                Ok(Value::Null)
            }
            b'-' | b'0'..=b'9' => self.number(),
            _ => Err(Fail::Code(4)),
        }
    }

    fn number(&mut self) -> Result<Value, Fail> {
        let start = self.at;
        if self.s.get(self.at) == Some(&b'-') {
            self.at += 1;
        }
        let digits = |p: &mut Self| {
            let from = p.at;
            while p.at < p.s.len() && p.s[p.at].is_ascii_digit() {
                p.at += 1;
            }
            p.at - from
        };
        let int_start = self.at;
        if digits(self) == 0 {
            return Err(Fail::Code(4));
        }
        if self.s[int_start] == b'0' && self.at - int_start > 1 {
            return Err(Fail::Code(4));
        }
        let mut float = false;
        if self.s.get(self.at) == Some(&b'.') {
            self.at += 1;
            float = true;
            if digits(self) == 0 {
                return Err(Fail::Code(4));
            }
        }
        if matches!(self.s.get(self.at), Some(b'e' | b'E')) {
            self.at += 1;
            float = true;
            if matches!(self.s.get(self.at), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if digits(self) == 0 {
                return Err(Fail::Code(4));
            }
        }
        let text = std::str::from_utf8(&self.s[start..self.at]).unwrap_or("0");
        if !float {
            if let Ok(i) = text.parse::<i64>() {
                return Ok(Value::Int(i));
            }
            if self.bigint_string {
                return Ok(Value::Str(PhpStr::from(text)));
            }
        }
        Ok(Value::Float(text.parse::<f64>().unwrap_or(0.0)))
    }

    fn hex4(&mut self) -> Result<u32, Fail> {
        let h = self.s.get(self.at..self.at + 4).ok_or(Fail::Code(4))?;
        let h = std::str::from_utf8(h).map_err(|_| Fail::Code(4))?;
        let v = u32::from_str_radix(h, 16).map_err(|_| Fail::Code(4))?;
        self.at += 4;
        Ok(v)
    }

    fn string(&mut self) -> Result<PhpStr, Fail> {
        self.at += 1;
        let mut out = Vec::new();
        loop {
            let Some(&c) = self.s.get(self.at) else { return Err(Fail::Code(4)) };
            self.at += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let Some(&e) = self.s.get(self.at) else { return Err(Fail::Code(4)) };
                    self.at += 1;
                    match e {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let mut cp = self.hex4()?;
                            if (0xD800..0xDC00).contains(&cp) {
                                if self.s.get(self.at..self.at + 2) != Some(b"\\u") {
                                    return Err(Fail::Code(10));
                                }
                                self.at += 2;
                                let low = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&low) {
                                    return Err(Fail::Code(10));
                                }
                                cp = 0x10000 + ((cp - 0xD800) << 10) + (low - 0xDC00);
                            } else if (0xDC00..0xE000).contains(&cp) {
                                return Err(Fail::Code(10));
                            }
                            let ch = char::from_u32(cp).ok_or(Fail::Code(10))?;
                            let mut buf = [0; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        _ => return Err(Fail::Code(4)),
                    }
                }
                c if c < 0x20 => return Err(Fail::Code(3)),
                c => out.push(c),
            }
        }
        if std::str::from_utf8(&out).is_err() {
            return Err(Fail::Code(5));
        }
        Ok(PhpStr::new(out))
    }
}

/// json_decode(string $json, ?bool $associative = null, int $depth = 512, int $flags = 0)
fn json_decode(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let flags = if args.len() > 3 { arg(args, 3).to_int() } else { 0 };
    let assoc = match args.get(1).map(Value::deref) {
        Some(Value::Null) | None => flags & OBJECT_AS_ARRAY != 0,
        Some(v) => v.to_bool(),
    };
    let depth = if args.len() > 2 { arg(args, 2).to_int() } else { 512 };
    if depth <= 0 {
        return Err(ctx.throw("ValueError", "json_decode(): Argument #3 ($depth) must be greater than 0".into()));
    }
    if s.is_empty() {
        return finish(ctx, flags, 4, Value::Null);
    }
    let mut p = Parser { s: s.as_bytes(), at: 0, assoc, bigint_string: flags & BIGINT_AS_STRING != 0, depth: 0, max_depth: depth as usize };
    let r = p.value(ctx);
    match r {
        Ok(v) => {
            p.ws();
            if p.at != p.s.len() {
                return finish(ctx, flags, 4, Value::Null);
            }
            finish(ctx, flags, 0, v)
        }
        Err(Fail::Code(code)) => finish(ctx, flags, code, Value::Null),
    }
}

fn json_validate(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let mut p = Parser { s: s.as_bytes(), at: 0, assoc: true, bigint_string: true, depth: 0, max_depth: 512 };
    let ok = !s.is_empty() && p.value(ctx).is_ok() && {
        p.ws();
        p.at == p.s.len()
    };
    *ctx.state("json_error") = Value::Int(if ok { 0 } else { 4 });
    Ok(Value::Bool(ok))
}

fn json_last_error(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(ctx.state("json_error").to_int()))
}

fn json_last_error_msg(ctx: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    let code = ctx.state("json_error").to_int();
    Ok(Value::str(message(code)))
}
