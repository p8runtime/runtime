//! More of ext/standard's string functions: entities, tags, URLs, query
//! strings, base64, wrapping, spans, similarity, natural order, C-style
//! escapes, shell quoting, the C locale.
//!
//! Reference: $PHP_SRC_PATH/ext/standard/string.c, html.c, url.c,
//! base64.c, http.c, strnatcmp.c, levenshtein.c, soundex.c, exec.c

use crate::{arg, str_arg};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cmp::Ordering;
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "strtok", run: strtok, by_ref: &[] },
    Builtin { name: "strip_tags", run: strip_tags, by_ref: &[] },
    Builtin { name: "html_entity_decode", run: html_entity_decode, by_ref: &[] },
    Builtin { name: "htmlentities", run: htmlentities, by_ref: &[] },
    Builtin { name: "htmlspecialchars_decode", run: htmlspecialchars_decode, by_ref: &[] },
    Builtin { name: "parse_url", run: parse_url, by_ref: &[] },
    Builtin { name: "http_build_query", run: http_build_query, by_ref: &[] },
    Builtin { name: "parse_str", run: parse_str, by_ref: &[1] },
    Builtin { name: "urlencode", run: urlencode, by_ref: &[] },
    Builtin { name: "rawurlencode", run: rawurlencode, by_ref: &[] },
    Builtin { name: "urldecode", run: urldecode, by_ref: &[] },
    Builtin { name: "rawurldecode", run: rawurldecode, by_ref: &[] },
    Builtin { name: "base64_encode", run: base64_encode, by_ref: &[] },
    Builtin { name: "base64_decode", run: base64_decode, by_ref: &[] },
    Builtin { name: "chunk_split", run: chunk_split, by_ref: &[] },
    Builtin { name: "wordwrap", run: wordwrap, by_ref: &[] },
    Builtin { name: "strspn", run: strspn, by_ref: &[] },
    Builtin { name: "strcspn", run: strcspn, by_ref: &[] },
    Builtin { name: "strpbrk", run: strpbrk, by_ref: &[] },
    Builtin { name: "substr_compare", run: substr_compare, by_ref: &[] },
    Builtin { name: "similar_text", run: similar_text, by_ref: &[2] },
    Builtin { name: "levenshtein", run: levenshtein, by_ref: &[] },
    Builtin { name: "soundex", run: soundex, by_ref: &[] },
    Builtin { name: "str_word_count", run: str_word_count, by_ref: &[] },
    Builtin { name: "count_chars", run: count_chars, by_ref: &[] },
    Builtin { name: "quotemeta", run: quotemeta, by_ref: &[] },
    Builtin { name: "addcslashes", run: addcslashes, by_ref: &[] },
    Builtin { name: "stripcslashes", run: stripcslashes, by_ref: &[] },
    Builtin { name: "strnatcmp", run: strnatcmp, by_ref: &[] },
    Builtin { name: "strnatcasecmp", run: strnatcasecmp, by_ref: &[] },
    Builtin { name: "natsort", run: natsort, by_ref: &[0] },
    Builtin { name: "natcasesort", run: natcasesort, by_ref: &[0] },
    Builtin { name: "escapeshellarg", run: escapeshellarg, by_ref: &[] },
    Builtin { name: "escapeshellcmd", run: escapeshellcmd, by_ref: &[] },
    Builtin { name: "setlocale", run: setlocale, by_ref: &[] },
    Builtin { name: "localeconv", run: localeconv, by_ref: &[] },
];

fn bytes(v: Vec<u8>) -> Result<Value, Throw> {
    Ok(Value::Str(PhpStr::new(v)))
}

/// strip_tags(string $string, array|string|null $allowed_tags = null)
fn strip_tags(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let allowed: Vec<Vec<u8>> = match arg(args, 1) {
        Value::Array(a) => a.values().map(|v| v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes().to_ascii_lowercase()).collect(),
        Value::Null => Vec::new(),
        v => {
            let t = v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes().to_ascii_lowercase();
            t.split(|&c| c == b'<' || c == b'>').filter(|x| !x.is_empty()).map(<[u8]>::to_vec).collect()
        }
    };
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            out.push(b[i]);
            i += 1;
            continue;
        }
        // A comment, or a tag up to its (unquoted) '>'.
        if b[i..].starts_with(b"<!--") {
            i = match b[i + 4..].windows(3).position(|w| w == b"-->") {
                Some(p) => i + 4 + p + 3,
                None => b.len(),
            };
            continue;
        }
        let start = i;
        let mut quote = 0u8;
        let mut depth = 0;
        i += 1;
        while i < b.len() {
            let c = b[i];
            if quote != 0 {
                if c == quote {
                    quote = 0;
                }
            } else if c == b'"' || c == b'\'' {
                quote = c;
            } else if c == b'<' {
                depth += 1;
            } else if c == b'>' {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            i += 1;
        }
        let end = (i + 1).min(b.len());
        let tag = &b[start..end];
        let name: Vec<u8> = tag[1..].iter().skip_while(|&&c| c == b'/').take_while(|c| c.is_ascii_alphanumeric()).map(u8::to_ascii_lowercase).collect();
        if !name.is_empty() && allowed.contains(&name) {
            out.extend_from_slice(tag);
        }
        i = end;
    }
    bytes(out)
}

/// The HTML 4.01 named entities PHP knows (name, code point).
const ENTITIES: &[(&str, u32)] = &[
    ("nbsp", 160), ("iexcl", 161), ("cent", 162), ("pound", 163), ("curren", 164), ("yen", 165), ("brvbar", 166), ("sect", 167),
    ("uml", 168), ("copy", 169), ("ordf", 170), ("laquo", 171), ("not", 172), ("shy", 173), ("reg", 174), ("macr", 175),
    ("deg", 176), ("plusmn", 177), ("sup2", 178), ("sup3", 179), ("acute", 180), ("micro", 181), ("para", 182), ("middot", 183),
    ("cedil", 184), ("sup1", 185), ("ordm", 186), ("raquo", 187), ("frac14", 188), ("frac12", 189), ("frac34", 190), ("iquest", 191),
    ("Agrave", 192), ("Aacute", 193), ("Acirc", 194), ("Atilde", 195), ("Auml", 196), ("Aring", 197), ("AElig", 198), ("Ccedil", 199),
    ("Egrave", 200), ("Eacute", 201), ("Ecirc", 202), ("Euml", 203), ("Igrave", 204), ("Iacute", 205), ("Icirc", 206), ("Iuml", 207),
    ("ETH", 208), ("Ntilde", 209), ("Ograve", 210), ("Oacute", 211), ("Ocirc", 212), ("Otilde", 213), ("Ouml", 214), ("times", 215),
    ("Oslash", 216), ("Ugrave", 217), ("Uacute", 218), ("Ucirc", 219), ("Uuml", 220), ("Yacute", 221), ("THORN", 222), ("szlig", 223),
    ("agrave", 224), ("aacute", 225), ("acirc", 226), ("atilde", 227), ("auml", 228), ("aring", 229), ("aelig", 230), ("ccedil", 231),
    ("egrave", 232), ("eacute", 233), ("ecirc", 234), ("euml", 235), ("igrave", 236), ("iacute", 237), ("icirc", 238), ("iuml", 239),
    ("eth", 240), ("ntilde", 241), ("ograve", 242), ("oacute", 243), ("ocirc", 244), ("otilde", 245), ("ouml", 246), ("divide", 247),
    ("oslash", 248), ("ugrave", 249), ("uacute", 250), ("ucirc", 251), ("uuml", 252), ("yacute", 253), ("thorn", 254), ("yuml", 255),
    ("OElig", 338), ("oelig", 339), ("Scaron", 352), ("scaron", 353), ("Yuml", 376), ("fnof", 402), ("circ", 710), ("tilde", 732),
    ("Alpha", 913), ("Beta", 914), ("Gamma", 915), ("Delta", 916), ("Epsilon", 917), ("Zeta", 918), ("Eta", 919), ("Theta", 920),
    ("Iota", 921), ("Kappa", 922), ("Lambda", 923), ("Mu", 924), ("Nu", 925), ("Xi", 926), ("Omicron", 927), ("Pi", 928),
    ("Rho", 929), ("Sigma", 931), ("Tau", 932), ("Upsilon", 933), ("Phi", 934), ("Chi", 935), ("Psi", 936), ("Omega", 937),
    ("alpha", 945), ("beta", 946), ("gamma", 947), ("delta", 948), ("epsilon", 949), ("zeta", 950), ("eta", 951), ("theta", 952),
    ("iota", 953), ("kappa", 954), ("lambda", 955), ("mu", 956), ("nu", 957), ("xi", 958), ("omicron", 959), ("pi", 960),
    ("rho", 961), ("sigmaf", 962), ("sigma", 963), ("tau", 964), ("upsilon", 965), ("phi", 966), ("chi", 967), ("psi", 968),
    ("omega", 969), ("thetasym", 977), ("upsih", 978), ("piv", 982), ("ensp", 8194), ("emsp", 8195), ("thinsp", 8201), ("zwnj", 8204),
    ("zwj", 8205), ("lrm", 8206), ("rlm", 8207), ("ndash", 8211), ("mdash", 8212), ("lsquo", 8216), ("rsquo", 8217), ("sbquo", 8218),
    ("ldquo", 8220), ("rdquo", 8221), ("bdquo", 8222), ("dagger", 8224), ("Dagger", 8225), ("bull", 8226), ("hellip", 8230), ("permil", 8240),
    ("prime", 8242), ("Prime", 8243), ("lsaquo", 8249), ("rsaquo", 8250), ("oline", 8254), ("frasl", 8260), ("euro", 8364), ("image", 8465),
    ("weierp", 8472), ("real", 8476), ("trade", 8482), ("alefsym", 8501), ("larr", 8592), ("uarr", 8593), ("rarr", 8594), ("darr", 8595),
    ("harr", 8596), ("crarr", 8629), ("lArr", 8656), ("uArr", 8657), ("rArr", 8658), ("dArr", 8659), ("hArr", 8660), ("forall", 8704),
    ("part", 8706), ("exist", 8707), ("empty", 8709), ("nabla", 8711), ("isin", 8712), ("notin", 8713), ("ni", 8715), ("prod", 8719),
    ("sum", 8721), ("minus", 8722), ("lowast", 8727), ("radic", 8730), ("prop", 8733), ("infin", 8734), ("ang", 8736), ("and", 8743),
    ("or", 8744), ("cap", 8745), ("cup", 8746), ("int", 8747), ("there4", 8756), ("sim", 8764), ("cong", 8773), ("asymp", 8776),
    ("ne", 8800), ("equiv", 8801), ("le", 8804), ("ge", 8805), ("sub", 8834), ("sup", 8835), ("nsub", 8836), ("sube", 8838),
    ("supe", 8839), ("oplus", 8853), ("otimes", 8855), ("perp", 8869), ("sdot", 8901), ("lceil", 8968), ("rceil", 8969), ("lfloor", 8970),
    ("rfloor", 8971), ("lang", 9001), ("rang", 9002), ("loz", 9674), ("spades", 9824), ("clubs", 9827), ("hearts", 9829), ("diams", 9830),
];

const ENT_COMPAT: i64 = 2;
const ENT_QUOTES: i64 = 3;
const ENT_HTML5: i64 = 48;

fn push_char(out: &mut Vec<u8>, cp: u32) {
    if let Some(c) = char::from_u32(cp) {
        let mut buf = [0; 4];
        out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
    }
}

/// Decode the entities in `s`: named ones from `named` (when given),
/// numeric ones, and the quotes `flags` allow.
fn decode(s: &[u8], flags: i64, named: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] != b'&' {
            out.push(s[i]);
            i += 1;
            continue;
        }
        let end = s[i..].iter().take(34).position(|&c| c == b';');
        let Some(end) = end else {
            out.push(b'&');
            i += 1;
            continue;
        };
        let body = &s[i + 1..i + end];
        let quote_ok = |cp: u32| match cp {
            0x22 => flags & ENT_COMPAT != 0,
            0x27 => flags & ENT_QUOTES == ENT_QUOTES,
            _ => true,
        };
        let decoded: Option<u32> = if let Some(num) = body.strip_prefix(b"#") {
            let (digits, radix) = match num.first() {
                Some(b'x' | b'X') => (&num[1..], 16),
                _ => (num, 10),
            };
            std::str::from_utf8(digits)
                .ok()
                .filter(|d| !d.is_empty() && d.len() <= 8)
                .and_then(|d| u32::from_str_radix(d, radix).ok())
                .filter(|&cp| cp != 0 && char::from_u32(cp).is_some() && quote_ok(cp))
        } else {
            match body {
                b"amp" => Some(0x26),
                b"lt" => Some(0x3c),
                b"gt" => Some(0x3e),
                b"quot" if flags & ENT_COMPAT != 0 => Some(0x22),
                b"apos" if flags & ENT_QUOTES == ENT_QUOTES && flags & ENT_HTML5 != 0 => Some(0x27),
                name if named => ENTITIES.iter().find(|(n, _)| n.as_bytes() == name).map(|(_, cp)| *cp),
                _ => None,
            }
        };
        match decoded {
            Some(cp) => {
                push_char(&mut out, cp);
                i += end + 1;
            }
            None => {
                out.push(b'&');
                i += 1;
            }
        }
    }
    out
}

fn ent_flags(args: &[Value], i: usize) -> i64 {
    if args.len() > i { arg(args, i).to_int() } else { ENT_QUOTES | 8 }
}

fn html_entity_decode(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    bytes(decode(str_arg(args, 0).as_bytes(), ent_flags(args, 1), true))
}

fn htmlspecialchars_decode(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    bytes(decode(str_arg(args, 0).as_bytes(), ent_flags(args, 1), false))
}

/// htmlentities: htmlspecialchars, and every character with a name.
fn htmlentities(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let flags = ent_flags(args, 1);
    let double = args.len() < 4 || arg(args, 3).to_bool();
    let text = String::from_utf8_lossy(s.as_bytes());
    let mut out = Vec::with_capacity(s.len());
    let src = text.as_bytes();
    let mut i = 0;
    for c in text.chars() {
        let at = i;
        i += c.len_utf8();
        match c {
            '&' if !double && decode(&src[at..(at + 34).min(src.len())], ENT_QUOTES, true).first() != Some(&b'&') => out.push(b'&'),
            '&' => out.extend_from_slice(b"&amp;"),
            '<' => out.extend_from_slice(b"&lt;"),
            '>' => out.extend_from_slice(b"&gt;"),
            '"' if flags & ENT_COMPAT != 0 => out.extend_from_slice(b"&quot;"),
            '\'' if flags & ENT_QUOTES == ENT_QUOTES => out.extend_from_slice(b"&#039;"),
            c => match ENTITIES.iter().find(|(_, cp)| *cp == c as u32) {
                Some((name, _)) => out.extend_from_slice(format!("&{name};").as_bytes()),
                None => {
                    let mut buf = [0; 4];
                    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                }
            },
        }
    }
    bytes(out)
}

/// parse_url's parts, in PHP's order.
struct Url {
    scheme: Option<Vec<u8>>,
    host: Option<Vec<u8>>,
    port: Option<i64>,
    user: Option<Vec<u8>>,
    pass: Option<Vec<u8>>,
    path: Option<Vec<u8>>,
    query: Option<Vec<u8>>,
    fragment: Option<Vec<u8>>,
}

/// php_url_parse_ex2, for the URLs scripts give it.
fn parse(url: &[u8]) -> Option<Url> {
    let mut u = Url { scheme: None, host: None, port: None, user: None, pass: None, path: None, query: None, fragment: None };
    let mut rest = url;
    if let Some(h) = rest.iter().position(|&c| c == b'#') {
        u.fragment = Some(rest[h + 1..].to_vec());
        rest = &rest[..h];
    }
    if let Some(q) = rest.iter().position(|&c| c == b'?') {
        u.query = Some(rest[q + 1..].to_vec());
        rest = &rest[..q];
    }
    // A scheme: letters, digits, + - . up to ':' (not a port: "host:80").
    if let Some(colon) = rest.iter().position(|&c| c == b':') {
        let scheme = &rest[..colon];
        let after = &rest[colon + 1..];
        let valid = !scheme.is_empty() && scheme.iter().all(|c| c.is_ascii_alphanumeric() || b"+-.".contains(c));
        let port_like = !after.starts_with(b"//") && !after.is_empty() && after.iter().take_while(|c| c.is_ascii_digit()).count() > 0
            && after.iter().find(|c| !c.is_ascii_digit()).is_none_or(|&c| c == b'/');
        if valid && !port_like {
            u.scheme = Some(scheme.to_vec());
            rest = after;
        } else if valid && port_like && !after.starts_with(b"//") {
            // "host:80/path": a host and port.
            let digits: Vec<u8> = after.iter().take_while(|c| c.is_ascii_digit()).copied().collect();
            u.host = Some(scheme.to_vec());
            u.port = std::str::from_utf8(&digits).ok()?.parse().ok();
            let path = &after[digits.len()..];
            if !path.is_empty() {
                u.path = Some(path.to_vec());
            }
            return Some(u);
        }
    }
    if let Some(auth) = rest.strip_prefix(b"//") {
        let end = auth.iter().position(|&c| c == b'/').unwrap_or(auth.len());
        let (authority, path) = (&auth[..end], &auth[end..]);
        let mut host_part = authority;
        if let Some(at) = authority.iter().rposition(|&c| c == b'@') {
            let creds = &authority[..at];
            match creds.iter().position(|&c| c == b':') {
                Some(c) => {
                    u.user = Some(creds[..c].to_vec());
                    u.pass = Some(creds[c + 1..].to_vec());
                }
                None => u.user = Some(creds.to_vec()),
            }
            host_part = &authority[at + 1..];
        }
        let port_at = if host_part.starts_with(b"[") {
            host_part.iter().position(|&c| c == b']').and_then(|e| (host_part.get(e + 1) == Some(&b':')).then_some(e + 1))
        } else {
            host_part.iter().rposition(|&c| c == b':')
        };
        if let Some(p) = port_at {
            let digits = &host_part[p + 1..];
            if !digits.is_empty() {
                if !digits.iter().all(u8::is_ascii_digit) || digits.len() > 5 {
                    return None;
                }
                let port: i64 = std::str::from_utf8(digits).ok()?.parse().ok()?;
                if port > 65535 {
                    return None;
                }
                u.port = Some(port);
            }
            host_part = &host_part[..p];
        }
        if host_part.is_empty() && u.scheme.as_deref() != Some(b"file") {
            return None;
        }
        if !host_part.is_empty() {
            u.host = Some(host_part.to_vec());
        }
        if !path.is_empty() {
            u.path = Some(path.to_vec());
        }
        return Some(u);
    }
    if !rest.is_empty() {
        u.path = Some(rest.to_vec());
    }
    Some(u)
}

/// parse_url(string $url, int $component = -1)
fn parse_url(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let url = str_arg(args, 0);
    let component = if args.len() > 1 { arg(args, 1).to_int() } else { -1 };
    let Some(u) = parse(url.as_bytes()) else { return Ok(Value::Bool(false)) };
    let s = |v: &Option<Vec<u8>>| v.as_ref().map(|b| Value::Str(PhpStr::from(b.as_slice())));
    let parts = [
        ("scheme", s(&u.scheme)),
        ("host", s(&u.host)),
        ("port", u.port.map(Value::Int)),
        ("user", s(&u.user)),
        ("pass", s(&u.pass)),
        ("path", s(&u.path)),
        ("query", s(&u.query)),
        ("fragment", s(&u.fragment)),
    ];
    if component >= 0 {
        if component > 7 {
            return Err(ctx.throw("ValueError", "parse_url(): Argument #2 ($component) must be a valid URL component identifier, ".to_string() + &component.to_string() + " given"));
        }
        return Ok(parts[component as usize].1.clone().unwrap_or(Value::Null));
    }
    let mut out = PhpArray::new();
    for (k, v) in parts {
        if let Some(v) = v {
            out.insert(Key::from_string(PhpStr::from(k)), v);
        }
    }
    Ok(Value::Array(Rc::new(out)))
}

/// urlencode (`raw` false: space as '+') / rawurlencode (RFC 3986).
fn url_encode(s: &[u8], raw: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for &c in s {
        match c {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' => out.push(c),
            b'~' if raw => out.push(c),
            b' ' if !raw => out.push(b'+'),
            c => out.extend_from_slice(format!("%{c:02X}").as_bytes()),
        }
    }
    out
}

fn url_decode(s: &[u8], raw: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        match s[i] {
            b'+' if !raw => out.push(b' '),
            b'%' if i + 2 < s.len() && s[i + 1].is_ascii_hexdigit() && s[i + 2].is_ascii_hexdigit() => {
                let h = std::str::from_utf8(&s[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()).unwrap_or(0);
                out.push(h);
                i += 2;
            }
            c => out.push(c),
        }
        i += 1;
    }
    out
}

fn urlencode(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    bytes(url_encode(str_arg(args, 0).as_bytes(), false))
}

fn rawurlencode(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    bytes(url_encode(str_arg(args, 0).as_bytes(), true))
}

fn urldecode(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    bytes(url_decode(str_arg(args, 0).as_bytes(), false))
}

fn rawurldecode(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    bytes(url_decode(str_arg(args, 0).as_bytes(), true))
}

/// http_build_query(array|object $data, string $numeric_prefix = "",
/// ?string $arg_separator = null, int $encoding_type = PHP_QUERY_RFC1738)
fn http_build_query(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let prefix = str_arg(args, 1);
    let sep = match arg(args, 2) {
        Value::Null => b"&".to_vec(),
        v => v.to_php_string().unwrap_or_else(PhpStr::empty).as_bytes().to_vec(),
    };
    let raw = args.len() > 3 && arg(args, 3).to_int() == 2;
    let mut pairs: Vec<Vec<u8>> = Vec::new();
    fn walk(v: &Value, name: Option<Vec<u8>>, prefix: &[u8], raw: bool, pairs: &mut Vec<Vec<u8>>) {
        let entries: Vec<(Key, Value)> = match v.deref() {
            Value::Array(a) => a.iter().map(|(k, v)| (k, v.deref())).collect(),
            Value::Object(o) => {
                let mut e: Vec<(Key, Value)> = Vec::new();
                for (p, val) in o.class.properties.iter().zip(o.slots.borrow().iter()) {
                    if p.visibility == p8_runtime::object::Visibility::Public && !p.is_internal() && !matches!(val, Value::Undef) {
                        e.push((Key::from_string(p.name.clone()), val.deref()));
                    }
                }
                e.extend(o.dynamic.borrow().iter().map(|(k, v)| (k, v.deref())));
                e
            }
            _ => return,
        };
        for (k, v) in entries {
            let key: Vec<u8> = match (&k, &name) {
                (Key::Int(i), None) => [prefix, i.to_string().as_bytes()].concat(),
                (Key::Int(i), Some(_)) => i.to_string().into_bytes(),
                (Key::Str(s), _) => s.as_bytes().to_vec(),
            };
            let full = match &name {
                Some(n) => [n.as_slice(), b"%5B", &url_encode(&key, raw), b"%5D"].concat(),
                None => url_encode(&key, raw),
            };
            match v {
                Value::Array(_) | Value::Object(_) => walk(&v, Some(full), prefix, raw, pairs),
                Value::Null => {}
                Value::Bool(b) => pairs.push([full.as_slice(), b"=", if b { b"1" } else { b"0" }].concat()),
                other => {
                    let s = other.to_php_string().unwrap_or_else(PhpStr::empty);
                    pairs.push([full.as_slice(), b"=", &url_encode(s.as_bytes(), raw)].concat());
                }
            }
        }
    }
    walk(&arg(args, 0), None, prefix.as_bytes(), raw, &mut pairs);
    bytes(pairs.join(sep.as_slice()))
}

/// parse_str(string $string, array &$result)
fn parse_str(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let mut out = PhpArray::new();
    for pair in s.as_bytes().split(|&c| c == b'&').filter(|p| !p.is_empty()) {
        let (name, value) = match pair.iter().position(|&c| c == b'=') {
            Some(e) => (url_decode(&pair[..e], false), url_decode(&pair[e + 1..], false)),
            None => (url_decode(pair, false), Vec::new()),
        };
        // name[a][b][] → nested; spaces and dots in the base become _.
        let base_end = name.iter().position(|&c| c == b'[').unwrap_or(name.len());
        let base: Vec<u8> = name[..base_end].iter().map(|&c| if c == b' ' || c == b'.' { b'_' } else { c }).collect();
        if base.is_empty() {
            continue;
        }
        let mut path: Vec<Option<Vec<u8>>> = Vec::new();
        let mut rest = &name[base_end..];
        while let Some(r) = rest.strip_prefix(b"[") {
            let Some(close) = r.iter().position(|&c| c == b']') else { break };
            let k = &r[..close];
            path.push(if k.is_empty() { None } else { Some(k.to_vec()) });
            rest = &r[close + 1..];
        }
        fn set(into: &mut PhpArray, key: Option<Vec<u8>>, path: &[Option<Vec<u8>>], value: Value) {
            let Some((next, more)) = path.split_first() else {
                match key {
                    Some(k) => into.insert(Key::from_string(PhpStr::new(k)), value),
                    None => {
                        let _ = into.push(value);
                    }
                }
                return;
            };
            let k = match key {
                Some(k) => Key::from_string(PhpStr::new(k)),
                None => {
                    let _ = into.push(Value::Array(Rc::new(PhpArray::new())));
                    Key::Int(into.len() as i64 - 1)
                }
            };
            let mut child = match into.get(&k) {
                Some(Value::Array(a)) => (**a).clone(),
                _ => PhpArray::new(),
            };
            set(&mut child, next.clone(), more, value);
            into.insert(k, Value::Array(Rc::new(child)));
        }
        set(&mut out, Some(base), &path, Value::Str(PhpStr::new(value)));
    }
    if let Some(Value::Ref(cell)) = args.get(1) {
        *cell.borrow_mut() = Value::Array(Rc::new(out));
    }
    Ok(Value::Null)
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let mut out = Vec::with_capacity(s.len().div_ceil(3) * 4);
    for chunk in s.as_bytes().chunks(3) {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(B64[(n >> 18) as usize & 63]);
        out.push(B64[(n >> 12) as usize & 63]);
        out.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] } else { b'=' });
        out.push(if chunk.len() > 2 { B64[n as usize & 63] } else { b'=' });
    }
    bytes(out)
}

/// base64_decode(string $string, bool $strict = false)
fn base64_decode(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let strict = args.len() > 1 && arg(args, 1).to_bool();
    let mut out = Vec::new();
    let (mut acc, mut bits, mut pad) = (0u32, 0, 0);
    for &c in s.as_bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                pad += 1;
                continue;
            }
            _ if strict && !c.is_ascii_whitespace() => return Ok(Value::Bool(false)),
            _ => continue,
        };
        if pad > 0 && strict {
            return Ok(Value::Bool(false));
        }
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    if strict && (bits >= 6 || pad > 2) {
        return Ok(Value::Bool(false));
    }
    bytes(out)
}

/// chunk_split(string $string, int $length = 76, string $separator = "\r\n")
fn chunk_split(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let len = if args.len() > 1 { arg(args, 1).to_int() } else { 76 };
    if len < 1 {
        return Err(ctx.throw("ValueError", "chunk_split(): Argument #2 ($length) must be greater than 0".into()));
    }
    let sep = if args.len() > 2 { str_arg(args, 2) } else { PhpStr::from("\r\n") };
    let mut out = Vec::new();
    for c in s.as_bytes().chunks(len as usize) {
        out.extend_from_slice(c);
        out.extend_from_slice(sep.as_bytes());
    }
    bytes(out)
}

/// wordwrap(string $string, int $width = 75, string $break = "\n", bool $cut_long_words = false)
fn wordwrap(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let width = if args.len() > 1 { arg(args, 1).to_int() } else { 75 };
    let brk = if args.len() > 2 { str_arg(args, 2) } else { PhpStr::from("\n") };
    let cut = args.len() > 3 && arg(args, 3).to_bool();
    if brk.is_empty() {
        return Err(ctx.throw("ValueError", "wordwrap(): Argument #3 ($break) cannot be empty".into()));
    }
    if width == 0 && cut {
        return Err(ctx.throw("ValueError", "wordwrap(): Argument #4 ($cut_long_words) cannot be true when argument #2 ($width) is 0".into()));
    }
    let (text, br) = (s.as_bytes(), brk.as_bytes());
    let width = width.max(0) as usize;
    // php's general algorithm.
    let mut out = Vec::with_capacity(text.len());
    let (mut last_start, mut last_space) = (0usize, 0usize);
    let mut current = 0;
    while current < text.len() {
        let c = text[current];
        if c == br[0] && current + br.len() < text.len() && &text[current..current + br.len()] == br {
            out.extend_from_slice(&text[last_start..current + br.len()]);
            current += br.len() - 1;
            last_start = current + 1;
            last_space = last_start;
        } else if c == b' ' {
            if current - last_start >= width {
                out.extend_from_slice(&text[last_start..current]);
                out.extend_from_slice(br);
                last_start = current + 1;
            }
            last_space = current;
        } else if current - last_start >= width && cut && last_start >= last_space {
            out.extend_from_slice(&text[last_start..current]);
            out.extend_from_slice(br);
            last_start = current;
            last_space = current;
        } else if current - last_start >= width && last_start < last_space {
            out.extend_from_slice(&text[last_start..last_space]);
            out.extend_from_slice(br);
            last_start = last_space + 1;
            last_space = last_start;
        }
        current += 1;
    }
    if last_start < text.len() {
        out.extend_from_slice(&text[last_start..]);
    }
    bytes(out)
}

/// The part of the subject strspn and friends look at.
fn span_subject(args: &[Value]) -> Vec<u8> {
    let s = str_arg(args, 0);
    let b = s.as_bytes();
    let n = b.len() as i64;
    let offset = if args.len() > 2 { arg(args, 2).to_int() } else { 0 };
    let start = if offset < 0 { (n + offset).max(0) } else { offset.min(n) } as usize;
    let end = match args.get(3).map(Value::deref) {
        None | Some(Value::Null) => b.len(),
        Some(l) => {
            let l = l.to_int();
            if l < 0 { ((n + l).max(start as i64)) as usize } else { (start + l as usize).min(b.len()) }
        }
    };
    b[start..end.max(start)].to_vec()
}

fn strspn(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mask = str_arg(args, 1);
    Ok(Value::Int(span_subject(args).iter().take_while(|c| mask.as_bytes().contains(c)).count() as i64))
}

fn strcspn(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mask = str_arg(args, 1);
    Ok(Value::Int(span_subject(args).iter().take_while(|c| !mask.as_bytes().contains(c)).count() as i64))
}

fn strpbrk(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let chars = str_arg(args, 1);
    if chars.is_empty() {
        return Err(ctx.throw("ValueError", "strpbrk(): Argument #2 ($characters) must be a non-empty string".into()));
    }
    Ok(match s.as_bytes().iter().position(|c| chars.as_bytes().contains(c)) {
        Some(i) => Value::Str(PhpStr::from(&s.as_bytes()[i..])),
        None => Value::Bool(false),
    })
}

/// substr_compare(string $haystack, string $needle, int $offset, ?int $length = null, bool $case_insensitive = false)
fn substr_compare(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let h = str_arg(args, 0);
    let n = str_arg(args, 1);
    let len = h.len() as i64;
    let mut offset = arg(args, 2).to_int();
    if offset < 0 {
        offset = (len + offset).max(0);
    }
    if offset > len {
        return Err(ctx.throw("ValueError", "substr_compare(): Argument #3 ($offset) must be contained in argument #1 ($main_str)".into()));
    }
    let length = match args.get(3).map(Value::deref) {
        None | Some(Value::Null) => None,
        Some(v) => {
            let l = v.to_int();
            if l < 0 {
                return Err(ctx.throw("ValueError", "substr_compare(): Argument #4 ($length) must be greater than or equal to 0".into()));
            }
            Some(l as usize)
        }
    };
    let ci = args.len() > 4 && arg(args, 4).to_bool();
    let a = &h.as_bytes()[offset as usize..];
    let (a, b) = match length {
        Some(l) => (&a[..l.min(a.len())], &n.as_bytes()[..l.min(n.len())]),
        None => (a, n.as_bytes()),
    };
    let (a, b) = if ci { (a.to_ascii_lowercase(), b.to_ascii_lowercase()) } else { (a.to_vec(), b.to_vec()) };
    Ok(Value::Int(compare_bytes(&a, &b)))
}

/// strcmp's result: the first differing bytes' difference, or the length difference.
fn compare_bytes(a: &[u8], b: &[u8]) -> i64 {
    for (x, y) in a.iter().zip(b) {
        if x != y {
            return i64::from(*x) - i64::from(*y);
        }
    }
    a.len() as i64 - b.len() as i64
}

/// php_similar_char
fn similar(a: &[u8], b: &[u8]) -> usize {
    let (mut best, mut pa, mut pb) = (0, 0, 0);
    for i in 0..a.len() {
        for j in 0..b.len() {
            let mut k = 0;
            while i + k < a.len() && j + k < b.len() && a[i + k] == b[j + k] {
                k += 1;
            }
            if k > best {
                best = k;
                pa = i;
                pb = j;
            }
        }
    }
    if best == 0 {
        return 0;
    }
    best + similar(&a[..pa], &b[..pb]) + similar(&a[pa + best..], &b[pb + best..])
}

fn similar_text(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (a, b) = (str_arg(args, 0), str_arg(args, 1));
    let sim = similar(a.as_bytes(), b.as_bytes());
    if let Some(Value::Ref(cell)) = args.get(2) {
        let total = a.len() + b.len();
        *cell.borrow_mut() = Value::Float(if total == 0 { 0.0 } else { sim as f64 * 2.0 * 100.0 / total as f64 });
    }
    Ok(Value::Int(sim as i64))
}

/// levenshtein(string $string1, string $string2, int $insertion_cost = 1, int $replacement_cost = 1, int $deletion_cost = 1)
fn levenshtein(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (a, b) = (str_arg(args, 0), str_arg(args, 1));
    let cost = |i: usize| if args.len() > i { arg(args, i).to_int() } else { 1 };
    let (ins, rep, del) = (cost(2), cost(3), cost(4));
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.is_empty() {
        return Ok(Value::Int(b.len() as i64 * ins));
    }
    if b.is_empty() {
        return Ok(Value::Int(a.len() as i64 * del));
    }
    let mut p1: Vec<i64> = (0..=b.len() as i64).map(|i| i * ins).collect();
    let mut p2 = vec![0i64; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        p2[0] = p1[0] + del;
        for (j, &cb) in b.iter().enumerate() {
            let c0 = p1[j] + if ca == cb { 0 } else { rep };
            let c1 = p1[j + 1] + del;
            let c2 = p2[j] + ins;
            p2[j + 1] = c0.min(c1).min(c2);
        }
        let _ = i;
        std::mem::swap(&mut p1, &mut p2);
    }
    Ok(Value::Int(p1[b.len()]))
}

fn soundex(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    const CODES: &[u8; 26] = b"01230120022455012623010202";
    let mut out = Vec::with_capacity(4);
    let mut last = 0u8;
    for &c in s.as_bytes() {
        if !c.is_ascii_alphabetic() {
            continue;
        }
        let up = c.to_ascii_uppercase();
        let code = CODES[(up - b'A') as usize];
        if out.is_empty() {
            out.push(up);
            last = code;
        } else {
            if code != last {
                if code != b'0' {
                    out.push(code);
                }
                last = code;
            }
            if out.len() == 4 {
                break;
            }
        }
    }
    if out.is_empty() {
        return Ok(Value::Str(PhpStr::empty()));
    }
    out.resize(4, b'0');
    bytes(out)
}

/// str_word_count(string $string, int $format = 0, ?string $characters = null)
fn str_word_count(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let format = if args.len() > 1 { arg(args, 1).to_int() } else { 0 };
    let extra = str_arg(args, 2);
    let word = |c: u8| c.is_ascii_alphabetic() || c == b'\'' || c == b'-' || extra.as_bytes().contains(&c);
    let b = s.as_bytes();
    let mut words: Vec<(usize, &[u8])> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        // A word cannot start with ' or - (unless listed), nor end with -.
        if word(b[i]) && !((b[i] == b'-' || b[i] == b'\'') && !extra.as_bytes().contains(&b[i])) {
            let start = i;
            while i < b.len() && word(b[i]) {
                i += 1;
            }
            let mut end = i;
            if end > start && b[end - 1] == b'-' && !extra.as_bytes().contains(&b'-') {
                end -= 1;
            }
            words.push((start, &b[start..end]));
        } else {
            i += 1;
        }
    }
    match format {
        0 => Ok(Value::Int(words.len() as i64)),
        1 => Ok(Value::Array(Rc::new(PhpArray::from_list(words.into_iter().map(|(_, w)| Value::Str(PhpStr::from(w))).collect())))),
        2 => {
            let mut out = PhpArray::new();
            for (at, w) in words {
                out.insert(Key::Int(at as i64), Value::Str(PhpStr::from(w)));
            }
            Ok(Value::Array(Rc::new(out)))
        }
        _ => Err(ctx.throw("ValueError", "str_word_count(): Argument #2 ($format) must be a valid format value".into())),
    }
}

/// count_chars(string $string, int $mode = 0)
fn count_chars(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let mode = if args.len() > 1 { arg(args, 1).to_int() } else { 0 };
    let mut counts = [0i64; 256];
    for &c in s.as_bytes() {
        counts[c as usize] += 1;
    }
    let mut out = PhpArray::new();
    match mode {
        0..=2 => {
            for (c, &n) in counts.iter().enumerate() {
                if mode == 0 || (mode == 1 && n > 0) || (mode == 2 && n == 0) {
                    out.insert(Key::Int(c as i64), Value::Int(n));
                }
            }
            Ok(Value::Array(Rc::new(out)))
        }
        3 | 4 => {
            let picked: Vec<u8> = (0..=255u8).filter(|&c| (counts[c as usize] > 0) == (mode == 3)).collect();
            bytes(picked)
        }
        _ => Err(ctx.throw("ValueError", "count_chars(): Argument #2 ($mode) must be between 0 and 4 (inclusive)".into())),
    }
}

fn quotemeta(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mut out = Vec::new();
    for &c in str_arg(args, 0).as_bytes() {
        if b".\\+*?[^]$()".contains(&c) {
            out.push(b'\\');
        }
        out.push(c);
    }
    bytes(out)
}

/// addcslashes(string $string, string $characters): `a..z` ranges allowed.
fn addcslashes(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let list = str_arg(args, 1);
    let l = list.as_bytes();
    let mut mask = [false; 256];
    let mut i = 0;
    while i < l.len() {
        if i + 3 < l.len() && l[i + 1] == b'.' && l[i + 2] == b'.' {
            if l[i + 3] >= l[i] {
                for c in l[i]..=l[i + 3] {
                    mask[c as usize] = true;
                }
                i += 4;
                continue;
            }
            // A range going down: PHP warns and takes the characters as they are.
            ctx.warn("addcslashes(): Invalid '..'-range, '..'-range needs to be incrementing");
            mask[l[i] as usize] = true;
            i += 1;
        } else {
            mask[l[i] as usize] = true;
            i += 1;
        }
    }
    let mut out = Vec::new();
    for &c in str_arg(args, 0).as_bytes() {
        if !mask[c as usize] {
            out.push(c);
            continue;
        }
        out.push(b'\\');
        match c {
            b'\n' => out.push(b'n'),
            b'\t' => out.push(b't'),
            b'\r' => out.push(b'r'),
            7 => out.push(b'a'),
            11 => out.push(b'v'),
            8 => out.push(b'b'),
            12 => out.push(b'f'),
            c if !(32..127).contains(&c) => out.extend_from_slice(format!("{c:03o}").as_bytes()),
            c => out.push(c),
        }
    }
    bytes(out)
}

fn stripcslashes(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' || i + 1 >= b.len() {
            out.push(b[i]);
            i += 1;
            continue;
        }
        i += 1;
        match b[i] {
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'r' => out.push(b'\r'),
            b'a' => out.push(7),
            b'v' => out.push(11),
            b'b' => out.push(8),
            b'f' => out.push(12),
            b'x' if i + 1 < b.len() && b[i + 1].is_ascii_hexdigit() => {
                let end = (i + 1..(i + 3).min(b.len())).take_while(|&j| b[j].is_ascii_hexdigit()).last().unwrap_or(i);
                out.push(u8::from_str_radix(std::str::from_utf8(&b[i + 1..=end]).unwrap_or("0"), 16).unwrap_or(0));
                i = end;
            }
            b'0'..=b'7' => {
                let end = (i..(i + 3).min(b.len())).take_while(|&j| (b'0'..=b'7').contains(&b[j])).last().unwrap_or(i);
                out.push(u32::from_str_radix(std::str::from_utf8(&b[i..=end]).unwrap_or("0"), 8).unwrap_or(0) as u8);
                i = end;
            }
            c => out.push(c),
        }
        i += 1;
    }
    bytes(out)
}

/// strnatcmp_ex: digit runs compare as numbers (leading zeros as fractions).
pub(crate) fn natural(a: &[u8], b: &[u8], fold: bool) -> Ordering {
    let (mut i, mut j) = (0, 0);
    loop {
        while i < a.len() && a[i].is_ascii_whitespace() {
            i += 1;
        }
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        let (ca, cb) = (a.get(i).copied(), b.get(j).copied());
        match (ca, cb) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let fractional = x == b'0' || y == b'0';
                let (si, sj) = (i, j);
                while i < a.len() && a[i].is_ascii_digit() {
                    i += 1;
                }
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                let (da, db) = (&a[si..i], &b[sj..j]);
                let o = if fractional {
                    da.cmp(db)
                } else {
                    let (ta, tb) = (trim_zeros(da), trim_zeros(db));
                    ta.len().cmp(&tb.len()).then(ta.cmp(tb))
                };
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                let (x, y) = if fold { (x.to_ascii_uppercase(), y.to_ascii_uppercase()) } else { (x, y) };
                if x != y {
                    return x.cmp(&y);
                }
                i += 1;
                j += 1;
            }
        }
    }
}

fn trim_zeros(d: &[u8]) -> &[u8] {
    let n = d.iter().take_while(|&&c| c == b'0').count();
    &d[n..]
}

fn ordering(o: Ordering) -> Value {
    Value::Int(match o {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    })
}

fn strnatcmp(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(ordering(natural(str_arg(args, 0).as_bytes(), str_arg(args, 1).as_bytes(), false)))
}

fn strnatcasecmp(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(ordering(natural(str_arg(args, 0).as_bytes(), str_arg(args, 1).as_bytes(), true)))
}

fn nat_sort(args: &[Value], fold: bool) -> Result<Value, Throw> {
    let Some(Value::Ref(cell)) = args.first() else { return Ok(Value::Bool(true)) };
    let mut value = cell.borrow_mut();
    if let Value::Array(a) = &*value {
        let mut entries: Vec<(Key, Value)> = a.iter().map(|(k, v)| (k, v.clone())).collect();
        let text = |v: &Value| v.to_php_string().unwrap_or_else(PhpStr::empty);
        entries.sort_by(|x, y| natural(text(&x.1).as_bytes(), text(&y.1).as_bytes(), fold));
        let mut out = PhpArray::new();
        for (k, v) in entries {
            out.insert(k, v);
        }
        *value = Value::Array(Rc::new(out));
    }
    Ok(Value::Bool(true))
}

fn natsort(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    nat_sort(args, false)
}

fn natcasesort(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    nat_sort(args, true)
}

fn escapeshellarg(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mut out = vec![b'\''];
    for &c in str_arg(args, 0).as_bytes() {
        if c == b'\'' {
            out.extend_from_slice(b"'\\''");
        } else {
            out.push(c);
        }
    }
    out.push(b'\'');
    bytes(out)
}

fn escapeshellcmd(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = str_arg(args, 0);
    let b = s.as_bytes();
    let mut out = Vec::new();
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'#' | b'&' | b';' | b'`' | b'|' | b'*' | b'?' | b'~' | b'<' | b'>' | b'^' | b'(' | b')' | b'[' | b']' | b'{' | b'}'
            | b'$' | b'\\' | b',' | b'\x0A' | b'\xFF' => {
                out.push(b'\\');
                out.push(c);
            }
            // A quote with no pair is escaped.
            b'"' | b'\'' if b[i + 1..].iter().filter(|&&x| x == c).count() % 2 == 0 && b[..i].iter().filter(|&&x| x == c).count() % 2 == 0 => {
                out.push(b'\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    bytes(out)
}

/// setlocale: p8 runs in the C locale; "C", "POSIX", "" and "0" give it.
fn setlocale(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mut names: Vec<PhpStr> = Vec::new();
    for v in &args[1.min(args.len())..] {
        match v.deref() {
            Value::Array(a) => names.extend(a.values().map(|x| x.to_php_string().unwrap_or_else(PhpStr::empty))),
            Value::Null => names.push(PhpStr::empty()),
            other => names.push(other.to_php_string().unwrap_or_else(PhpStr::empty)),
        }
    }
    for n in names {
        if matches!(n.as_bytes(), b"C" | b"POSIX" | b"" | b"0") {
            return Ok(Value::str("C"));
        }
    }
    Ok(Value::Bool(false))
}

fn localeconv(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    let mut out = PhpArray::new();
    let mut put = |k: &str, v: Value| out.insert(Key::from_string(PhpStr::from(k)), v);
    put("decimal_point", Value::str("."));
    put("thousands_sep", Value::str(""));
    put("int_curr_symbol", Value::str(""));
    put("currency_symbol", Value::str(""));
    put("mon_decimal_point", Value::str(""));
    put("mon_thousands_sep", Value::str(""));
    put("positive_sign", Value::str(""));
    put("negative_sign", Value::str(""));
    for k in ["int_frac_digits", "frac_digits", "p_cs_precedes", "p_sep_by_space", "n_cs_precedes", "n_sep_by_space", "p_sign_posn", "n_sign_posn"] {
        put(k, Value::Int(127));
    }
    put("grouping", Value::Array(Rc::new(PhpArray::new())));
    put("mon_grouping", Value::Array(Rc::new(PhpArray::new())));
    Ok(Value::Array(Rc::new(out)))
}

/// strtok($string, $token) starts on a string; strtok($token) goes on with
/// it: the next run of bytes not in `token` (leading ones skipped), or
/// false at its end (and the string is forgotten).
fn strtok(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let token = if args.len() > 1 && !matches!(arg(args, 1), Value::Null) {
        let s = str_arg(args, 0);
        *ctx.state("strtok") = Value::Array(Rc::new(PhpArray::from_list(vec![Value::Str(s), Value::Int(0)])));
        str_arg(args, 1)
    } else {
        str_arg(args, 0)
    };
    let (s, mut p) = match &*ctx.state("strtok") {
        Value::Array(a) => match (a.get(&Key::Int(0)), a.get(&Key::Int(1))) {
            (Some(Value::Str(s)), Some(Value::Int(p))) => (s.clone(), *p as usize),
            _ => return Ok(Value::Bool(false)),
        },
        _ => {
            ctx.warn("strtok(): Both arguments must be provided when starting tokenization");
            return Ok(Value::Bool(false));
        }
    };
    let b = s.as_bytes();
    let delim = |c: u8| token.as_bytes().contains(&c);
    if p >= b.len() {
        return Ok(Value::Bool(false));
    }
    while p < b.len() && delim(b[p]) {
        p += 1;
    }
    if p >= b.len() {
        *ctx.state("strtok") = Value::Null;
        return Ok(Value::Bool(false));
    }
    let start = p;
    while p < b.len() && !delim(b[p]) {
        p += 1;
    }
    let out = PhpStr::from(&b[start..p]);
    *ctx.state("strtok") = Value::Array(Rc::new(PhpArray::from_list(vec![Value::Str(s.clone()), Value::Int(p as i64 + 1)])));
    Ok(Value::Str(out))
}
