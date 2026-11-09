//! The mb_* functions.
//!
//! Reference: $PHP_SRC_PATH/ext/mbstring/mbstring.c

use crate::enc::{self, BAD, Enc, Kind, Subst};
use crate::unicode::{self, Mode, Turkish};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::string::PhpStr;
use p8_runtime::value::{Kind as ValueKind, Value};

pub type R = Result<Value, Throw>;

macro_rules! b {
    ($name:literal, $run:expr) => {
        Builtin { name: $name, run: $run, by_ref: &[] }
    };
    ($name:literal, $run:expr, $refs:expr) => {
        Builtin { name: $name, run: $run, by_ref: $refs }
    };
}

pub static BUILTINS: &[Builtin] = &[
    b!("mb_language", mb_language),
    b!("mb_internal_encoding", mb_internal_encoding),
    b!("mb_http_input", mb_http_input),
    b!("mb_http_output", mb_http_output),
    b!("mb_detect_order", mb_detect_order),
    b!("mb_substitute_character", mb_substitute_character),
    b!("mb_preferred_mime_name", mb_preferred_mime_name),
    b!("mb_parse_str", crate::mime::mb_parse_str, &[1]),
    b!("mb_output_handler", crate::mime::mb_output_handler),
    b!("mb_str_split", mb_str_split),
    b!("mb_strlen", mb_strlen),
    b!("mb_strpos", mb_strpos),
    b!("mb_strrpos", mb_strrpos),
    b!("mb_stripos", mb_stripos),
    b!("mb_strripos", mb_strripos),
    b!("mb_strstr", mb_strstr),
    b!("mb_strrchr", mb_strrchr),
    b!("mb_stristr", mb_stristr),
    b!("mb_strrichr", mb_strrichr),
    b!("mb_substr_count", mb_substr_count),
    b!("mb_substr", mb_substr),
    b!("mb_strcut", mb_strcut),
    b!("mb_strwidth", mb_strwidth),
    b!("mb_strimwidth", mb_strimwidth),
    b!("mb_convert_encoding", mb_convert_encoding),
    b!("mb_convert_case", mb_convert_case),
    b!("mb_strtoupper", mb_strtoupper),
    b!("mb_strtolower", mb_strtolower),
    b!("mb_ucfirst", mb_ucfirst),
    b!("mb_lcfirst", mb_lcfirst),
    b!("mb_trim", mb_trim),
    b!("mb_ltrim", mb_ltrim),
    b!("mb_rtrim", mb_rtrim),
    b!("mb_detect_encoding", mb_detect_encoding),
    b!("mb_list_encodings", mb_list_encodings),
    b!("mb_encoding_aliases", mb_encoding_aliases),
    b!("mb_encode_mimeheader", crate::mime::mb_encode_mimeheader),
    b!("mb_decode_mimeheader", crate::mime::mb_decode_mimeheader),
    b!("mb_convert_variables", mb_convert_variables, &[2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]),
    b!("mb_encode_numericentity", mb_encode_numericentity),
    b!("mb_decode_numericentity", mb_decode_numericentity),
    b!("mb_get_info", mb_get_info),
    b!("mb_check_encoding", mb_check_encoding),
    b!("mb_scrub", mb_scrub),
    b!("mb_ord", mb_ord),
    b!("mb_chr", mb_chr),
    b!("mb_str_pad", mb_str_pad),
    b!("mb_regex_encoding", mb_regex_encoding),
    b!("mb_regex_set_options", mb_regex_set_options),
];

// ---- arguments and settings ----

/// A ValueError for argument `n`: "f(): Argument #n ($name) message".
pub fn arg_error(ctx: &dyn Ctx, n: usize, message: &str) -> Throw {
    let f = ctx.builtin_name();
    let param = p8_stubs::lookup(f.as_bytes()).and_then(|s| s.params.get(n - 1)).map_or("", |p| p.name);
    ctx.throw("ValueError", format!("{f}(): Argument #{n} (${param}) {message}"))
}

pub fn str_arg(args: &[Value], i: usize) -> Vec<u8> {
    match args.get(i) {
        Some(v) => {
            let v = v.deref();
            match v.as_str() {
                Some(s) => s.as_bytes().to_vec(),
                None => v.to_php_string().map(|s| s.as_bytes().to_vec()).unwrap_or_default(),
            }
        }
        None => Vec::new(),
    }
}

pub fn opt_str(args: &[Value], i: usize) -> Option<Vec<u8>> {
    match args.get(i).map(Value::deref) {
        Some(v) if !v.is_null_or_undef() => Some(v.to_php_string().map(|s| s.as_bytes().to_vec()).unwrap_or_default()),
        _ => None,
    }
}

pub fn int_arg(args: &[Value], i: usize, default: i64) -> i64 {
    match args.get(i).map(Value::deref) {
        Some(v) if !v.is_null_or_undef() => v.to_int(),
        _ => default,
    }
}

pub fn bool_arg(args: &[Value], i: usize) -> bool {
    args.get(i).is_some_and(|v| v.deref().to_bool())
}

/// A C string's worth of bytes (a message's %s stops at a NUL).
pub fn text(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

pub fn out(bytes: Vec<u8>) -> R {
    Ok(Value::str(PhpStr::new(bytes)))
}

pub fn strict_detection(ctx: &dyn Ctx) -> bool {
    ini_text(ctx, "mbstring.strict_detection").is_some_and(|v| v != "0" && !v.eq_ignore_ascii_case("off") && !v.eq_ignore_ascii_case("false"))
}

fn ini_text(ctx: &dyn Ctx, name: &str) -> Option<String> {
    match ctx.ini_get(name.as_bytes())?.as_str() {
        Some(s) if !s.is_empty() => Some(text(s.as_bytes())),
        _ => None,
    }
}

/// The internal encoding: mb_internal_encoding()'s, else the ini setting,
/// else default_charset.
pub fn internal(ctx: &mut dyn Ctx) -> &'static Enc {
    if let Some(s) = ctx.state("mb_internal").as_str()
        && let Some(e) = enc::lookup(&text(s.as_bytes())) {
            return e;
        }
    for ini in ["mbstring.internal_encoding", "default_charset"] {
        if let Some(e) = ini_text(ctx, ini).and_then(|n| enc::lookup(&n)) {
            return e;
        }
    }
    enc::utf8()
}

/// The encoding named by argument `i` (null or missing: the internal one),
/// with the deprecations for the transfer encodings.
pub fn encoding_arg(ctx: &mut dyn Ctx, args: &[Value], i: usize) -> Result<&'static Enc, Throw> {
    match opt_str(args, i) {
        None => Ok(internal(ctx)),
        Some(name) => {
            let name = text(&name);
            match enc::lookup(&name) {
                None => Err(arg_error(ctx, i + 1, &format!("must be a valid encoding, \"{name}\" given"))),
                Some(e) => {
                    let msg = match e.kind {
                        Kind::Base64 => Some("Handling Base64 via mbstring is deprecated; use base64_encode/base64_decode instead"),
                        Kind::Qprint => Some("Handling QPrint via mbstring is deprecated; use quoted_printable_encode/quoted_printable_decode instead"),
                        Kind::HtmlEnt => Some("Handling HTML entities via mbstring is deprecated; use htmlspecialchars, htmlentities, or mb_encode_numericentity/mb_decode_numericentity instead"),
                        Kind::Uuencode => Some("Handling Uuencode via mbstring is deprecated; use convert_uuencode/convert_uudecode instead"),
                        _ => None,
                    };
                    if let Some(m) = msg {
                        let f = ctx.builtin_name();
                        ctx.deprecated(&format!("{f}(): {m}"));
                    }
                    Ok(e)
                }
            }
        }
    }
}

/// How characters an encoding can't have are replaced.
pub fn subst(ctx: &mut dyn Ctx) -> Subst {
    let mode = ctx.state("mb_mode").clone();
    if let Some(m) = mode.as_int() {
        let ch = match ctx.state("mb_char").as_int() {
            Some(c) => c as u32,
            None => b'?' as u32,
        };
        return Subst { mode: m as u8, ch };
    }
    if let Some(v) = ini_text(ctx, "mbstring.substitute_character") {
        match v.to_ascii_lowercase().as_str() {
            "none" => return Subst { mode: 1, ch: b'?' as u32 },
            "long" => return Subst { mode: 2, ch: b'?' as u32 },
            "entity" => return Subst { mode: 3, ch: b'?' as u32 },
            n => {
                if let Ok(c) = n.parse::<u32>() {
                    return Subst { mode: 0, ch: c };
                }
            }
        }
    }
    Subst::DEFAULT
}

fn count_illegal(ctx: &mut dyn Ctx, n: usize) {
    if n > 0 {
        let c = ctx.state("mb_illegal");
        *c = Value::int(c.to_int() + n as i64);
    }
}

/// Characters to this encoding's bytes, with the current substitution.
pub fn encode_out(ctx: &mut dyn Ctx, e: &Enc, cps: &[u32]) -> Vec<u8> {
    let sub = subst(ctx);
    let mut errs = 0;
    let bytes = enc::encode(e, cps, &sub, &mut errs);
    count_illegal(ctx, errs);
    bytes
}

/// Convert bytes between encodings; the transfer encodings go through
/// 8-bit bytes the way libmbfl does.
pub fn convert(ctx: &mut dyn Ctx, s: &[u8], to: &Enc, from: &Enc) -> Vec<u8> {
    let (from, to) = if matches!(to.kind, Kind::Base64 | Kind::Qprint) {
        (enc::lookup("8bit").unwrap(), to)
    } else if from.is_transfer() {
        (from, enc::lookup("8bit").unwrap())
    } else {
        (from, to)
    };
    if to.kind == Kind::Bit8 && from.kind == Kind::Bit8 {
        return s.to_vec();
    }
    let cps = enc::decode(from, s);
    encode_out(ctx, to, &cps)
}

// ---- settings functions ----

const LANGUAGES: &[(&str, &str, &[&str])] = &[
    ("uni", "uni", &["universal"]),
    ("Japanese", "ja", &[]),
    ("Korean", "ko", &[]),
    ("Simplified Chinese", "zh-cn", &[]),
    ("Traditional Chinese", "zh-tw", &[]),
    ("English", "en", &[]),
    ("German", "de", &["Deutsch"]),
    ("Russian", "ru", &[]),
    ("Ukrainian", "ua", &[]),
    ("Armenian", "hy", &[]),
    ("Turkish", "tr", &[]),
    ("neutral", "neutral", &[]),
];

fn language_named(name: &str) -> Option<&'static str> {
    LANGUAGES
        .iter()
        .find(|l| l.0.eq_ignore_ascii_case(name))
        .or_else(|| LANGUAGES.iter().find(|l| l.1.eq_ignore_ascii_case(name)))
        .or_else(|| LANGUAGES.iter().find(|l| l.2.iter().any(|a| a.eq_ignore_ascii_case(name))))
        .map(|l| l.0)
}

pub fn current_language(ctx: &mut dyn Ctx) -> &'static str {
    if let Some(s) = ctx.state("mb_language").as_str()
        && let Some(l) = language_named(&text(s.as_bytes())) {
            return l;
        }
    ini_text(ctx, "mbstring.language").and_then(|n| language_named(&n)).unwrap_or("neutral")
}

fn mb_language(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    match opt_str(args, 0) {
        None => Ok(Value::str(current_language(ctx))),
        Some(n) => {
            let n = text(&n);
            match language_named(&n) {
                None => {
                    // The failed ini change leaves the language neutral.
                    *ctx.state("mb_language") = Value::str("neutral");
                    Err(arg_error(ctx, 1, &format!("must be a valid language, \"{n}\" given")))
                }
                Some(l) => {
                    *ctx.state("mb_language") = Value::str(l);
                    Ok(Value::TRUE)
                }
            }
        }
    }
}

fn mb_internal_encoding(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    match opt_str(args, 0) {
        None => Ok(Value::str(internal(ctx).name)),
        Some(n) => {
            let n = text(&n);
            match enc::lookup(&n) {
                None => Err(arg_error(ctx, 1, &format!("must be a valid encoding, \"{n}\" given"))),
                Some(e) => {
                    *ctx.state("mb_internal") = Value::str(e.name);
                    Ok(Value::TRUE)
                }
            }
        }
    }
}

pub fn http_input_list(ctx: &mut dyn Ctx) -> Vec<&'static Enc> {
    for ini in ["mbstring.http_input", "input_encoding", "default_charset"] {
        if let Some(l) = ini_text(ctx, ini) {
            let list: Vec<_> = l.split(',').filter_map(|n| enc::lookup(n.trim())).collect();
            if !list.is_empty() {
                return list;
            }
        }
    }
    Vec::new()
}

fn mb_http_input(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let list = http_input_list(ctx);
    let identified = |ctx: &mut dyn Ctx, key: &'static str| match ctx.state(key).cloned_str() {
        Some(s) => Value::str(s),
        None => Value::FALSE,
    };
    match opt_str(args, 0) {
        None => Ok(identified(ctx, "mb_http_identify")),
        Some(t) if t.len() == 1 => match t[0].to_ascii_uppercase() {
            b'G' => Ok(identified(ctx, "mb_http_identify_get")),
            b'P' => Ok(identified(ctx, "mb_http_identify_post")),
            b'C' => Ok(identified(ctx, "mb_http_identify_cookie")),
            b'S' => Ok(identified(ctx, "mb_http_identify_string")),
            b'I' => {
                let mut a = PhpArray::new();
                for e in list {
                    let _ = a.push(Value::str(e.name));
                }
                Ok(Value::array(a))
            }
            b'L' => {
                if list.is_empty() {
                    Ok(Value::FALSE)
                } else {
                    Ok(Value::str(list.iter().map(|e| e.name).collect::<Vec<_>>().join(",")))
                }
            }
            _ => Err(arg_error(ctx, 1, "must be one of \"G\", \"P\", \"C\", \"S\", \"I\", or \"L\"")),
        },
        Some(_) => Err(arg_error(ctx, 1, "must be one of \"G\", \"P\", \"C\", \"S\", \"I\", or \"L\"")),
    }
}

/// The output encoding: "pass" or an encoding's name.
pub fn http_output(ctx: &mut dyn Ctx) -> String {
    if let Some(s) = ctx.state("mb_http_output").as_str() {
        return text(s.as_bytes());
    }
    let configured = ini_text(ctx, "mbstring.http_output").or_else(|| ini_text(ctx, "output_encoding"));
    if let Some(e) = configured.and_then(|n| if n.eq_ignore_ascii_case("pass") { Some("pass".to_string()) } else { enc::lookup(&n).map(|e| e.name.to_string()) }) {
        return e;
    }
    "UTF-8".to_string()
}

fn mb_http_output(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    match opt_str(args, 0) {
        None => Ok(Value::str(http_output(ctx))),
        Some(n) => {
            let n = text(&n);
            let name = if n.eq_ignore_ascii_case("pass") { Some("pass") } else { enc::lookup(&n).map(|e| e.name) };
            match name {
                None => Err(arg_error(ctx, 1, &format!("must be a valid encoding, \"{n}\" given"))),
                Some(name) => {
                    *ctx.state("mb_http_output") = Value::str(name);
                    Ok(Value::TRUE)
                }
            }
        }
    }
}

/// An encoding list ("a, b" with "auto") as mbstring parses it.
pub fn parse_list(ctx: &mut dyn Ctx, value: &[u8], argn: usize) -> Result<Vec<&'static Enc>, Throw> {
    if value.is_empty() {
        return Ok(Vec::new());
    }
    let mut v = value;
    if v.len() > 2 && v[0] == b'"' && v[v.len() - 1] == b'"' {
        v = &v[1..v.len() - 1];
    }
    let mut list: Vec<&'static Enc> = Vec::new();
    let mut auto = false;
    for part in v.split(|&c| c == b',') {
        let p = text(part);
        let p = p.trim_matches(|c| c == ' ' || c == '\t');
        if !p.is_empty() && p.len() <= 4 && "auto"[..p.len()].eq_ignore_ascii_case(p) {
            if !auto {
                auto = true;
                list.extend(default_detect_order(ctx));
            }
        } else {
            match enc::lookup(p) {
                Some(e) => list.push(e),
                None => {
                    if argn == 0 {
                        ctx.warn(&format!("{}(): INI setting contains invalid encoding \"{p}\"", ctx.builtin_name()));
                    }
                    return Err(arg_error(ctx, argn.max(1), &format!("contains invalid encoding \"{p}\"")));
                }
            }
        }
    }
    Ok(list)
}

pub fn parse_array(ctx: &mut dyn Ctx, a: &PhpArray, argn: usize) -> Result<Vec<&'static Enc>, Throw> {
    let mut list: Vec<&'static Enc> = Vec::new();
    let mut auto = false;
    for v in a.values() {
        let v = v.deref();
        if v.is_array() {
            ctx.warn("Array to string conversion");
        }
        let s = if v.is_array() {
            "Array".to_string()
        } else if v.is_object() {
            let callable = Value::array(PhpArray::from_list(vec![v.clone(), Value::str("__toString")]));
            let r = ctx.call(&callable, &mut [])?;
            text(r.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes())
        } else {
            text(v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes())
        };
        if s.eq_ignore_ascii_case("auto") {
            if !auto {
                auto = true;
                list.extend(default_detect_order(ctx));
            }
        } else {
            match enc::lookup(&s) {
                Some(e) => list.push(e),
                None => return Err(arg_error(ctx, argn, &format!("contains invalid encoding \"{s}\""))),
            }
        }
    }
    Ok(list)
}

/// "auto": the detect order the language gives (ASCII, UTF-8 for ours).
fn default_detect_order(_ctx: &mut dyn Ctx) -> Vec<&'static Enc> {
    vec![enc::lookup("ASCII").unwrap(), enc::utf8()]
}

pub fn detect_order(ctx: &mut dyn Ctx) -> Vec<&'static Enc> {
    if let Some(s) = ctx.state("mb_detect").as_str() {
        let names = text(s.as_bytes());
        return names.split(',').filter_map(enc::lookup).collect();
    }
    if let Some(list) = ini_text(ctx, "mbstring.detect_order")
        && let Ok(l) = parse_list(ctx, list.as_bytes(), 0)
            && !l.is_empty() {
                return l;
            }
    default_detect_order(ctx)
}

fn encoding_list_arg(ctx: &mut dyn Ctx, v: &Value, argn: usize) -> Result<Vec<&'static Enc>, Throw> {
    let v = v.deref();
    match v.as_array() {
        Some(a) => parse_array(ctx, a, argn),
        None => {
            let s = v.to_php_string().unwrap_or_else(PhpStr::empty);
            parse_list(ctx, s.as_bytes(), argn)
        }
    }
}

fn mb_detect_order(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    match args.first().map(Value::deref) {
        Some(v) if !v.is_null() => {
            let list = encoding_list_arg(ctx, &v, 1)?;
            if list.is_empty() {
                return Err(arg_error(ctx, 1, "must specify at least one encoding"));
            }
            *ctx.state("mb_detect") = Value::str(list.iter().map(|e| e.name).collect::<Vec<_>>().join(","));
            Ok(Value::TRUE)
        }
        _ => {
            let mut a = PhpArray::new();
            for e in detect_order(ctx) {
                let _ = a.push(Value::str(e.name));
            }
            Ok(Value::array(a))
        }
    }
}

fn mb_substitute_character(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    match args.first().map(Value::deref) {
        Some(v) if v.is_str() => {
            let s = v.as_str().expect("a string");
            let t = text(s.as_bytes()).to_ascii_lowercase();
            let mode = match t.as_str() {
                "none" => 1,
                "long" => 2,
                "entity" => 3,
                _ => return Err(arg_error(ctx, 1, "must be \"none\", \"long\", \"entity\" or a valid codepoint")),
            };
            *ctx.state("mb_mode") = Value::int(mode);
            Ok(Value::TRUE)
        }
        Some(v) if !v.is_null() => {
            if let Some(f) = v.as_float()
                && f.fract() != 0.0 {
                    let shown = Value::float(f).to_php_string().map(|s| text(s.as_bytes())).unwrap_or_default();
                    ctx.deprecated(&format!("Implicit conversion from float {shown} to int loses precision"));
                }
            let cp = v.to_int();
            if !(0..0x110000).contains(&cp) || (0xD800..=0xDFFF).contains(&cp) {
                return Err(arg_error(ctx, 1, "is not a valid codepoint"));
            }
            *ctx.state("mb_mode") = Value::int(0);
            *ctx.state("mb_char") = Value::int(cp);
            Ok(Value::TRUE)
        }
        _ => {
            let s = subst(ctx);
            Ok(match s.mode {
                1 => Value::str("none"),
                2 => Value::str("long"),
                3 => Value::str("entity"),
                _ => Value::int(s.ch as i64),
            })
        }
    }
}

fn mb_preferred_mime_name(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let n = text(&str_arg(args, 0));
    match enc::lookup(&n) {
        None => Err(arg_error(ctx, 1, &format!("must be a valid encoding, \"{n}\" given"))),
        Some(e) => match e.mime {
            Some(m) if !m.is_empty() => Ok(Value::str(m)),
            _ => {
                ctx.warn(&format!("mb_preferred_mime_name(): No MIME preferred name corresponding to \"{n}\""));
                Ok(Value::FALSE)
            }
        },
    }
}

fn mb_regex_encoding(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    match opt_str(args, 0) {
        None => Ok(match ctx.state("mb_regex_enc").cloned_str() {
            Some(s) => Value::str(s),
            None => Value::str("UTF-8"),
        }),
        Some(n) => {
            let n = text(&n);
            match enc::lookup(&n) {
                None => Err(arg_error(ctx, 1, &format!("must be a valid encoding, \"{n}\" given"))),
                Some(e) => {
                    *ctx.state("mb_regex_enc") = Value::str(e.name);
                    Ok(Value::TRUE)
                }
            }
        }
    }
}

/// mb_regex_set_options(): the options string as onig keeps it (i x m s l n
/// as bits, p is m and s together, one syntax letter).
fn regex_option_string(bits: i64, syntax: u8) -> String {
    let mut o = String::new();
    if bits & 1 != 0 {
        o.push('i');
    }
    if bits & 2 != 0 {
        o.push('x');
    }
    if bits & 12 == 12 {
        o.push('p');
    } else {
        if bits & 4 != 0 {
            o.push('m');
        }
        if bits & 8 != 0 {
            o.push('s');
        }
    }
    if bits & 16 != 0 {
        o.push('l');
    }
    if bits & 32 != 0 {
        o.push('n');
    }
    o.push(syntax as char);
    o
}

fn mb_regex_set_options(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let bits = ctx.state("mb_regex_bits").as_int().unwrap_or(12);
    let syntax = match ctx.state("mb_regex_syntax").as_int() {
        Some(c) => c as u8,
        None => b'r',
    };
    match opt_str(args, 0) {
        None => Ok(Value::str(regex_option_string(bits, syntax))),
        Some(o) => {
            let mut nb = 0;
            let mut ns = b'r';
            for &c in &o {
                match c {
                    b'i' => nb |= 1,
                    b'x' => nb |= 2,
                    b'm' => nb |= 4,
                    b's' => nb |= 8,
                    b'p' => nb |= 12,
                    b'l' => nb |= 16,
                    b'n' => nb |= 32,
                    b'j' | b'u' | b'g' | b'c' | b'r' | b'z' | b'b' | b'd' => ns = c,
                    _ => return Err(ctx.throw("ValueError", format!("Option \"{}\" is not supported", c as char))),
                }
            }
            *ctx.state("mb_regex_bits") = Value::int(nb);
            *ctx.state("mb_regex_syntax") = Value::int(ns as i64);
            Ok(Value::str(regex_option_string(bits, syntax)))
        }
    }
}

// ---- lengths, substrings ----

/// Characters' byte spans for encodings found by width or length table.
fn spans(e: &Enc, s: &[u8]) -> Option<Vec<(usize, usize)>> {
    let w = e.fixed_width();
    if w > 0 {
        return Some((0..s.len()).step_by(w).map(|i| (i, (i + w).min(s.len()))).collect());
    }
    if e.is_utf8() {
        let mut v = Vec::new();
        let mut i = 0;
        while i < s.len() {
            let n = enc::mblen_utf8(s[i]);
            v.push((i, (i + n).min(s.len())));
            i += n;
        }
        return Some(v);
    }
    None
}

pub fn strlen(e: &Enc, s: &[u8]) -> usize {
    if let Some(n) = s.len().checked_div(e.fixed_width()) {
        return n;
    }
    if e.is_utf8() && std::str::from_utf8(s).is_ok() {
        // A valid string has a character for each byte that is not a continuation.
        return s.iter().filter(|&&b| b & 0xC0 != 0x80).count();
    }
    enc::decode(e, s).len()
}

/// `len` characters from character `from` (None: to the end). Characters the
/// encoding finds invalid come out as the substitute character, except for
/// the fixed-width encodings, which cut by bytes.
pub fn substr(sb: &Subst, e: &Enc, s: &[u8], from: usize, len: Option<usize>) -> Vec<u8> {
    if len == Some(0) || (from >= s.len() && e.kind != Kind::Utf7 && e.kind != Kind::Utf7Imap && e.kind != Kind::HtmlEnt && !e.is_transfer()) {
        return Vec::new();
    }
    let w = e.fixed_width();
    if w > 0 {
        let from = from.saturating_mul(w);
        if from >= s.len() {
            return Vec::new();
        }
        let l = len.map_or(usize::MAX, |l| l.saturating_mul(w)).min(s.len() - from);
        return s[from..from + l].to_vec();
    }
    if e.is_utf8()
        && let Ok(t) = std::str::from_utf8(s) {
            let mut idx = t.char_indices().map(|(i, _)| i).skip(from);
            let Some(start) = idx.next() else { return Vec::new() };
            let end = match len {
                None => t.len(),
                Some(l) => idx.nth(l.saturating_sub(1)).unwrap_or(t.len()),
            };
            return t.as_bytes()[start..end].to_vec();
        }
    let cps = enc::decode(e, s);
    if from >= cps.len() {
        return Vec::new();
    }
    let end = len.map_or(cps.len(), |l| from.saturating_add(l).min(cps.len()));
    let mut errs = 0;
    enc::encode(e, &cps[from..end], sb, &mut errs)
}

fn mb_strlen(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let e = encoding_arg(ctx, args, 1)?;
    Ok(Value::int(strlen(e, &str_arg(args, 0)) as i64))
}

fn mb_substr(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let s = str_arg(args, 0);
    let from = int_arg(args, 1, 0);
    let len = opt_int(args, 2);
    if from == i64::MIN {
        return Err(arg_error(ctx, 2, &format!("must be between {} and {}", i64::MIN + 1, i64::MAX)));
    }
    if len == Some(i64::MIN) {
        return Err(arg_error(ctx, 3, &format!("must be between {} and {}", i64::MIN + 1, i64::MAX)));
    }
    let e = encoding_arg(ctx, args, 3)?;
    let mut mblen = 0usize;
    if from < 0 || len.is_some_and(|l| l < 0) {
        mblen = strlen(e, &s);
    }
    let real_from = if from >= 0 {
        from as usize
    } else if ((-from) as usize) < mblen {
        (mblen as i64 + from) as usize
    } else {
        0
    };
    let real_len = match len {
        None => None,
        Some(l) if l >= 0 => Some(l as usize),
        Some(l) => {
            if real_from < mblen && ((-l) as usize) < mblen - real_from {
                Some((mblen - real_from) - (-l) as usize)
            } else {
                Some(0)
            }
        }
    };
    out(substr(&subst(ctx), e, &s, real_from, real_len))
}

fn opt_int(args: &[Value], i: usize) -> Option<i64> {
    match args.get(i).map(Value::deref) {
        Some(v) if !v.is_null_or_undef() => Some(v.to_int()),
        _ => None,
    }
}

fn mb_strcut(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let s = str_arg(args, 0);
    let mut from = int_arg(args, 1, 0);
    let mut len = opt_int(args, 2);
    let e = encoding_arg(ctx, args, 3)?;
    let total = s.len() as i64;
    let mut l = len.take().unwrap_or(total);
    if from < 0 {
        from = (total + from).max(0);
    }
    if l < 0 {
        l = ((total - from) + l).max(0);
    }
    if from > total || l == 0 {
        return out(Vec::new());
    }
    let w = e.fixed_width();
    if w > 0 {
        let from = from - from % w as i64;
        let l = l.min(total - from);
        let l = l - l % w as i64;
        return out(s[from as usize..(from + l) as usize].to_vec());
    }
    if e.is_utf8() {
        // Back up to the start of a character at both ends.
        let mut start = from as usize;
        while start > 0 && start < s.len() && s[start] & 0xC0 == 0x80 {
            start -= 1;
        }
        let mut end = if l >= total - start as i64 { s.len() } else { start + l as usize };
        if end < s.len() {
            while end > start && s[end] & 0xC0 == 0x80 {
                end -= 1;
            }
        }
        return out(s[start..end].to_vec());
    }
    // Other encodings: by characters, cut at byte offsets.
    let cps = enc::decode(e, &s);
    let mut bytes = 0i64;
    let mut res = Vec::new();
    let mut errs = 0;
    for cp in cps {
        let one = enc::encode(e, &[cp], &Subst::DEFAULT, &mut errs);
        let n = one.len() as i64;
        if bytes >= from && bytes + n <= from + l {
            res.extend(one);
        }
        bytes += n;
    }
    out(res)
}

fn mb_str_split(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let s = str_arg(args, 0);
    let mut n = int_arg(args, 1, 1);
    if n <= 0 {
        return Err(arg_error(ctx, 2, "must be greater than 0"));
    }
    if n as u64 > (u32::MAX / 4) as u64 {
        return Err(arg_error(ctx, 2, "is too large"));
    }
    if n as usize > s.len() {
        n = s.len() as i64;
    }
    let e = encoding_arg(ctx, args, 2)?;
    let mut a = PhpArray::new();
    if s.is_empty() {
        return Ok(Value::array(a));
    }
    let n = n as usize;
    if let Some(sp) = spans(e, &s) {
        for chunk in sp.chunks(n) {
            let _ = a.push(Value::str(PhpStr::new(s[chunk[0].0..chunk[chunk.len() - 1].1].to_vec())));
        }
    } else {
        let cps = enc::decode(e, &s);
        for chunk in cps.chunks(n) {
            let bytes = encode_out(ctx, e, chunk);
            let _ = a.push(Value::str(PhpStr::new(bytes)));
        }
    }
    Ok(Value::array(a))
}

// ---- searching ----
//
// Like php-src, searches run on UTF-8 bytes (other encodings are converted
// first, a bad character becoming an 0xFF byte) and count characters as the
// bytes that are not continuations.

enum Found {
    At(usize),
    Missing,
    BadOffset,
}

fn is_lead(b: u8) -> bool {
    b & 0xC0 != 0x80
}

fn chars_in(b: &[u8]) -> usize {
    b.iter().filter(|&&c| is_lead(c)).count()
}

/// The text as UTF-8 for searching.
fn to_utf8(e: &Enc, s: &[u8]) -> Vec<u8> {
    if e.is_utf8() {
        return s.to_vec();
    }
    let mut out = Vec::with_capacity(s.len());
    for w in enc::decode(e, s) {
        if w == BAD || w >= 0x110000 {
            out.push(0xFF);
        } else {
            enc::push_utf8(w, &mut out);
        }
    }
    out
}

/// `offset` characters into `h` as a byte position (None: out of range).
fn offset_to_pointer(h: &[u8], mut offset: i64) -> Option<usize> {
    if offset < 0 {
        let mut pos = h.len();
        while offset < 0 {
            if pos == 0 {
                return None;
            }
            pos -= 1;
            if is_lead(h[pos]) {
                offset += 1;
            }
        }
        Some(pos)
    } else {
        let mut pos = 0usize;
        while offset > 0 {
            if pos >= h.len() {
                return None;
            }
            pos += enc::mblen_utf8(h[pos]);
            offset -= 1;
        }
        Some(pos.min(h.len()))
    }
}

fn find_first(h: &[u8], n: &[u8]) -> Option<usize> {
    if n.is_empty() {
        return Some(0);
    }
    memchr::memmem::find(h, n)
}

fn find_last(h: &[u8], n: &[u8]) -> Option<usize> {
    if n.is_empty() {
        return Some(h.len());
    }
    memchr::memmem::rfind(h, n)
}

fn find(h: &[u8], n: &[u8], offset: i64, reverse: bool) -> Found {
    let Some(op) = offset_to_pointer(h, offset) else { return Found::BadOffset };
    if h.len() < n.len() {
        return Found::Missing;
    }
    let found = if !reverse {
        find_first(&h[op..], n).map(|i| op + i)
    } else if offset >= 0 {
        find_last(&h[op..], n).map(|i| op + i)
    } else {
        let end = offset_to_pointer(&h[op..], chars_in(n) as i64).map_or(h.len(), |i| op + i);
        find_last(&h[..end], n)
    };
    match found {
        Some(i) => Found::At(chars_in(&h[..i])),
        None => Found::Missing,
    }
}

/// Both strings as UTF-8, folded when searching without regard to case.
fn search_texts(e: &Enc, hay: &[u8], needle: &[u8], fold: bool) -> (Vec<u8>, Vec<u8>) {
    if !fold {
        return (to_utf8(e, hay), to_utf8(e, needle));
    }
    let tr = Turkish(e.name == "ISO-8859-9");
    let fold_one = |s: &[u8]| {
        let cps = unicode::convert_case(Mode::FoldSimple, &enc::decode(e, s), tr);
        to_utf8(enc::utf8(), &{
            let mut v = Vec::new();
            for w in cps {
                if w == BAD || w >= 0x110000 {
                    v.push(0xFF);
                } else {
                    enc::push_utf8(w, &mut v);
                }
            }
            v
        })
    };
    (fold_one(hay), fold_one(needle))
}

fn strpos_impl(ctx: &mut dyn Ctx, args: &[Value], reverse: bool, fold: bool) -> R {
    let e = encoding_arg(ctx, args, 3)?;
    let (h, n) = search_texts(e, &str_arg(args, 0), &str_arg(args, 1), fold);
    match find(&h, &n, int_arg(args, 2, 0), reverse) {
        Found::At(i) => Ok(Value::int(i as i64)),
        Found::Missing => Ok(Value::FALSE),
        Found::BadOffset => Err(arg_error(ctx, 3, "must be contained in argument #1 ($haystack)")),
    }
}

fn mb_strpos(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    strpos_impl(ctx, args, false, false)
}
fn mb_strrpos(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    strpos_impl(ctx, args, true, false)
}
fn mb_stripos(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    strpos_impl(ctx, args, false, true)
}
fn mb_strripos(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    strpos_impl(ctx, args, true, true)
}

fn strstr_impl(ctx: &mut dyn Ctx, args: &[Value], reverse: bool, fold: bool) -> R {
    let e = encoding_arg(ctx, args, 3)?;
    let hay = str_arg(args, 0);
    let (h, n) = search_texts(e, &hay, &str_arg(args, 1), fold);
    match find(&h, &n, 0, reverse) {
        Found::At(i) => {
            if bool_arg(args, 2) {
                out(substr(&subst(ctx), e, &hay, 0, Some(i)))
            } else {
                out(substr(&subst(ctx), e, &hay, i, None))
            }
        }
        _ => Ok(Value::FALSE),
    }
}

fn mb_strstr(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    strstr_impl(ctx, args, false, false)
}
fn mb_strrchr(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    strstr_impl(ctx, args, true, false)
}
fn mb_stristr(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    strstr_impl(ctx, args, false, true)
}
fn mb_strrichr(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    strstr_impl(ctx, args, true, true)
}

fn mb_substr_count(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let needle = str_arg(args, 1);
    if needle.is_empty() {
        return Err(arg_error(ctx, 2, "must not be empty"));
    }
    let e = encoding_arg(ctx, args, 2)?;
    let (h, n) = search_texts(e, &str_arg(args, 0), &needle, false);
    if n.is_empty() {
        return Err(arg_error(ctx, 2, "must not be empty"));
    }
    let mut count = 0;
    let mut i = 0;
    while h.len() >= n.len() && i <= h.len() - n.len() {
        if h[i..i + n.len()] == n[..] {
            count += 1;
            i += n.len();
        } else {
            i += 1;
        }
    }
    Ok(Value::int(count))
}

// ---- widths ----

fn char_width(c: u32) -> usize {
    if c < 0x1100 {
        return 1;
    }
    let t = crate::gendata::EAW;
    let (mut lo, mut hi) = (0, t.len());
    while lo < hi {
        let p = (lo + hi) / 2;
        if c < t[p].0 {
            hi = p;
        } else if c > t[p].1 {
            lo = p + 1;
        } else {
            return 2;
        }
    }
    1
}

fn strwidth(cps: &[u32]) -> usize {
    cps.iter().map(|&c| char_width(c)).sum()
}

fn mb_strwidth(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let e = encoding_arg(ctx, args, 1)?;
    Ok(Value::int(strwidth(&enc::decode(e, &str_arg(args, 0))) as i64))
}

fn mb_strimwidth(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let s = str_arg(args, 0);
    let mut from = int_arg(args, 1, 0);
    let mut width = int_arg(args, 2, 0);
    let marker = str_arg(args, 3);
    let e = encoding_arg(ctx, args, 4)?;
    let cps = enc::decode(e, &s);
    if from != 0 {
        let n = strlen(e, &s) as i64;
        if from < 0 {
            from += n;
        }
        if from < 0 || from > n {
            return Err(arg_error(ctx, 2, "is out of range"));
        }
    }
    if width < 0 {
        ctx.deprecated("mb_strimwidth(): passing a negative integer to argument #3 ($width) is deprecated");
        width += strwidth(&cps) as i64;
        if from > 0 {
            width -= strwidth(&cps[..(from as usize).min(cps.len())]) as i64;
        }
        if width < 0 {
            return Err(arg_error(ctx, 3, "is out of range"));
        }
    }
    let from = (from as usize).min(cps.len());
    let width = width as usize;
    let rest = &cps[from..];
    if strwidth(rest) <= width {
        return out(encode_out(ctx, e, rest));
    }
    let marker_width = strwidth(&enc::decode(e, &marker));
    if width <= marker_width {
        return out(marker);
    }
    let mut avail = width - marker_width;
    let mut take = 0;
    for &c in rest {
        let w = char_width(c);
        if avail < w {
            break;
        }
        avail -= w;
        take += 1;
    }
    let mut res = encode_out(ctx, e, &rest[..take]);
    res.extend_from_slice(&marker);
    out(res)
}

// ---- conversion ----

fn convert_value(ctx: &mut dyn Ctx, v: &Value, to: &'static Enc, from: &[&'static Enc]) -> Option<Value> {
    let v = v.deref();
    if let Some(s) = v.as_str() {
        convert_str(ctx, s.as_bytes(), to, from).map(|b| Value::str(PhpStr::new(b)))
    } else if let Some(a) = v.as_array() {
        let mut res = PhpArray::new();
        for (k, val) in a.iter() {
            let key = match &k {
                Key::Str(s) => match convert_str(ctx, s.as_bytes(), to, from) {
                    Some(b) => Key::from_string(PhpStr::new(b)),
                    None => continue,
                },
                Key::Int(i) => Key::Int(*i),
            };
            let inner = val.deref();
            let nv = if inner.is_str() || inner.is_array() {
                match convert_value(ctx, val, to, from) {
                    Some(x) => x,
                    None => continue,
                }
            } else if inner.is_null() || inner.is_bool() || inner.is_number() {
                inner
            } else {
                ctx.warn(&format!("{}(): Object is not supported", ctx.builtin_name()));
                continue;
            };
            res.insert(key, nv);
        }
        Some(Value::array(res))
    } else {
        None
    }
}

fn convert_str(ctx: &mut dyn Ctx, s: &[u8], to: &'static Enc, from: &[&'static Enc]) -> Option<Vec<u8>> {
    let src = if from.len() == 1 {
        from[0]
    } else {
        match guess(&[s], from, strict_detection(ctx), true) {
            Some(e) => e,
            None => {
                ctx.warn(&format!("{}(): Unable to detect character encoding", ctx.builtin_name()));
                return None;
            }
        }
    };
    Some(convert(ctx, s, to, src))
}

fn remove_non_encodings(list: &mut Vec<&'static Enc>) {
    list.retain(|e| !matches!(e.kind, Kind::Base64 | Kind::Uuencode | Kind::HtmlEnt | Kind::Qprint | Kind::Bit7 | Kind::Bit8));
}

fn mb_convert_encoding(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let to = encoding_arg(ctx, args, 1)?;
    let mut from = match args.get(2).map(Value::deref) {
        Some(v) if !v.is_null() => encoding_list_arg(ctx, &v, 3)?,
        _ => vec![internal(ctx)],
    };
    if from.len() > 1 {
        remove_non_encodings(&mut from);
    }
    if from.is_empty() {
        return Err(arg_error(ctx, 3, "must specify at least one encoding"));
    }
    let input = args[0].deref();
    match input.as_str() {
        Some(s) => match convert_str(ctx, s.as_bytes(), to, &from) {
            Some(b) => out(b),
            None => Ok(Value::FALSE),
        },
        None => Ok(convert_value(ctx, &input, to, &from).unwrap_or(Value::array(PhpArray::new()))),
    }
}

// ---- detection ----

fn demerits(w: u32) -> u64 {
    if w > 0xFFFF {
        40
    } else if (0x21..=0x2F).contains(&w) {
        6
    } else if (crate::gendata::RARE[(w >> 5) as usize] >> (w & 0x1F)) & 1 == 1 {
        30
    } else {
        1
    }
}

/// The candidate encoding the strings most likely are in.
pub fn guess(strings: &[&[u8]], list: &[&'static Enc], strict: bool, order_significant: bool) -> Option<&'static Enc> {
    if list.is_empty() {
        return None;
    }
    if list.len() == 1 {
        if strict && strings.iter().any(|s| enc::decode(list[0], s).contains(&BAD)) {
            return None;
        }
        return Some(list[0]);
    }
    if strings.len() == 1 && strings[0].is_empty() {
        return Some(list[0]);
    }
    struct Cand {
        enc: &'static Enc,
        demerits: u64,
        mult: f64,
    }
    let n = list.len();
    let mut cands: Vec<Cand> = list
        .iter()
        .enumerate()
        .map(|(i, &e)| Cand { enc: e, demerits: 0, mult: if order_significant { 1.0 + (0.3 * i as f64) / n as f64 } else { 1.0 } })
        .collect();
    for s in strings.iter().rev() {
        let mut i = cands.len();
        while i > 0 {
            i -= 1;
            let mut input: &[u8] = s;
            match cands[i].enc.kind {
                Kind::Utf8 if input.starts_with(&[0xEF, 0xBB, 0xBF]) => input = &input[3..],
                Kind::Utf16Be if input.starts_with(&[0xFE, 0xFF]) => input = &input[2..],
                Kind::Utf16Le if input.starts_with(&[0xFF, 0xFE]) => input = &input[2..],
                _ => {}
            }
            if !(strict || cands.len() > 1) {
                continue;
            }
            let cps = enc::decode(cands[i].enc, input);
            let mut eliminated = false;
            let mut d = 0;
            for &w in cps.iter().rev() {
                if w == BAD {
                    if strict {
                        eliminated = true;
                        break;
                    }
                    d += 1000;
                } else {
                    d += demerits(w);
                }
            }
            if eliminated {
                cands.remove(i);
            } else {
                cands[i].demerits += d;
            }
        }
        if cands.is_empty() {
            return None;
        }
    }
    let mut best = 0;
    let score = |c: &Cand| (c.demerits as f64 * c.mult) as u64;
    for i in 1..cands.len() {
        if score(&cands[i]) < score(&cands[best]) {
            best = i;
        }
    }
    Some(cands[best].enc)
}

fn mb_detect_encoding(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let s = str_arg(args, 0);
    let mut list = match args.get(1).map(Value::deref) {
        Some(v) if !v.is_null() => encoding_list_arg(ctx, &v, 2)?,
        _ => detect_order(ctx),
    };
    if list.is_empty() {
        return Err(arg_error(ctx, 2, "must specify at least one encoding"));
    }
    remove_non_encodings(&mut list);
    if list.is_empty() {
        return Ok(Value::FALSE);
    }
    let strict = if args.len() >= 3 { bool_arg(args, 2) } else { strict_detection(ctx) };
    // The order given is not significant when it is the full list in its
    // default order (mb_list_encodings()).
    let order_significant = !args
            .get(1)
            .map(Value::deref)
            .as_ref()
            .and_then(Value::as_array)
            .is_some_and(|a| a.len() == enc::list().len() && a.values().zip(enc::list()).all(|(v, e)| v.to_php_string().is_some_and(|x| x.as_bytes() == e.name.as_bytes())));
    match guess(&[&s], &list, strict, order_significant) {
        Some(e) => Ok(Value::str(e.name)),
        None => Ok(Value::FALSE),
    }
}

fn mb_list_encodings(_: &mut dyn Ctx, _: &mut [Value]) -> R {
    let mut a = PhpArray::new();
    for e in enc::list() {
        let _ = a.push(Value::str(e.name));
    }
    Ok(Value::array(a))
}

fn mb_encoding_aliases(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let e = encoding_arg(ctx, args, 0)?;
    let mut a = PhpArray::new();
    for al in e.aliases {
        let _ = a.push(Value::str(*al));
    }
    Ok(Value::array(a))
}

fn check_value(ctx: &mut dyn Ctx, v: &Value, e: &Enc, depth: usize) -> bool {
    let v = v.deref();
    if let Some(s) = v.as_str() {
        !enc::decode(e, s.as_bytes()).contains(&BAD)
    } else if let Some(a) = v.as_array() {
        if depth > 64 {
            ctx.warn("mb_check_encoding(): Cannot not handle circular references");
            return false;
        }
        for (k, val) in a.iter() {
            if let Key::Str(s) = &k
                && enc::decode(e, s.as_bytes()).contains(&BAD) {
                    return false;
                }
            let inner = val.deref();
            if inner.is_str() || inner.is_array() {
                if !check_value(ctx, val, e, depth + 1) {
                    return false;
                }
            } else if !(inner.is_null() || inner.is_bool() || inner.is_number()) {
                return false;
            }
        }
        true
    } else {
        true
    }
}

fn mb_check_encoding(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let e = encoding_arg(ctx, args, 1)?;
    match args.first().map(Value::deref) {
        Some(v) if !v.is_null() => Ok(Value::bool(check_value(ctx, &v, e, 0))),
        _ => {
            ctx.deprecated("mb_check_encoding(): Calling mb_check_encoding() without argument is deprecated");
            Ok(Value::bool(ctx.state("mb_illegal").to_int() == 0))
        }
    }
}

fn mb_scrub(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let e = encoding_arg(ctx, args, 1)?;
    out(convert(ctx, &str_arg(args, 0), e, e))
}

fn unsupported_for_codepoints(e: &Enc) -> bool {
    matches!(e.kind, Kind::Base64 | Kind::Uuencode | Kind::HtmlEnt | Kind::Qprint | Kind::Utf7 | Kind::Utf7Imap)
}

fn mb_ord(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let s = str_arg(args, 0);
    if s.is_empty() {
        return Err(arg_error(ctx, 1, "must not be empty"));
    }
    let e = encoding_arg(ctx, args, 1)?;
    if unsupported_for_codepoints(e) {
        return Err(ctx.throw("ValueError", format!("mb_ord() does not support the \"{}\" encoding", e.name)));
    }
    match enc::decode(e, &s).first() {
        Some(&c) if c != BAD => Ok(Value::int(c as i64)),
        _ => Ok(Value::FALSE),
    }
}

fn mb_chr(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let cp = int_arg(args, 0, 0);
    let e = encoding_arg(ctx, args, 1)?;
    if unsupported_for_codepoints(e) {
        return Err(ctx.throw("ValueError", format!("mb_chr() does not support the \"{}\" encoding", e.name)));
    }
    if !(0..=0x10FFFF).contains(&cp) {
        return Ok(Value::FALSE);
    }
    let cp = cp as u32;
    if e.is_utf8() {
        if (0xD800..0xE000).contains(&cp) {
            return Ok(Value::FALSE);
        }
        let mut v = Vec::new();
        enc::push_utf8(cp, &mut v);
        return out(v);
    }
    let mut errs = 0;
    let bytes = enc::encode(e, &[cp], &subst(ctx), &mut errs);
    if errs > 0 { Ok(Value::FALSE) } else { out(bytes) }
}

// ---- case ----

fn case_convert(ctx: &mut dyn Ctx, s: &[u8], mode: Mode, e: &Enc) -> Vec<u8> {
    let cps = enc::decode(e, s);
    let conv = unicode::convert_case(mode, &cps, Turkish(e.name == "ISO-8859-9"));
    encode_out(ctx, e, &conv)
}

fn mb_convert_case(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let e = encoding_arg(ctx, args, 2)?;
    let Some(mode) = Mode::from_int(int_arg(args, 1, 0)) else {
        return Err(arg_error(ctx, 2, "must be one of the MB_CASE_* constants"));
    };
    out(case_convert(ctx, &str_arg(args, 0), mode, e))
}

fn mb_strtoupper(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let e = encoding_arg(ctx, args, 1)?;
    out(case_convert(ctx, &str_arg(args, 0), Mode::Upper, e))
}

fn mb_strtolower(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let e = encoding_arg(ctx, args, 1)?;
    out(case_convert(ctx, &str_arg(args, 0), Mode::Lower, e))
}

fn ulcfirst(ctx: &mut dyn Ctx, args: &[Value], mode: Mode) -> R {
    let e = encoding_arg(ctx, args, 1)?;
    let s = str_arg(args, 0);
    let sb = subst(ctx);
    let first = substr(&sb, e, &s, 0, Some(1));
    let head = case_convert(ctx, &first, mode, e);
    if head == first {
        return out(s);
    }
    let mut res = head;
    res.extend(substr(&sb, e, &s, 1, None));
    out(res)
}

fn mb_ucfirst(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    ulcfirst(ctx, args, Mode::Title)
}

fn mb_lcfirst(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    ulcfirst(ctx, args, Mode::Lower)
}

// ---- trim, pad ----

const TRIM_DEFAULT: [u32; 27] = [
    0x20, 0x0C, 0x0A, 0x0D, 0x09, 0x0B, 0x00, 0xA0, 0x1680, 0x2000, 0x2001, 0x2002, 0x2003, 0x2004, 0x2005, 0x2006, 0x2007, 0x2008, 0x2009, 0x200A, 0x2028,
    0x2029, 0x202F, 0x205F, 0x3000, 0x85, 0x180E,
];

fn trim_impl(ctx: &mut dyn Ctx, args: &[Value], left: bool, right: bool) -> R {
    let s = str_arg(args, 0);
    let e = encoding_arg(ctx, args, 2)?;
    let what: Vec<u32> = match opt_str(args, 1) {
        Some(w) => enc::decode(e, &w),
        None => TRIM_DEFAULT.to_vec(),
    };
    if opt_str(args, 1).is_some() && what.is_empty() {
        return out(s);
    }
    let cps = enc::decode(e, &s);
    let mut l = 0;
    let mut r = cps.len();
    if left {
        while l < r && what.contains(&cps[l]) {
            l += 1;
        }
    }
    if right {
        while r > l && what.contains(&cps[r - 1]) {
            r -= 1;
        }
    }
    if l == 0 && r == cps.len() {
        return out(s);
    }
    out(substr(&subst(ctx), e, &s, l, Some(r - l)))
}

fn mb_trim(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    trim_impl(ctx, args, true, true)
}
fn mb_ltrim(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    trim_impl(ctx, args, true, false)
}
fn mb_rtrim(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    trim_impl(ctx, args, false, true)
}

fn mb_str_pad(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let input = str_arg(args, 0);
    let to = int_arg(args, 1, 0);
    let pad = if args.len() > 2 { str_arg(args, 2) } else { b" ".to_vec() };
    let kind = int_arg(args, 3, 1);
    let e = encoding_arg(ctx, args, 4)?;
    let in_len = strlen(e, &input);
    if to < 0 || to as usize <= in_len {
        return out(input);
    }
    if pad.is_empty() {
        return Err(arg_error(ctx, 3, "must not be empty"));
    }
    if !(0..=2).contains(&kind) {
        return Err(arg_error(ctx, 4, "must be STR_PAD_LEFT, STR_PAD_RIGHT, or STR_PAD_BOTH"));
    }
    let pad_len = strlen(e, &pad);
    if pad_len == 0 {
        return Err(arg_error(ctx, 3, "must not be empty"));
    }
    let total = to as usize - in_len;
    let (l, r) = match kind {
        1 => (0, total),
        0 => (total, 0),
        _ => (total / 2, total - total / 2),
    };
    let sb = subst(ctx);
    let padding = |n: usize| {
        let mut v = Vec::new();
        for _ in 0..n / pad_len {
            v.extend_from_slice(&pad);
        }
        v.extend(substr(&sb, e, &pad, 0, Some(n % pad_len)));
        v
    };
    let mut res = padding(l);
    res.extend_from_slice(&input);
    res.extend(padding(r));
    out(res)
}

// ---- numeric entities ----

fn convmap(ctx: &mut dyn Ctx, v: &Value) -> Result<Vec<u32>, Throw> {
    let Ok(a) = v.deref().into_array() else { return Ok(Vec::new()) };
    if a.len() % 4 != 0 {
        return Err(arg_error(ctx, 2, "must have a multiple of 4 elements"));
    }
    let mut m = Vec::new();
    for x in a.values() {
        let x = x.deref();
        match x.kind() {
            ValueKind::Int => m.push(x.as_int().expect("an int") as u32),
            ValueKind::Float => m.push(x.as_float().expect("a float") as i64 as u32),
            ValueKind::Bool => m.push(x.as_bool().expect("a bool") as u32),
            ValueKind::Null => m.push(0),
            ValueKind::Str => match std::str::from_utf8(x.as_str().expect("a string").as_bytes()).ok().and_then(|t| t.trim().parse::<i64>().ok()) {
                Some(i) => m.push(i as u32),
                None => return Err(arg_error(ctx, 2, "must only be composed of values of type int")),
            },
            _ => return Err(arg_error(ctx, 2, "must only be composed of values of type int")),
        }
    }
    Ok(m)
}

fn mb_encode_numericentity(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let s = str_arg(args, 0);
    let e = encoding_arg(ctx, args, 2)?;
    let map = convmap(ctx, &args[1])?;
    let hex = bool_arg(args, 3);
    let mut res: Vec<u32> = Vec::new();
    for w in enc::decode(e, &s) {
        let mapped = map.chunks(4).find(|m| w >= m[0] && w <= m[1]).map(|m| w.wrapping_add(m[2]) & m[3]);
        match mapped {
            Some(v) => {
                res.extend("&#".bytes().map(u32::from));
                if hex {
                    res.push(b'x' as u32);
                    res.extend(format!("{v:X}").bytes().map(u32::from));
                } else {
                    res.extend(v.to_string().bytes().map(u32::from));
                }
                res.push(b';' as u32);
            }
            None => res.push(w),
        }
    }
    out(encode_out(ctx, e, &res))
}

fn mb_decode_numericentity(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let s = str_arg(args, 0);
    let e = encoding_arg(ctx, args, 2)?;
    let map = convmap(ctx, &args[1])?;
    let cps = enc::decode(e, &s);
    let mut res: Vec<u32> = Vec::new();
    let mut i = 0;
    let deconvert = |n: u32| map.chunks(4).find_map(|m| {
        let cp = n.wrapping_sub(m[2]);
        (cp >= m[0] && cp <= m[1]).then_some(cp)
    });
    while i < cps.len() {
        if cps[i] != b'&' as u32 {
            res.push(cps[i]);
            i += 1;
            continue;
        }
        let p = i;
        let mut p2 = i + 1;
        if p2 < cps.len() && cps[p2] == b'#' as u32 {
            p2 += 1;
            if p2 < cps.len() && cps[p2] == b'x' as u32 {
                p2 += 1;
                while p2 < cps.len() && char::from_u32(cps[p2]).is_some_and(|c| c.is_ascii_hexdigit()) {
                    p2 += 1;
                }
                let n = p2 - p;
                if !(4..=11).contains(&n) {
                    res.extend_from_slice(&cps[p..p2]);
                } else {
                    let mut v: u32 = 0;
                    for &d in &cps[p + 3..p2] {
                        v = v.wrapping_mul(16).wrapping_add(char::from_u32(d).unwrap().to_digit(16).unwrap());
                    }
                    match deconvert(v) {
                        Some(cp) => {
                            res.push(cp);
                            if p2 < cps.len() && cps[p2] == b';' as u32 {
                                p2 += 1;
                            }
                        }
                        None => res.extend_from_slice(&cps[p..p2]),
                    }
                }
            } else {
                while p2 < cps.len() && (b'0' as u32..=b'9' as u32).contains(&cps[p2]) {
                    p2 += 1;
                }
                let n = p2 - p;
                if !(3..=12).contains(&n) {
                    res.extend_from_slice(&cps[p..p2]);
                } else {
                    let mut v: u32 = 0;
                    let mut too_big = false;
                    for &d in &cps[p + 2..p2] {
                        if v > 0x19999999 {
                            too_big = true;
                            break;
                        }
                        v = v * 10 + (d - b'0' as u32);
                    }
                    if too_big {
                        res.extend_from_slice(&cps[p..p2]);
                    } else {
                        match deconvert(v) {
                            Some(cp) => {
                                res.push(cp);
                                if p2 < cps.len() && cps[p2] == b';' as u32 {
                                    p2 += 1;
                                }
                            }
                            None => res.extend_from_slice(&cps[p..p2]),
                        }
                    }
                }
            }
        } else {
            res.push(b'&' as u32);
        }
        i = p2;
    }
    out(encode_out(ctx, e, &res))
}

// ---- mb_convert_variables ----

fn collect_strings(v: &Value, out: &mut Vec<Vec<u8>>, depth: usize) -> bool {
    let v = v.deref();
    if let Some(s) = v.as_str() {
        out.push(s.as_bytes().to_vec());
    } else if let Some(a) = v.as_array() {
        if depth > 64 {
            return true;
        }
        for (k, val) in a.iter() {
            if let Key::Str(s) = &k {
                out.push(s.as_bytes().to_vec());
            }
            if collect_strings(val, out, depth + 1) {
                return true;
            }
        }
    }
    false
}

pub fn convert_var(ctx: &mut dyn Ctx, v: &Value, from: &'static Enc, to: &'static Enc, depth: usize) -> Option<Value> {
    let v = v.deref();
    if let Some(s) = v.as_str() {
        Some(Value::str(PhpStr::new(convert(ctx, s.as_bytes(), to, from))))
    } else if let Some(a) = v.as_array() {
        if depth > 64 {
            return None;
        }
        let mut res = PhpArray::new();
        for (k, val) in a.iter() {
            let key = match &k {
                Key::Str(s) => Key::from_string(PhpStr::new(convert(ctx, s.as_bytes(), to, from))),
                Key::Int(i) => Key::Int(*i),
            };
            res.insert(key, convert_var(ctx, val, from, to, depth + 1)?);
        }
        Some(Value::array(res))
    } else {
        Some(v)
    }
}

fn mb_convert_variables(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let to = encoding_arg(ctx, &[args[0].clone()], 0)?;
    let list = encoding_list_arg(ctx, &args[1], 2)?;
    if list.is_empty() {
        return Err(arg_error(ctx, 2, "must specify at least one encoding"));
    }
    let from = if list.len() == 1 {
        list[0]
    } else {
        let mut strings = Vec::new();
        for a in &args[2..] {
            if collect_strings(a, &mut strings, 0) {
                ctx.warn("mb_convert_variables(): Cannot handle recursive references");
                return Ok(Value::FALSE);
            }
        }
        let refs: Vec<&[u8]> = strings.iter().map(|s| s.as_slice()).collect();
        let strict = strict_detection(ctx);
        match guess(&refs, &list, strict, true) {
            Some(e) => e,
            None => {
                ctx.warn("mb_convert_variables(): Unable to detect encoding");
                return Ok(Value::FALSE);
            }
        }
    };
    for arg in args.iter_mut().skip(2) {
        let cur = arg.deref();
        match convert_var(ctx, &cur, from, to, 0) {
            Some(nv) => match arg.as_reference() {
                Some(r) => *r.borrow_mut() = nv,
                None => *arg = nv,
            },
            None => {
                ctx.warn("mb_convert_variables(): Cannot handle recursive references");
                return Ok(Value::FALSE);
            }
        }
    }
    Ok(Value::str(from.name))
}

// ---- mb_get_info ----

fn mb_get_info(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let t = if args.is_empty() { "all".to_string() } else { text(&str_arg(args, 0)).to_ascii_lowercase() };
    let internal_name = internal(ctx).name;
    let language = current_language(ctx);
    let sub = subst(ctx);
    let strict = strict_detection(ctx);
    let detect: Vec<Value> = detect_order(ctx).iter().map(|e| Value::str(e.name)).collect();
    let http_in: Value = {
        match ctx.state("mb_http_identify").cloned_str() {
            Some(s) => Value::str(s),
            None => Value::NULL,
        }
    };
    let subst_value = match sub.mode {
        1 => Value::str("none"),
        2 => Value::str("long"),
        3 => Value::str("entity"),
        _ => Value::int(sub.ch as i64),
    };
    let http_out = http_output(ctx);
    let conv = ini_text(ctx, "mbstring.http_output_conv_mimetypes").unwrap_or_default();
    let translation = ini_text(ctx, "mbstring.encoding_translation").is_some_and(|v| v != "0");
    let entries: Vec<(&str, Value)> = vec![
        ("internal_encoding", Value::str(internal_name)),
        ("http_input", http_in),
        ("http_output", Value::str(http_out)),
        ("http_output_conv_mimetypes", Value::str(conv)),
        ("mail_charset", Value::str("UTF-8")),
        ("mail_header_encoding", Value::str("BASE64")),
        ("mail_body_encoding", Value::str("BASE64")),
        ("illegal_chars", Value::int(ctx.state("mb_illegal").to_int())),
        ("encoding_translation", Value::str(if translation { "On" } else { "Off" })),
        ("language", Value::str(language)),
        ("detect_order", Value::array(PhpArray::from_list(detect))),
        ("substitute_character", subst_value),
        ("strict_detection", Value::str(if strict { "On" } else { "Off" })),
    ];
    if t == "all" {
        let mut a = PhpArray::new();
        for (k, v) in entries {
            a.insert(Key::Str(PhpStr::from(k)), v);
        }
        return Ok(Value::array(a));
    }
    match entries.into_iter().find(|(k, _)| *k == t) {
        Some((_, v)) => Ok(v),
        None => {
            ctx.warn("mb_get_info(): argument #1 ($type) must be a valid type");
            Ok(Value::FALSE)
        }
    }
}
