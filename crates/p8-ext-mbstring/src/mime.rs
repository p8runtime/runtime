//! MIME headers (RFC 2047 encoded words), mb_parse_str() and mb_output_handler().
//!
//! Reference: $PHP_SRC_PATH/ext/mbstring/mbstring.c - mb_mime_header_encode,
//! mb_mime_header_decode, PHP_FUNCTION(mb_parse_str), PHP_FUNCTION(mb_output_handler)

use crate::enc::{self, Enc, Subst};
use crate::funcs::{R, arg_error, bool_arg, convert, encoding_arg, http_output, int_arg, internal, opt_str, out, str_arg, text};
use crate::gendata::QENCODE;
use p8_runtime::ext::{Ctx, Throw};
use p8_runtime::array::PhpArray;
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::RefCell;
use std::rc::Rc;

fn needs_q(c: u8) -> bool {
    c > 0x7F || c == b'=' || QENCODE[c as usize] != 0
}

fn encoded_size(tmp: &[u8], base64: bool) -> usize {
    if base64 { tmp.len().div_ceil(3) * 4 } else { tmp.iter().map(|&c| if needs_q(c) { 3 } else { 1 }).sum() }
}

fn transfer_encode(tmp: &[u8], base64: bool, out: &mut Vec<u8>) {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    if base64 {
        for ch in tmp.chunks(3) {
            let a = ch[0] as u32;
            let b = ch.get(1).copied().unwrap_or(0) as u32;
            let c = ch.get(2).copied().unwrap_or(0) as u32;
            let bits = (a << 16) | (b << 8) | c;
            out.push(T[((bits >> 18) & 63) as usize]);
            out.push(T[((bits >> 12) & 63) as usize]);
            out.push(if ch.len() > 1 { T[((bits >> 6) & 63) as usize] } else { b'=' });
            out.push(if ch.len() > 2 { T[(bits & 63) as usize] } else { b'=' });
        }
    } else {
        for &c in tmp {
            if needs_q(c) {
                out.push(b'=');
                out.push(b"0123456789ABCDEF"[(c >> 4) as usize]);
                out.push(b"0123456789ABCDEF"[(c & 15) as usize]);
            } else {
                out.push(c);
            }
        }
    }
}

/// The header encoded as RFC 2047 words, folded the way mbstring does.
pub fn header_encode(input: &[u8], incode: &Enc, outcode: &Enc, base64: bool, linefeed: &[u8], mut indent: i64) -> Vec<u8> {
    if input.is_empty() {
        return Vec::new();
    }
    if !(0..74).contains(&indent) {
        indent = 0;
    }
    let mut lf = linefeed;
    if lf.len() > 8 {
        lf = &lf[..8];
    }
    if let Some(z) = lf.iter().position(|&c| c == 0) {
        lf = &lf[..z];
    }
    let cps = enc::decode(incode, input);
    let mime = outcode.mime.unwrap_or("");
    let mut leading = true;
    let mut plain = true;
    for &w in &cps {
        if leading {
            if w == b' ' as u32 {
                continue;
            }
            leading = false;
        }
        if !(0x21..=0x7E).contains(&w) || w == b'=' as u32 || w == b'?' as u32 || w == b'_' as u32 {
            plain = false;
            break;
        }
    }
    if plain {
        return input.to_vec();
    }
    let e = cps.len();
    let mut buf: Vec<u8> = Vec::new();
    let mut line_start = 0usize;
    let mut p = 0usize;
    let mut word_start = 0usize;
    while p < e && cps[p] == b' ' as u32 && p - word_start <= 74 {
        p += 1;
    }
    let mut mime_needed = false;
    while p < e {
        let w = cps[p];
        p += 1;
        if !(0x20..=0x7E).contains(&w) || w == b'?' as u32 || w == b'=' as u32 || w == b'_' as u32 || (w == b' ' as u32 && p - word_start > 74) {
            if buf.len() - line_start + indent as usize + mime.len() > 55 {
                buf.extend_from_slice(lf);
                buf.push(b' ');
                indent = 0;
                line_start = buf.len();
            } else if !buf.is_empty() {
                buf.push(b' ');
            }
            p = word_start;
            mime_needed = true;
            break;
        } else if w == b' ' as u32 {
            if buf.len() - line_start + (p - word_start) + indent as usize > 75 {
                buf.extend_from_slice(lf);
                buf.push(b' ');
                indent = 0;
                line_start = buf.len();
            } else if !buf.is_empty() {
                buf.push(b' ');
            }
            while word_start + 1 < p {
                buf.push((cps[word_start] & 0xFF) as u8);
                word_start += 1;
            }
            word_start += 1;
            while p < e && cps[p] == b' ' as u32 {
                p += 1;
            }
        }
    }
    if !mime_needed {
        if word_start < e && !buf.is_empty() {
            if buf.len() - line_start + (p - word_start) + indent as usize > 74 {
                buf.extend_from_slice(lf);
                buf.push(b' ');
            } else {
                buf.push(b' ');
            }
        }
        while word_start < e {
            buf.push((cps[word_start] & 0xFF) as u8);
            word_start += 1;
        }
        return buf;
    }
    // MIME encoding from `p` to the end, line by line.
    let mut errs = 0;
    let mut tmp: Vec<u8> = Vec::new();
    loop {
        buf.extend_from_slice(b"=?");
        buf.extend_from_slice(mime.as_bytes());
        buf.push(b'?');
        buf.push(if base64 { b'B' } else { b'Q' });
        buf.push(b'?');
        let mut n = 12usize;
        let space_available = (73 - indent - (buf.len() - line_start) as i64).max(0) as usize;
        loop {
            let tmppos = tmp.len();
            n = n.min(e - p);
            tmp.extend(enc::encode(outcode, &cps[p..p + n], &Subst::DEFAULT, &mut errs));
            if encoded_size(&tmp, base64) <= space_available || (n == 1 && tmppos == 0) {
                p += n;
                if p == e {
                    transfer_encode(&tmp, base64, &mut buf);
                    buf.extend_from_slice(b"?=");
                    return buf;
                }
            } else {
                tmp.truncate(tmppos);
                if n == 1 {
                    transfer_encode(&tmp, base64, &mut buf);
                    tmp.clear();
                    buf.extend_from_slice(b"?=");
                    indent = 0;
                    if p < e {
                        buf.extend_from_slice(lf);
                        buf.push(b' ');
                        line_start = buf.len();
                        break;
                    }
                    return buf;
                }
                n = (n >> 1).max(1);
            }
        }
    }
}

pub fn mb_encode_mimeheader(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let s = str_arg(args, 0);
    let mut charset = enc::utf8();
    let mut base64 = true;
    if let Some(name) = opt_str(args, 1) {
        let c = encoding_arg(ctx, args, 1)?;
        if c.mime.is_none_or(|m| m.is_empty()) || c.kind == enc::Kind::Qprint {
            return Err(arg_error(ctx, 2, &format!("\"{}\" cannot be used for MIME header encoding", text(&name))));
        }
        charset = c;
    }
    if let Some(t) = opt_str(args, 2)
        && matches!(t.first(), Some(b'Q' | b'q')) {
            base64 = false;
        }
    let lf = if args.len() > 3 { str_arg(args, 3) } else { b"\r\n".to_vec() };
    let indent = int_arg(args, 4, 0);
    let internal = internal(ctx);
    out(header_encode(&s, internal, charset, base64, &lf, indent))
}

fn qval(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}

/// One encoded word at `p`; the decoded text in `outcode` and where it ended.
fn decode_word(ctx: &mut dyn Ctx, s: &[u8], mut p: usize, outcode: &Enc, outbuf: &mut Vec<u8>) -> Option<usize> {
    let mut e = s.len();
    if e - p < 6 {
        return None;
    }
    p += 2;
    let charset = p;
    let charset_end = charset + s[charset..e].iter().position(|&c| c == b'?')?;
    let encoding = charset_end + 1;
    p = encoding + 1;
    if p >= e || s[p] != b'?' {
        return None;
    }
    p += 1;
    let incode = enc::lookup(&text(&s[charset..charset_end]))?;
    if let Some(m) = s[p..e].windows(2).position(|w| w == b"?=") {
        e = p + m;
    } else if p < e && s[e - 1] == b'?' {
        e -= 1;
    }
    let mut bytes = Vec::new();
    match s[encoding] {
        b'Q' | b'q' => {
            while p < e {
                let c = s[p];
                p += 1;
                if c == b'_' {
                    bytes.push(b' ');
                    continue;
                } else if c == b'=' && e - p >= 2 {
                    let c2 = s[p];
                    let c3 = s[p + 1];
                    p += 2;
                    if let (Some(a), Some(b)) = (qval(c2), qval(c3)) {
                        bytes.push((a << 4) | (b & 0xF));
                        continue;
                    } else if c2 == b'\r' {
                        if c3 != b'\n' {
                            p -= 1;
                        }
                        continue;
                    } else if c2 == b'\n' {
                        p -= 1;
                        continue;
                    }
                }
                bytes.push(c);
            }
        }
        b'B' | b'b' => {
            let (mut bits, mut cache) = (0u32, 0u32);
            while p < e {
                let c = s[p];
                p += 1;
                if matches!(c, b'\r' | b'\n' | b' ' | b'\t' | b'=') {
                    continue;
                }
                let d = match c {
                    b'A'..=b'Z' => c - b'A',
                    b'a'..=b'z' => c - b'a' + 26,
                    b'0'..=b'9' => c - b'0' + 52,
                    b'+' => 62,
                    b'/' => 63,
                    _ => {
                        bytes.push(b'?');
                        continue;
                    }
                } as u32;
                bits += 6;
                cache = (cache << 6) | d;
                if bits == 24 {
                    bytes.extend([(cache >> 16) as u8, (cache >> 8) as u8, cache as u8]);
                    bits = 0;
                    cache = 0;
                }
            }
            if bits == 18 {
                bytes.extend([(cache >> 10) as u8, (cache >> 2) as u8]);
            } else if bits == 12 {
                bytes.push((cache >> 4) as u8);
            }
        }
        _ => return None,
    }
    let _ = ctx;
    let cps = enc::decode(incode, &bytes);
    let mut errs = 0;
    outbuf.extend(enc::encode(outcode, &cps, &Subst::DEFAULT, &mut errs));
    Some(e + 2)
}

pub fn mb_decode_mimeheader(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let s = str_arg(args, 0);
    let outcode = internal(ctx);
    let e = s.len();
    let mut p = 0;
    let mut buf: Vec<u8> = Vec::new();
    let mut space_pending = false;
    let space = |buf: &mut Vec<u8>| {
        let mut errs = 0;
        buf.extend(enc::encode(outcode, &[b' ' as u32], &Subst::DEFAULT, &mut errs));
    };
    let is_ws = |c: u8| matches!(c, b'\n' | b'\r' | b'\t' | b' ');
    while p < e {
        let c = s[p];
        if c == b'=' && p + 1 < e && s[p + 1] == b'?' && e - p >= 6
            && let Some(ie) = s[p + 2..e].iter().position(|&x| x == b'?').map(|i| i + p + 2)
                && e - ie >= 3
                    && let Some(temp) = decode_word(ctx, &s, p, outcode, &mut buf) {
                        p = temp;
                        if p < e && is_ws(s[p]) {
                            while p < e && is_ws(s[p]) {
                                p += 1;
                            }
                            space_pending = true;
                        } else {
                            space_pending = false;
                        }
                        continue;
                    }
        if space_pending {
            space(&mut buf);
            space_pending = false;
        }
        if c != b'\n' && c != b'\r' {
            let mut end = p + 1;
            while end < e && s[end] != b'=' && s[end] != b'\n' && s[end] != b'\r' {
                end += 1;
            }
            let cps = enc::decode(enc::lookup("ASCII").unwrap(), &s[p..end]);
            let mut errs = 0;
            buf.extend(enc::encode(outcode, &cps, &Subst::DEFAULT, &mut errs));
            p = end;
        }
        if p < e && (s[p] == b'\n' || s[p] == b'\r') {
            while p < e && is_ws(s[p]) && (s[p] == b'\n' || s[p] == b'\r' || s[p] == b'\t' || s[p] == b' ') {
                p += 1;
            }
            if p < e {
                space(&mut buf);
            }
        }
    }
    out(buf)
}

/// php_url_decode: '+' is a space and %XX a byte.
fn url_decode(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        match s[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < s.len() && s.get(i + 1).and_then(|&c| (c as char).to_digit(16)).is_some() && s.get(i + 2).and_then(|&c| (c as char).to_digit(16)).is_some() => {
                out.push((((s[i + 1] as char).to_digit(16).unwrap() << 4) | (s[i + 2] as char).to_digit(16).unwrap()) as u8);
                i += 2;
            }
            c => out.push(c),
        }
        i += 1;
    }
    out
}

fn url_encode(s: &[u8], out: &mut Vec<u8>) {
    for &c in s {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else {
            out.push(b'%');
            out.push(b"0123456789ABCDEF"[(c >> 4) as usize]);
            out.push(b"0123456789ABCDEF"[(c & 15) as usize]);
        }
    }
}

pub fn mb_parse_str(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let query = str_arg(args, 0);
    let list = crate::funcs::http_input_list(ctx);
    let to = internal(ctx);
    // Split into names and values, URL-decoded, as the input handler does.
    let mut parts: Vec<Vec<u8>> = Vec::new();
    let seps: Vec<u8> = match ctx.ini_get(b"arg_separator.input") {
        Some(Value::Str(s)) if !s.is_empty() => s.as_bytes().to_vec(),
        _ => b"&".to_vec(),
    };
    for var in query.split(|c| seps.contains(c)).filter(|v| !v.is_empty()) {
        match var.iter().position(|&c| c == b'=') {
            Some(eq) => {
                parts.push(url_decode(&var[..eq]));
                parts.push(url_decode(&var[eq + 1..]));
            }
            None => {
                parts.push(url_decode(var));
                parts.push(Vec::new());
            }
        }
    }
    // Which encoding the input is in: the only candidate, else the best of them.
    let from: Option<Option<&'static enc::Enc>> = if query.is_empty() {
        None
    } else if list.is_empty() {
        Some(None)
    } else if list.len() == 1 {
        Some(Some(list[0]))
    } else {
        let refs: Vec<&[u8]> = parts.iter().map(|p| p.as_slice()).collect();
        let strict = crate::funcs::strict_detection(ctx);
        match crate::funcs::guess(&refs, &list, strict, false) {
            Some(e) => Some(Some(e)),
            None => {
                ctx.warn("mb_parse_str(): Unable to detect encoding");
                Some(None)
            }
        }
    };
    let mut rebuilt: Vec<u8> = Vec::new();
    for (i, pair) in parts.chunks(2).enumerate() {
        if i > 0 {
            rebuilt.push(seps[0]);
        }
        let conv: Vec<Vec<u8>> = pair
            .iter()
            .map(|x| match from {
                Some(Some(f)) => convert(ctx, x, to, f),
                _ => x.clone(),
            })
            .collect();
        // The input handler drops a variable name's leading spaces.
        let name = &conv[0];
        let name = &name[name.iter().take_while(|&&c| c == b' ').count()..];
        url_encode(name, &mut rebuilt);
        rebuilt.push(b'=');
        url_encode(&conv[1], &mut rebuilt);
    }
    let cell = Rc::new(RefCell::new(Value::Array(Rc::new(PhpArray::new()))));
    if !rebuilt.is_empty() {
        let mut a = [Value::Str(PhpStr::new(rebuilt)), Value::Ref(Rc::clone(&cell))];
        ctx.call(&Value::str("parse_str"), &mut a)?;
    }
    let result = cell.borrow().clone();
    if let Some(Value::Ref(r)) = args.get(1) {
        *r.borrow_mut() = result;
    }
    // The encoding the input was identified as; "pass" (no conversion) counts.
    *ctx.state("mb_http_identify") = match from {
        Some(Some(f)) => Value::str(f.name),
        Some(None) => Value::str("pass"),
        None => Value::Null,
    };
    Ok(Value::Bool(from.is_some()))
}

pub fn mb_output_handler(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let s = str_arg(args, 0);
    let status = int_arg(args, 1, 0);
    let target = http_output(ctx);
    if target == "pass" {
        return out(s);
    }
    if status & 1 != 0 {
        *ctx.state("mb_outconv") = Value::Bool(true);
    }
    if !ctx.state("mb_outconv").to_bool() {
        return out(s);
    }
    let to = enc::lookup(&target).ok_or_else(|| Throw::Exit(0))?;
    let from = internal(ctx);
    let res = convert(ctx, &s, to, from);
    if status & 8 != 0 {
        *ctx.state("mb_outconv") = Value::Bool(false);
    }
    let _ = bool_arg;
    out(res)
}
