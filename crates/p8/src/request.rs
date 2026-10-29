//! A web request's variables: $_GET, $_POST, $_COOKIE, $_FILES and
//! $_REQUEST, parsed the way PHP's SAPI layer does.
//!
//! Reference: $PHP_SRC_PATH/main/php_variables.c (php_register_variable_ex,
//! php_default_treat_data, add_post_vars, php_autoglobal_merge),
//! $PHP_SRC_PATH/main/rfc1867.c (multipart/form-data and uploads)

use p8_runtime::array::{Key, PhpArray};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

/// The ini settings that shape parsing.
pub struct Limits {
    pub post_max_size: i64,
    pub upload_max_filesize: i64,
    pub max_file_uploads: i64,
    pub max_input_vars: i64,
    pub max_input_nesting_level: i64,
    pub max_multipart_body_parts: i64,
    pub file_uploads: bool,
    pub upload_tmp_dir: Option<PathBuf>,
    pub variables_order: Vec<u8>,
    pub request_order: Option<Vec<u8>>,
}

/// What parsing a request's input gives.
#[derive(Default)]
pub struct Parsed {
    pub get: PhpArray,
    pub post: PhpArray,
    pub cookie: PhpArray,
    pub files: PhpArray,
    pub request: PhpArray,
    /// Warnings PHP raises while parsing (at request startup), complete.
    pub warnings: Vec<String>,
    /// The temporary files made for uploads.
    pub uploads: Vec<PathBuf>,
}

/// What a request is, for parsing.
pub struct Input<'a> {
    pub method: &'a str,
    pub query: Option<&'a [u8]>,
    pub cookie: Option<&'a [u8]>,
    pub content_type: Option<&'a [u8]>,
    pub body: &'a [u8],
}

const STARTUP: &str = "PHP Request Startup: ";

/// Parse everything a request carries into the superglobals' arrays.
pub fn parse(input: &Input, limits: &Limits) -> Parsed {
    let mut out = Parsed::default();
    if let Some(query) = input.query.filter(|q| !q.is_empty()) {
        treat_data(&mut out.get, query, b"&", false, limits, &mut out.warnings);
    }
    if input.method == "POST" && let Some(ct) = input.content_type {
        post(input, ct, limits, &mut out);
    }
    if let Some(cookie) = input.cookie.filter(|c| !c.is_empty()) {
        treat_data(&mut out.cookie, cookie, b";", true, limits, &mut out.warnings);
    }
    let order = limits.request_order.as_deref().unwrap_or(&limits.variables_order);
    let mut seen = [false; 3];
    for c in order {
        let (i, source) = match c.to_ascii_uppercase() {
            b'G' => (0, &out.get),
            b'P' => (1, &out.post),
            b'C' => (2, &out.cookie),
            _ => continue,
        };
        if !std::mem::replace(&mut seen[i], true) {
            autoglobal_merge(&mut out.request, source);
        }
    }
    out
}

/// php_autoglobal_merge(): arrays merge, anything else replaces.
fn autoglobal_merge(dest: &mut PhpArray, src: &PhpArray) {
    for (key, value) in src.iter() {
        if let Value::Array(s) = value
            && let Some(Value::Array(d)) = dest.get_mut(&key)
        {
            autoglobal_merge(Rc::make_mut(d), s);
            continue;
        }
        dest.insert(key, value.clone());
    }
}

/// The part of a string before its first NUL (what C sees).
fn c_str(s: &[u8]) -> &[u8] {
    &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())]
}

fn key(bytes: &[u8]) -> Key {
    Key::from_string(PhpStr::from(bytes))
}

/// php_url_decode / php_raw_url_decode.
fn url_decode(s: &[u8], raw: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        match s[i] {
            b'+' if !raw => out.push(b' '),
            b'%' if i + 2 < s.len() && s[i + 1].is_ascii_hexdigit() && s[i + 2].is_ascii_hexdigit() => {
                let h = |c: u8| (c as char).to_digit(16).unwrap_or(0) as u8;
                out.push(h(s[i + 1]) << 4 | h(s[i + 2]));
                i += 2;
            }
            c => out.push(c),
        }
        i += 1;
    }
    out
}

/// php_default_treat_data() for the query string and cookies: `name=value`
/// pairs split on `separators`.
fn treat_data(dest: &mut PhpArray, data: &[u8], separators: &[u8], cookie: bool, limits: &Limits, warnings: &mut Vec<String>) {
    let mut count = 0;
    for var in data.split(|c| separators.contains(c)).filter(|v| !v.is_empty()) {
        let mut var = c_str(var);
        if cookie {
            // Leading spaces go, needed for "a=1; b=2".
            while var.first().is_some_and(|&c| is_space(c)) {
                var = &var[1..];
            }
            if var.is_empty() || var.first() == Some(&b'=') {
                continue;
            }
        }
        let eq = var.iter().position(|&c| c == b'=');
        count += 1;
        if count > limits.max_input_vars {
            warnings.push(format!("{STARTUP}Input variables exceeded {}. To increase the limit change max_input_vars in php.ini.", limits.max_input_vars));
            break;
        }
        let (name, value) = match eq {
            Some(e) => (&var[..e], url_decode(&var[e + 1..], cookie)),
            None => (var, Vec::new()),
        };
        let name = if cookie { name.to_vec() } else { url_decode(name, false) };
        register_variable(dest, &name, Value::Str(PhpStr::new(value)), cookie, limits.max_input_nesting_level, warnings);
    }
}

/// POST: urlencoded or multipart; anything else is left for php://input.
fn post(input: &Input, content_type: &[u8], limits: &Limits, out: &mut Parsed) {
    // The type ends at the first of ";, ".
    let end = content_type.iter().position(|c| matches!(c, b';' | b',' | b' ')).unwrap_or(content_type.len());
    let kind = content_type[..end].to_ascii_lowercase();
    let too_big = limits.post_max_size > 0 && input.body.len() as i64 > limits.post_max_size;
    match kind.as_slice() {
        b"application/x-www-form-urlencoded" => {
            if too_big {
                out.warnings.push(format!(
                    "POST Content-Length of {} bytes exceeds the limit of {} bytes",
                    input.body.len(),
                    limits.post_max_size
                ));
                return;
            }
            let mut count = 0;
            for pair in input.body.split(|&c| c == b'&').filter(|p| !p.is_empty()) {
                let (name, value) = match pair.iter().position(|&c| c == b'=') {
                    Some(e) => (&pair[..e], url_decode(&pair[e + 1..], false)),
                    None => (pair, Vec::new()),
                };
                count += 1;
                if count > limits.max_input_vars {
                    out.warnings.push(format!("{STARTUP}Input variables exceeded {}. To increase the limit change max_input_vars in php.ini.", limits.max_input_vars));
                    break;
                }
                let name = url_decode(name, false);
                register_variable(&mut out.post, &name, Value::Str(PhpStr::new(value)), false, limits.max_input_nesting_level, &mut out.warnings);
            }
        }
        b"multipart/form-data" => multipart(input.body, content_type, limits, too_big, out),
        _ => {}
    }
}

/// php_register_variable_ex(): `name` may be `a[b][]`. `cookie` marks the
/// cookie array (the first of a name wins).
pub fn register_variable(dest: &mut PhpArray, name: &[u8], value: Value, cookie: bool, max_nesting: i64, warnings: &mut Vec<String>) {
    let name = c_str(name);
    let name = &name[name.iter().position(|&c| c != b' ').unwrap_or(name.len())..];
    // No spaces or dots in the variable name (up to the first bracket).
    let mut var = name.to_vec();
    let mut bracket = None;
    for (i, c) in var.iter_mut().enumerate() {
        if *c == b' ' || *c == b'.' {
            *c = b'_';
        } else if *c == b'[' {
            bracket = Some(i);
            break;
        }
    }
    let base_len = bracket.unwrap_or(var.len());
    if base_len == 0 {
        return;
    }
    // `index` is a slice of `var`: the key in the current table.
    let mut index: Option<(usize, usize)> = Some((0, base_len));
    if let Some(open) = bracket
        && nesting_exceeded(&var, open, max_nesting)
    {
        // Too many levels: the variable goes (PHP says so only when it is
        // not displaying errors, which the CLI does).
        dest.remove(&key(&var[..base_len]));
        return;
    }
    let mut table: &mut PhpArray = dest;
    let mut top = true;
    if let Some(mut ip) = bracket {
        loop {
            ip += 1;
            let index_s = ip;
            if ip < var.len() && is_space(var[ip]) {
                ip += 1;
            }
            let new_index: Option<(usize, usize)>;
            if var.get(ip) == Some(&b']') {
                new_index = None;
            } else {
                match var[ip.min(var.len())..].iter().position(|&c| c == b']') {
                    None => {
                        // Not an index: the bracket becomes an underscore and
                        // the rest is part of the name.
                        var[index_s - 1] = b'_';
                        for c in &mut var[index_s..] {
                            if matches!(*c, b' ' | b'.' | b'[') {
                                *c = b'_';
                            }
                        }
                        let start = index.map_or(0, |(s, _)| s);
                        let full = if index.is_some() { Some((start, var.len())) } else { None };
                        plain_var(table, &var, full, value, cookie && top, name, warnings);
                        return;
                    }
                    Some(rel) => {
                        let close = ip + rel;
                        new_index = Some((index_s, close));
                        ip = close;
                    }
                }
            }
            // Make (or find) the array at the current index.
            let k = index.map(|(s, e)| key(c_str(&var[s..e])));
            if let Some((s, e)) = index
                && forbidden_name(&var[s..e], name)
            {
                return;
            }
            let child = match child_array(table, k) {
                Some(c) => c,
                None => return,
            };
            table = child;
            top = false;
            index = new_index;
            ip += 1;
            if var.get(ip) == Some(&b'[') {
                continue;
            }
            break;
        }
    }
    plain_var(table, &var, index, value, cookie && top, name, warnings);
}

/// Whether `a[b][c]...` (the first `[` at `open`) nests deeper than
/// `max` levels, counting the way the parser walks it.
fn nesting_exceeded(var: &[u8], open: usize, max: i64) -> bool {
    let mut ip = open;
    let mut nest = 0;
    loop {
        nest += 1;
        if nest > max {
            return true;
        }
        ip += 1;
        if ip < var.len() && is_space(var[ip]) {
            ip += 1;
        }
        if var.get(ip) != Some(&b']') {
            match var[ip.min(var.len())..].iter().position(|&c| c == b']') {
                Some(rel) => ip += rel,
                None => return false,
            }
        }
        ip += 1;
        if var.get(ip) != Some(&b'[') {
            return false;
        }
    }
}

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

/// php_is_forbidden_variable_name(): mangling must not make `__Host-` or
/// `__Secure-` out of something that was not.
fn forbidden_name(mangled: &[u8], pre: &[u8]) -> bool {
    [&b"__Host-"[..], &b"__Secure-"[..]].iter().any(|p| mangled.starts_with(p) && !pre.starts_with(p))
}

/// The array under `k` in `table` (appended when `k` is None), made if it
/// is not one.
fn child_array(table: &mut PhpArray, k: Option<Key>) -> Option<&mut PhpArray> {
    let k = match k {
        Some(k) => k,
        None => {
            let next = table.next_key().unwrap_or(0);
            table.push(Value::Array(Rc::new(PhpArray::new()))).ok()?;
            Key::Int(next)
        }
    };
    if !matches!(table.get(&k), Some(Value::Array(_))) {
        table.insert(k.clone(), Value::Array(Rc::new(PhpArray::new())));
    }
    match table.get_mut(&k) {
        Some(Value::Array(rc)) => Some(Rc::make_mut(rc)),
        _ => None,
    }
}

fn plain_var(table: &mut PhpArray, var: &[u8], index: Option<(usize, usize)>, value: Value, cookie_top: bool, pre: &[u8], _warnings: &mut Vec<String>) {
    match index {
        None => {
            let _ = table.push(value);
        }
        Some((s, e)) => {
            let name = c_str(&var[s..e]);
            if forbidden_name(name, pre) {
                return;
            }
            let k = key(name);
            // A cookie of the same name already seen wins (rfc2965: the
            // more specific path comes first).
            if cookie_top && table.contains_key(&k) {
                return;
            }
            table.insert(k, value);
        }
    }
}

// ---- multipart/form-data -------------------------------------------------

const FILLUNIT: usize = 5 * 1024;

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// The next LF-terminated line at `*pos` (CRLF or LF removed).
fn next_line<'a>(data: &'a [u8], pos: &mut usize) -> Option<&'a [u8]> {
    let rest = &data[*pos..];
    let lf = rest.iter().position(|&c| c == b'\n')?;
    let line = if lf > 0 && rest[lf - 1] == b'\r' { &rest[..lf - 1] } else { &rest[..lf] };
    *pos += lf + 1;
    Some(line)
}

/// php_ap_getword(): up to `stop`, quotes protecting it; `line` moves past it.
fn getword(line: &mut &[u8], stop: u8) -> Vec<u8> {
    let s = *line;
    let mut pos = 0;
    while pos < s.len() && s[pos] != stop {
        let quote = s[pos];
        if quote == b'"' || quote == b'\'' {
            pos += 1;
            while pos < s.len() && s[pos] != quote {
                if s[pos] == b'\\' && s.get(pos + 1) == Some(&quote) {
                    pos += 2;
                } else {
                    pos += 1;
                }
            }
            if pos < s.len() {
                pos += 1;
            }
        } else {
            pos += 1;
        }
    }
    if pos >= s.len() {
        *line = &s[s.len()..];
        return s.to_vec();
    }
    let res = s[..pos].to_vec();
    while pos < s.len() && s[pos] == stop {
        pos += 1;
    }
    *line = &s[pos..];
    res
}

/// php_ap_getword_conf(): a word, quoted or up to whitespace.
fn getword_conf(s: &[u8]) -> Vec<u8> {
    let s = &s[s.iter().position(|&c| !is_space(c)).unwrap_or(s.len())..];
    let (text, quote) = match s.first() {
        None => return Vec::new(),
        Some(&q @ (b'"' | b'\'')) => (&s[1..], Some(q)),
        Some(_) => (&s[..s.iter().position(|&c| is_space(c)).unwrap_or(s.len())], None),
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() && Some(text[i]) != quote {
        if text[i] == b'\\' && (text.get(i + 1) == Some(&b'\\') || quote.is_some() && text.get(i + 1) == quote.as_ref()) {
            i += 1;
        }
        out.push(text[i]);
        i += 1;
    }
    out
}

/// php_ap_basename(): after the last / or \.
fn ap_basename(path: &[u8]) -> &[u8] {
    match path.iter().rposition(|&c| c == b'/' || c == b'\\') {
        Some(i) => &path[i + 1..],
        None => path,
    }
}

/// normalize_protected_variable(): the name as an array index path.
fn normalize_protected(name: &[u8]) -> Vec<u8> {
    let name = c_str(name);
    let mut v: Vec<u8> = name[name.iter().position(|&c| c != b' ').unwrap_or(name.len())..].to_vec();
    let end = v.iter().position(|&c| c == b'[').unwrap_or(v.len());
    for c in &mut v[..end] {
        if *c == b' ' || *c == b'.' {
            *c = b'_';
        }
    }
    let Some(open) = v.iter().position(|&c| c == b'[') else { return v };
    let mut out = v[..=open].to_vec();
    let mut i = open + 1;
    loop {
        while i < v.len() && matches!(v[i], b' ' | b'\r' | b'\n' | b'\t') {
            i += 1;
        }
        let close = v[i..].iter().position(|&c| c == b']').map(|p| i + p + 1).unwrap_or(v.len());
        out.extend_from_slice(&v[i..close]);
        i = close;
        if v.get(i) == Some(&b'[') {
            out.push(b'[');
            i += 1;
        } else {
            break;
        }
    }
    out
}

struct Uploads {
    protected: HashSet<Vec<u8>>,
}

impl Uploads {
    fn register(&self, dest: &mut PhpArray, name: &[u8], value: Value, override_protection: bool, limits: &Limits, warnings: &mut Vec<String>) {
        if override_protection || !self.protected.contains(&normalize_protected(name)) {
            register_variable(dest, name, value, false, limits.max_input_nesting_level, warnings);
        }
    }
}

fn text(s: &[u8]) -> Value {
    Value::Str(PhpStr::from(s))
}

/// A temporary file for an upload: `php` and six random letters or digits.
fn temp_file(dir: &std::path::Path) -> Option<(std::fs::File, PathBuf)> {
    use std::hash::{BuildHasher, Hasher};
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    const BASE32: &[u8] = b"0123456789abcdefghijklmnopqrstuv";
    // php_do_open_temporary_file(): "php", 13 characters of a random
    // number in base 32, then what mkstemp() adds.
    let random = |salt: u64| {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(u64::from(std::process::id()) ^ salt);
        h.finish()
    };
    for attempt in 0..100 {
        let mut r = random(attempt);
        let prefix: String = (0..13)
            .map(|_| {
                let c = BASE32[(r % 32) as usize] as char;
                r /= 32;
                c
            })
            .collect();
        let mut n = random(attempt + 1000);
        let suffix: String = (0..6)
            .map(|_| {
                let c = CHARS[(n % 62) as usize] as char;
                n /= 62;
                c
            })
            .collect();
        let path = dir.join(format!("php{prefix}{suffix}"));
        use std::os::unix::fs::OpenOptionsExt;
        if let Ok(f) = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path) {
            return Some((f, path));
        }
    }
    None
}

/// The system temporary directory, as sys_get_temp_dir() has it.
pub fn system_temp_dir() -> PathBuf {
    match std::env::var_os("TMPDIR") {
        Some(d) if !d.is_empty() => {
            let s = d.to_string_lossy().into_owned();
            PathBuf::from(if s.len() > 1 { s.trim_end_matches('/').to_string() } else { s })
        }
        _ => PathBuf::from("/tmp"),
    }
}

const UPLOAD_ERR_INI_SIZE: i64 = 1;
const UPLOAD_ERR_FORM_SIZE: i64 = 2;
const UPLOAD_ERR_PARTIAL: i64 = 3;
const UPLOAD_ERR_NO_FILE: i64 = 4;
const UPLOAD_ERR_NO_TMP_DIR: i64 = 6;
const UPLOAD_ERR_CANT_WRITE: i64 = 7;

/// rfc1867_post_handler(): the form fields into $_POST, the uploads into
/// $_FILES (each in a temporary file).
fn multipart(body: &[u8], content_type: &[u8], limits: &Limits, too_big: bool, out: &mut Parsed) {
    let warn_startup = |out: &mut Parsed, m: &str| out.warnings.push(format!("{STARTUP}{m}"));
    if too_big {
        warn_startup(out, &format!("POST Content-Length of {} bytes exceeds the limit of {} bytes", body.len(), limits.post_max_size));
        return;
    }
    let mut body_parts = limits.max_multipart_body_parts;
    if body_parts < 0 {
        body_parts = limits.max_input_vars + limits.max_file_uploads;
    }
    let body_parts_limit = body_parts;
    // The boundary.
    let ct = content_type;
    let lower = ct.to_ascii_lowercase();
    let Some(at) = find(ct, b"boundary").or_else(|| find(&lower, b"boundary")) else {
        warn_startup(out, "Missing boundary in multipart/form-data POST data");
        return;
    };
    let Some(eq) = ct[at..].iter().position(|&c| c == b'=') else {
        warn_startup(out, "Missing boundary in multipart/form-data POST data");
        return;
    };
    let mut boundary = &ct[at + eq + 1..];
    if boundary.first() == Some(&b'"') {
        boundary = &boundary[1..];
        match boundary.iter().position(|&c| c == b'"') {
            Some(e) => boundary = &boundary[..e],
            None => {
                warn_startup(out, "Invalid boundary in multipart/form-data POST data");
                return;
            }
        }
    } else if let Some(e) = boundary.iter().position(|&c| c == b',' || c == b';') {
        boundary = &boundary[..e];
    }
    let boundary = c_str(boundary);
    if boundary.len() > FILLUNIT - 4 {
        out.warnings.push("Boundary too large in multipart/form-data POST data".into());
        return;
    }
    let first_line = [b"--".as_slice(), boundary].concat();
    let next_boundary = [b"\n--".as_slice(), boundary].concat();
    let tmp_dir = limits.upload_tmp_dir.clone().unwrap_or_else(system_temp_dir);

    let mut uploads = Uploads { protected: HashSet::new() };
    let mut upload_cnt = limits.max_file_uploads;
    let mut max_file_size: i64 = 0;
    let mut count = 0;
    let mut anonymous = 0u32;
    let mut pos = 0;
    loop {
        if pos >= body.len() {
            break;
        }
        // find_boundary(): lines up to the boundary line.
        let mut found = false;
        while let Some(line) = next_line(body, &mut pos) {
            if line == first_line.as_slice() {
                found = true;
                break;
            }
        }
        if !found {
            break;
        }
        // The headers: Content-Disposition and Content-Type, the first of each.
        let mut headers: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        let mut current: Option<(Vec<u8>, Vec<u8>)> = None;
        while let Some(line) = next_line(body, &mut pos) {
            if line.is_empty() {
                break;
            }
            let colon = if is_space(line[0]) { None } else { line.iter().position(|&c| c == b':') };
            match colon {
                Some(c) => {
                    if let Some(done) = current.take() {
                        headers.push(done);
                    }
                    let name = &line[..c];
                    let wanted = (name.eq_ignore_ascii_case(b"Content-Disposition") || name.eq_ignore_ascii_case(b"Content-Type")) && !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name));
                    if wanted {
                        let mut v = &line[c + 1..];
                        while let [first, rest @ ..] = v
                            && is_space(*first)
                        {
                            v = rest;
                        }
                        current = Some((name.to_vec(), v.to_vec()));
                    }
                }
                None => {
                    if let Some((_, v)) = current.as_mut() {
                        v.extend_from_slice(line);
                    }
                }
            }
        }
        if let Some(done) = current.take() {
            headers.push(done);
        }
        let header = |name: &str| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name.as_bytes())).map(|(_, v)| v.clone());
        let Some(cd) = header("Content-Disposition") else { continue };
        body_parts -= 1;
        if body_parts < 0 {
            warn_startup(out, &format!("Multipart body parts limit exceeded {body_parts_limit}. To increase the limit change max_multipart_body_parts in php.ini."));
            break;
        }
        // name= and filename=
        let mut cd_rest: &[u8] = &cd[cd.iter().position(|&c| !is_space(c)).unwrap_or(cd.len())..];
        let (mut param, mut filename): (Option<Vec<u8>>, Option<Vec<u8>>) = (None, None);
        while !cd_rest.is_empty() {
            let pair = getword(&mut cd_rest, b';');
            while let [first, rest @ ..] = cd_rest
                && is_space(*first)
            {
                cd_rest = rest;
            }
            if pair.contains(&b'=') {
                let mut p: &[u8] = &pair;
                let k = getword(&mut p, b'=');
                if k.eq_ignore_ascii_case(b"name") {
                    param = Some(getword_conf(p));
                } else if k.eq_ignore_ascii_case(b"filename") {
                    filename = Some(getword_conf(p));
                }
            }
        }
        // The data of this part: up to the next boundary.
        let (data, end, next_pos) = match find(&body[pos..], &next_boundary) {
            Some(i) => {
                let mut d = &body[pos..pos + i];
                if d.last() == Some(&b'\r') {
                    d = &d[..d.len() - 1];
                }
                (d, true, pos + i)
            }
            None => (&body[pos..], false, body.len()),
        };
        pos = next_pos;
        // A normal form field.
        if filename.is_none() && let Some(param) = &param {
            count += 1;
            if count <= limits.max_input_vars {
                if !uploads.protected.contains(&normalize_protected(param)) {
                    register_variable(&mut out.post, param, text(data), false, limits.max_input_nesting_level, &mut out.warnings);
                }
            } else if count == limits.max_input_vars + 1 {
                warn_startup(out, &format!("Input variables exceeded {}. To increase the limit change max_input_vars in php.ini.", limits.max_input_vars));
            }
            if param.eq_ignore_ascii_case(b"MAX_FILE_SIZE") {
                let t = String::from_utf8_lossy(data);
                let digits: String = t.trim_start().chars().enumerate().take_while(|(i, c)| c.is_ascii_digit() || *i == 0 && (*c == '-' || *c == '+')).map(|(_, c)| c).collect();
                max_file_size = digits.parse().unwrap_or(0);
            }
            continue;
        }
        let mut skip = false;
        if !limits.file_uploads {
            skip = true;
        } else if upload_cnt <= 0 {
            skip = true;
            if upload_cnt == 0 {
                upload_cnt -= 1;
                warn_startup(out, "Maximum number of allowable file uploads has been exceeded");
            }
        }
        if param.is_none() && filename.is_none() {
            warn_startup(out, "File Upload Mime headers garbled");
            break;
        }
        let param = param.unwrap_or_else(|| {
            anonymous += 1;
            (anonymous - 1).to_string().into_bytes()
        });
        let filename = filename.unwrap_or_default();
        if !skip {
            // Brackets must be balanced and nothing may follow a ] but a [.
            let mut depth = 0i64;
            for (i, &c) in param.iter().enumerate() {
                if c == b'[' {
                    depth += 1;
                } else if c == b']' {
                    depth -= 1;
                    if param.get(i + 1).is_some_and(|&n| n != b'[') {
                        skip = true;
                        break;
                    }
                }
                if depth < 0 {
                    skip = true;
                    break;
                }
            }
            if depth != 0 {
                skip = true;
            }
        }
        if skip {
            continue;
        }
        let mut cancel = 0;
        let mut temp: Option<PathBuf> = None;
        let mut total: i64 = 0;
        if filename.is_empty() {
            cancel = UPLOAD_ERR_NO_FILE;
        } else {
            match temp_file(&tmp_dir) {
                None => {
                    upload_cnt -= 1;
                    warn_startup(out, "File upload error - unable to create a temporary file");
                    cancel = UPLOAD_ERR_NO_TMP_DIR;
                }
                Some((mut file, path)) => {
                    use std::io::Write;
                    upload_cnt -= 1;
                    if limits.upload_max_filesize > 0 && data.len() as i64 > limits.upload_max_filesize {
                        cancel = UPLOAD_ERR_INI_SIZE;
                    } else if max_file_size != 0 && data.len() as i64 > max_file_size {
                        cancel = UPLOAD_ERR_FORM_SIZE;
                    } else if file.write_all(data).is_err() {
                        cancel = UPLOAD_ERR_CANT_WRITE;
                    } else {
                        total = data.len() as i64;
                    }
                    if cancel == 0 && !end {
                        cancel = UPLOAD_ERR_PARTIAL;
                    }
                    drop(file);
                    if cancel != 0 {
                        let _ = std::fs::remove_file(&path);
                    } else {
                        out.uploads.push(path.clone());
                        temp = Some(path);
                    }
                }
            }
        }
        // $_FILES: [name], [full_path], [type], [tmp_name], [error], [size].
        let is_arr = param.contains(&b'[') && param.last() == Some(&b']');
        let (abuf, array_index): (Vec<u8>, Vec<u8>) = if is_arr {
            let open = param.iter().position(|&c| c == b'[').unwrap_or(0);
            (param[..open].to_vec(), param[open + 1..param.len() - 1].to_vec())
        } else {
            (param.clone(), Vec::new())
        };
        let field = |what: &str| -> Vec<u8> {
            let mut v = abuf.clone();
            v.extend_from_slice(format!("[{what}]").as_bytes());
            if is_arr {
                v.push(b'[');
                v.extend_from_slice(&array_index);
                v.push(b']');
            }
            v
        };
        let files = &mut out.files;
        let w = &mut out.warnings;
        uploads.register(files, &field("name"), text(ap_basename(&filename)), false, limits, w);
        uploads.register(files, &field("full_path"), text(&filename), false, limits, w);
        let ctype = if cancel != 0 {
            Vec::new()
        } else {
            match header("Content-Type") {
                Some(v) => v[..v.iter().position(|&c| c == b';').unwrap_or(v.len())].to_vec(),
                None => Vec::new(),
            }
        };
        uploads.register(files, &field("type"), text(&ctype), false, limits, w);
        uploads.protected.insert(normalize_protected(&param));
        let tmp_name = field("tmp_name");
        uploads.protected.insert(normalize_protected(&tmp_name));
        let tmp_value = temp.as_ref().map_or_else(Vec::new, |p| p.to_string_lossy().into_owned().into_bytes());
        uploads.register(files, &tmp_name, text(&tmp_value), true, limits, w);
        uploads.register(files, &field("error"), Value::Int(cancel), false, limits, w);
        uploads.register(files, &field("size"), Value::Int(if cancel != 0 { 0 } else { total }), false, limits, w);
    }
}

/// base64_decode(): None when `s` has characters outside the alphabet.
pub fn base64_decode(s: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for &c in s {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => continue,
            c if c.is_ascii_whitespace() => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}
