//! libmagic's text side: character-encoding detection (encoding.c), the
//! JSON, CSV and tar checks (is_json.c, is_csv.c, is_tar.c).

const T: u8 = 1;
const I: u8 = 2;
const X: u8 = 3;

/// text_chars of encoding.c: 0 never in text, 1 ASCII text, 2 ISO-8859, 3 extended.
static TEXT_CHARS: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = if i >= 0xa0 {
            I
        } else if i >= 0x80 {
            if i == 0x85 { T } else { X }
        } else if i >= 0x20 {
            if i == 0x7f { 0 } else { T }
        } else {
            match i {
                7..=13 | 27 => T,
                _ => 0,
            }
        };
        i += 1;
    }
    t
};

pub struct Encoding {
    /// "ASCII", "Unicode text, UTF-8", ...
    pub code: &'static str,
    /// "us-ascii", "utf-8", ..., "binary"
    pub mime: &'static str,
    /// "text" or "binary"
    pub kind: &'static str,
    pub looks_text: bool,
    /// the text as code points
    pub ubuf: Vec<u32>,
}

fn looks_with(buf: &[u8], ok: impl Fn(u8) -> bool, ubuf: &mut Vec<u32>) -> bool {
    ubuf.clear();
    for &c in buf {
        if !ok(TEXT_CHARS[c as usize]) {
            return false;
        }
        ubuf.push(u32::from(c));
    }
    true
}

static FIRST_ACCEPT: [(u8, u8); 5] = [(0x80, 0xbf), (0xa0, 0xbf), (0x80, 0x9f), (0x90, 0xbf), (0x80, 0x8f)];

/// first[] of encoding.c: (size info | accept-range index << 4), or special.
fn utf8_first(b: u8) -> u8 {
    const XX: u8 = 0xF1;
    const AS: u8 = 0xF0;
    match b {
        0x00..=0x7f => AS,
        0x80..=0xc1 => XX,
        0xc2..=0xdf => 0x02,
        0xe0 => 0x13,
        0xe1..=0xec | 0xee | 0xef => 0x03,
        0xed => 0x23,
        0xf0 => 0x34,
        0xf1..=0xf3 => 0x04,
        0xf4 => 0x44,
        _ => XX,
    }
}

fn looks_utf8_into(buf: &[u8], mut ubuf: Option<&mut Vec<u32>>) -> i32 {
    if let Some(u) = ubuf.as_deref_mut() {
        u.clear();
    }
    let mut gotone = false;
    let mut ctrl = false;
    let mut i = 0;
    while i < buf.len() {
        let c = buf[i];
        if c & 0x80 == 0 {
            if TEXT_CHARS[c as usize] != T {
                ctrl = true;
            }
            if let Some(u) = ubuf.as_deref_mut() {
                u.push(u32::from(c));
            }
        } else if c & 0x40 == 0 {
            return -1;
        } else {
            let x = utf8_first(c);
            if x == 0xF1 {
                return -1;
            }
            let (lo, hi) = FIRST_ACCEPT[usize::from(x >> 4)];
            let (mut cp, following) = if c & 0x20 == 0 {
                (u32::from(c & 0x1f), 1)
            } else if c & 0x10 == 0 {
                (u32::from(c & 0x0f), 2)
            } else if c & 0x08 == 0 {
                (u32::from(c & 0x07), 3)
            } else if c & 0x04 == 0 {
                (u32::from(c & 0x03), 4)
            } else if c & 0x02 == 0 {
                (u32::from(c & 0x01), 5)
            } else {
                return -1;
            };
            let mut n = 0;
            let mut truncated = false;
            while n < following {
                i += 1;
                if i >= buf.len() {
                    truncated = true;
                    break;
                }
                let d = buf[i];
                if n == 0 && (d < lo || d > hi) {
                    return -1;
                }
                if d & 0x80 == 0 || d & 0x40 != 0 {
                    return -1;
                }
                cp = (cp << 6) + u32::from(d & 0x3f);
                n += 1;
            }
            if truncated {
                break;
            }
            if let Some(u) = ubuf.as_deref_mut() {
                u.push(cp);
            }
            gotone = true;
        }
        i += 1;
    }
    if ctrl {
        0
    } else if gotone {
        2
    } else {
        1
    }
}

/// file_looks_utf8: -1 invalid, 0 control characters, 1 7-bit, 2 UTF-8.
pub fn looks_utf8(buf: &[u8]) -> i32 {
    looks_utf8_into(buf, None)
}

fn looks_ucs16(bf: &[u8], ubf: &mut Vec<u32>) -> i32 {
    if bf.len() < 2 {
        return 0;
    }
    let bigend = if bf[0] == 0xff && bf[1] == 0xfe {
        false
    } else if bf[0] == 0xfe && bf[1] == 0xff {
        true
    } else {
        return 0;
    };
    ubf.clear();
    let mut hi: u32 = 0;
    let mut i = 2;
    while i + 1 < bf.len() {
        let mut uc = if bigend { u32::from(bf[i + 1]) | (u32::from(bf[i]) << 8) } else { u32::from(bf[i]) | (u32::from(bf[i + 1]) << 8) };
        uc &= 0xffff;
        if uc == 0xfffe || uc == 0xffff || (0xfdd0..=0xfdef).contains(&uc) {
            return 0;
        }
        if hi != 0 {
            if !(0xdc00..=0xdfff).contains(&uc) {
                return 0;
            }
            uc = 0x10000 + 0x400 * (hi - 1) + (uc - 0xdc00);
            hi = 0;
        }
        if uc < 128 && TEXT_CHARS[uc as usize] != T {
            return 0;
        }
        ubf.push(uc);
        if (0xd800..=0xdbff).contains(&uc) {
            hi = uc - 0xd800 + 1;
        }
        if (0xdc00..=0xdfff).contains(&uc) {
            return 0;
        }
        i += 2;
    }
    1 + i32::from(bigend)
}

fn looks_ucs32(bf: &[u8], ubf: &mut Vec<u32>) -> i32 {
    if bf.len() < 4 {
        return 0;
    }
    let bigend = if bf[..4] == [0xff, 0xfe, 0, 0] {
        false
    } else if bf[..4] == [0, 0, 0xfe, 0xff] {
        true
    } else {
        return 0;
    };
    ubf.clear();
    let mut i = 4;
    while i + 3 < bf.len() {
        let v = if bigend {
            u32::from(bf[i + 3]) | (u32::from(bf[i + 2]) << 8) | (u32::from(bf[i + 1]) << 16) | (u32::from(bf[i]) << 24)
        } else {
            u32::from(bf[i]) | (u32::from(bf[i + 1]) << 8) | (u32::from(bf[i + 2]) << 16) | (u32::from(bf[i + 3]) << 24)
        };
        ubf.push(v);
        if v == 0xfffe {
            return 0;
        }
        if v < 128 && TEXT_CHARS[v as usize] != T {
            return 0;
        }
        i += 4;
    }
    1 + i32::from(bigend)
}

static EBCDIC: [u8; 256] = [
    0, 1, 2, 3, 156, 9, 134, 127, 151, 141, 142, 11, 12, 13, 14, 15, 16, 17, 18, 19, 157, 133, 8, 135, 24, 25, 146, 143, 28, 29, 30, 31, 128, 129, 130, 131, 132, 10, 23, 27, 136, 137, 138, 139, 140, 5, 6, 7, 144, 145,
    22, 147, 148, 149, 150, 4, 152, 153, 154, 155, 20, 21, 158, 26, b' ', 160, 161, 162, 163, 164, 165, 166, 167, 168, 213, b'.', b'<', b'(', b'+', b'|', b'&', 169, 170, 171, 172, 173, 174, 175, 176, 177, b'!', b'$',
    b'*', b')', b';', b'~', b'-', b'/', 178, 179, 180, 181, 182, 183, 184, 185, 203, b',', b'%', b'_', b'>', b'?', 186, 187, 188, 189, 190, 191, 192, 193, 194, b'`', b':', b'#', b'@', b'\'', b'=', b'"', 195, b'a',
    b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', 196, 197, 198, 199, 200, 201, 202, b'j', b'k', b'l', b'm', b'n', b'o', b'p', b'q', b'r', b'^', 204, 205, 206, 207, 208, 209, 229, b's', b't', b'u', b'v', b'w', b'x',
    b'y', b'z', 210, 211, 212, b'[', 214, 215, 216, 217, 218, 219, 220, 221, 222, 223, 224, 225, 226, 227, 228, b']', 230, 231, b'{', b'A', b'B', b'C', b'D', b'E', b'F', b'G', b'H', b'I', 232, 233, 234, 235, 236, 237,
    b'}', b'J', b'K', b'L', b'M', b'N', b'O', b'P', b'Q', b'R', 238, 239, 240, 241, 242, 243, b'\\', 159, b'S', b'T', b'U', b'V', b'W', b'X', b'Y', b'Z', 244, 245, 246, 247, 248, 249, b'0', b'1', b'2', b'3', b'4', b'5',
    b'6', b'7', b'8', b'9', 250, 251, 252, 253, 254, 255,
];

/// libmagic's file_encoding (encoding_max is applied by the caller).
pub fn file_encoding(buf: &[u8]) -> Encoding {
    let mut ubuf = Vec::new();
    let mk = |code, mime, kind, looks_text, ubuf| Encoding { code, mime, kind, looks_text, ubuf };
    if looks_with(buf, |t| t == T, &mut ubuf) {
        if buf.len() > 4 && buf[0] == b'+' && buf[1] == b'/' && buf[2] == b'v' && matches!(buf[3], b'8' | b'9' | b'+' | b'/') {
            ubuf.clear();
            return mk("Unicode text, UTF-7", "utf-7", "text", true, ubuf);
        }
        return mk("ASCII", "us-ascii", "text", true, ubuf);
    }
    if buf.len() > 3 && buf[..3] == [0xef, 0xbb, 0xbf] && looks_utf8_into(&buf[3..], Some(&mut ubuf)) > 0 {
        return mk("Unicode text, UTF-8 (with BOM)", "utf-8", "text", true, ubuf);
    }
    if looks_utf8_into(buf, Some(&mut ubuf)) > 1 {
        return mk("Unicode text, UTF-8", "utf-8", "text", true, ubuf);
    }
    let t = looks_ucs32(buf, &mut ubuf);
    if t != 0 {
        return if t == 1 { mk("Unicode text, UTF-32, little-endian", "utf-32le", "text", true, ubuf) } else { mk("Unicode text, UTF-32, big-endian", "utf-32be", "text", true, ubuf) };
    }
    let t = looks_ucs16(buf, &mut ubuf);
    if t != 0 {
        return if t == 1 { mk("Unicode text, UTF-16, little-endian", "utf-16le", "text", true, ubuf) } else { mk("Unicode text, UTF-16, big-endian", "utf-16be", "text", true, ubuf) };
    }
    if looks_with(buf, |t| t == T || t == I, &mut ubuf) {
        return mk("ISO-8859", "iso-8859-1", "text", true, ubuf);
    }
    if looks_with(buf, |t| t == T || t == I || t == X, &mut ubuf) {
        return mk("Non-ISO extended-ASCII", "unknown-8bit", "text", true, ubuf);
    }
    let nbuf: Vec<u8> = buf.iter().map(|&c| EBCDIC[c as usize]).collect();
    if looks_with(&nbuf, |t| t == T, &mut ubuf) {
        return mk("EBCDIC", "ebcdic", "text", true, ubuf);
    }
    if looks_with(&nbuf, |t| t == T || t == I, &mut ubuf) {
        return mk("International EBCDIC", "ebcdic", "text", true, ubuf);
    }
    mk("unknown", "binary", "binary", false, Vec::new())
}

/// encode_utf8 of ascmagic.c; None for an invalid character.
pub fn encode_utf8(u: &[u32]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(u.len());
    for &c in u {
        if c <= 0x7f {
            out.push(c as u8);
        } else if c <= 0x7ff {
            out.push(((c >> 6) + 0xc0) as u8);
            out.push(((c & 0x3f) + 0x80) as u8);
        } else if c <= 0xffff {
            out.push(((c >> 12) + 0xe0) as u8);
            out.push((((c >> 6) & 0x3f) + 0x80) as u8);
            out.push(((c & 0x3f) + 0x80) as u8);
        } else if c <= 0x1f_ffff {
            out.push(((c >> 18) + 0xf0) as u8);
            out.push((((c >> 12) & 0x3f) + 0x80) as u8);
            out.push((((c >> 6) & 0x3f) + 0x80) as u8);
            out.push(((c & 0x3f) + 0x80) as u8);
        } else if c <= 0x3ff_ffff {
            out.push(((c >> 24) + 0xf8) as u8);
            out.push((((c >> 18) & 0x3f) + 0x80) as u8);
            out.push((((c >> 12) & 0x3f) + 0x80) as u8);
            out.push((((c >> 6) & 0x3f) + 0x80) as u8);
            out.push(((c & 0x3f) + 0x80) as u8);
        } else if c <= 0x7fff_ffff {
            out.push(((c >> 30) + 0xfc) as u8);
            out.push((((c >> 24) & 0x3f) + 0x80) as u8);
            out.push((((c >> 18) & 0x3f) + 0x80) as u8);
            out.push((((c >> 12) & 0x3f) + 0x80) as u8);
            out.push((((c >> 6) & 0x3f) + 0x80) as u8);
            out.push(((c & 0x3f) + 0x80) as u8);
        } else {
            return None;
        }
    }
    Some(out)
}

// ---- JSON (is_json.c) ----

const J_ARRAYN: usize = 5;
const J_CONSTANT: usize = 1;
const J_NUMBER: usize = 2;
const J_OBJECT: usize = 3;
const J_STRING: usize = 4;
const J_ARRAY: usize = 0;

fn jspace(c: u8) -> bool {
    matches!(c, b' ' | b'\n' | b'\r' | b'\t')
}

fn jskip(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && jspace(b[i]) {
        i += 1;
    }
    i
}

fn jstring(b: &[u8], p: &mut usize) -> bool {
    let mut i = *p;
    while i < b.len() {
        let c = b[i];
        i += 1;
        match c {
            0 => {
                *p = i;
                return false;
            }
            b'\\' => {
                if i == b.len() {
                    *p = i;
                    return false;
                }
                let e = b[i];
                i += 1;
                match e {
                    0 => {
                        *p = i;
                        return false;
                    }
                    b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => {}
                    b'u' => {
                        if b.len() - i < 4 {
                            *p = b.len();
                            return false;
                        }
                        for _ in 0..4 {
                            let h = b[i];
                            i += 1;
                            if !h.is_ascii_hexdigit() {
                                *p = i;
                                return false;
                            }
                        }
                    }
                    _ => {
                        *p = i;
                        return false;
                    }
                }
            }
            b'"' => {
                *p = i;
                return true;
            }
            _ => {}
        }
    }
    *p = i;
    false
}

fn jarray(b: &[u8], p: &mut usize, st: &mut [usize; 6], lvl: usize) -> bool {
    let mut i = *p;
    while i < b.len() {
        i = jskip(b, i);
        if i == b.len() {
            *p = i;
            return false;
        }
        if b[i] != b']' {
            if !jparse(b, &mut i, st, lvl + 1) {
                *p = i;
                return false;
            }
            if i == b.len() {
                *p = i;
                return false;
            }
            match b[i] {
                b',' => {
                    i += 1;
                    continue;
                }
                b']' => {}
                _ => {
                    *p = i;
                    return false;
                }
            }
        }
        st[J_ARRAYN] += 1;
        *p = i + 1;
        return true;
    }
    *p = i;
    false
}

fn jobject(b: &[u8], p: &mut usize, st: &mut [usize; 6], lvl: usize) -> bool {
    let mut i = *p;
    while i < b.len() {
        i = jskip(b, i);
        if i == b.len() {
            *p = i;
            return false;
        }
        if b[i] == b'}' {
            *p = i + 1;
            return true;
        }
        let q = b[i];
        i += 1;
        if q != b'"' {
            *p = i;
            return false;
        }
        if !jstring(b, &mut i) {
            *p = i;
            return false;
        }
        i = jskip(b, i);
        if i == b.len() {
            *p = i;
            return false;
        }
        let c = b[i];
        i += 1;
        if c != b':' {
            *p = i;
            return false;
        }
        if !jparse(b, &mut i, st, lvl + 1) {
            *p = i;
            return false;
        }
        if i == b.len() {
            *p = i;
            return false;
        }
        let c = b[i];
        i += 1;
        match c {
            b',' => continue,
            b'}' => {
                *p = i;
                return true;
            }
            _ => {
                *p = i - 1;
                return false;
            }
        }
    }
    *p = i;
    false
}

fn jnumber(b: &[u8], p: &mut usize) -> bool {
    let mut i = *p;
    let mut got = false;
    if i == b.len() {
        return false;
    }
    if b[i] == b'-' {
        i += 1;
    }
    'out: {
        while i < b.len() {
            if !b[i].is_ascii_digit() {
                break;
            }
            got = true;
            i += 1;
        }
        if i == b.len() {
            break 'out;
        }
        if b[i] == b'.' {
            i += 1;
        }
        while i < b.len() {
            if !b[i].is_ascii_digit() {
                break;
            }
            got = true;
            i += 1;
        }
        if i == b.len() {
            break 'out;
        }
        if got && (b[i] == b'e' || b[i] == b'E') {
            i += 1;
            got = false;
            if i == b.len() {
                break 'out;
            }
            if b[i] == b'+' || b[i] == b'-' {
                i += 1;
            }
            while i < b.len() {
                if !b[i].is_ascii_digit() {
                    break;
                }
                got = true;
                i += 1;
            }
        }
    }
    *p = i;
    got
}

fn jconst(b: &[u8], p: &mut usize, word: &[u8]) -> bool {
    // *ucp += --len - 1: the first character is consumed already
    let start = *p;
    *p = (start + word.len() - 1).min(b.len());
    let mut i = start;
    let mut k = 1;
    while i < b.len() && k < word.len() {
        if b[i] != word[k] {
            return false;
        }
        i += 1;
        k += 1;
    }
    true
}

fn jparse(b: &[u8], p: &mut usize, st: &mut [usize; 6], lvl: usize) -> bool {
    let ouc = jskip(b, *p);
    let mut uc = ouc;
    let mut rv = false;
    if uc != b.len() {
        if lvl > 500 {
            return false;
        }
        let c = b[uc];
        uc += 1;
        let t;
        match c {
            b'"' => {
                rv = jstring(b, &mut uc);
                t = J_STRING;
            }
            b'[' => {
                rv = jarray(b, &mut uc, st, lvl + 1);
                t = J_ARRAY;
            }
            b'{' => {
                rv = jobject(b, &mut uc, st, lvl + 1);
                t = J_OBJECT;
            }
            b't' => {
                rv = jconst(b, &mut uc, b"true");
                t = J_CONSTANT;
            }
            b'f' => {
                rv = jconst(b, &mut uc, b"false");
                t = J_CONSTANT;
            }
            b'n' => {
                rv = jconst(b, &mut uc, b"null");
                t = J_CONSTANT;
            }
            _ => {
                uc -= 1;
                rv = jnumber(b, &mut uc);
                t = J_NUMBER;
            }
        }
        if rv {
            st[t] += 1;
        }
        uc = jskip(b, uc);
    }
    *p = uc;
    if lvl == 0 {
        if !rv {
            return false;
        }
        if uc == b.len() {
            return st[J_ARRAYN] != 0 || st[J_OBJECT] != 0;
        }
        return false;
    }
    rv
}

/// 0: not JSON, 1: JSON, 2: new-line delimited JSON.
pub fn is_json(b: &[u8]) -> i32 {
    let mut st = [0usize; 6];
    // Top level (lvl 0) of json_parse, including the NDJSON second pass.
    let ouc = jskip(b, 0);
    let mut uc = ouc;
    let mut rv = false;
    if uc != b.len() {
        let c = b[uc];
        uc += 1;
        let t;
        match c {
            b'"' => {
                rv = jstring(b, &mut uc);
                t = J_STRING;
            }
            b'[' => {
                rv = jarray(b, &mut uc, &mut st, 1);
                t = J_ARRAY;
            }
            b'{' => {
                rv = jobject(b, &mut uc, &mut st, 1);
                t = J_OBJECT;
            }
            b't' => {
                rv = jconst(b, &mut uc, b"true");
                t = J_CONSTANT;
            }
            b'f' => {
                rv = jconst(b, &mut uc, b"false");
                t = J_CONSTANT;
            }
            b'n' => {
                rv = jconst(b, &mut uc, b"null");
                t = J_CONSTANT;
            }
            _ => {
                uc -= 1;
                rv = jnumber(b, &mut uc);
                t = J_NUMBER;
            }
        }
        if rv {
            st[t] += 1;
        }
        uc = jskip(b, uc);
    }
    if !rv {
        return 0;
    }
    if uc == b.len() {
        return i32::from(st[J_ARRAYN] != 0 || st[J_OBJECT] != 0);
    }
    if b[ouc] == b[uc] && jparse(b, &mut uc, &mut st, 1) {
        return if st[J_ARRAYN] != 0 || st[J_OBJECT] != 0 { 2 } else { 0 };
    }
    0
}

// ---- CSV (is_csv.c) ----

fn eatquote(b: &[u8], mut i: usize) -> usize {
    let mut quote = false;
    while i < b.len() {
        let c = b[i];
        i += 1;
        if c != b'"' {
            if quote {
                return i - 1;
            }
            continue;
        }
        quote = !quote;
    }
    b.len()
}

pub fn is_csv(b: &[u8]) -> bool {
    let (mut nf, mut tf, mut nl) = (0usize, 0usize, 0usize);
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        i += 1;
        match c {
            b'"' => i = eatquote(b, i),
            b',' => nf += 1,
            b'\n' => {
                nl += 1;
                if nl == 10 {
                    return tf != 0 && tf == nf;
                }
                if tf == 0 {
                    if nf == 0 {
                        return false;
                    }
                    tf = nf;
                } else if tf != nf {
                    return false;
                }
                nf = 0;
            }
            _ => {}
        }
    }
    tf != 0 && nl >= 2
}

// ---- tar (is_tar.c) ----

fn from_oct(w: &[u8]) -> i32 {
    let mut i = 0;
    let mut digs = w.len();
    if digs == 0 {
        return -1;
    }
    while i < w.len() && is_c_space(w[i]) {
        i += 1;
        if digs == 0 {
            return -1;
        }
        digs -= 1;
    }
    let mut v: i32 = 0;
    while digs > 0 && i < w.len() && (b'0'..=b'7').contains(&w[i]) {
        v = (v << 3) | i32::from(w[i] - b'0');
        i += 1;
        digs -= 1;
    }
    if digs > 0 && i < w.len() && w[i] != 0 && !is_c_space(w[i]) {
        return -1;
    }
    v
}

fn is_c_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// 0 not tar; 1 old tar; 2 POSIX tar; 3 GNU tar.
pub fn is_tar(b: &[u8]) -> i32 {
    if b.len() < 512 {
        return 0;
    }
    let h = &b[..512];
    const GP: &[u8] = b"/gpkg-1\0";
    if let Some(nul) = h[..100].iter().position(|&c| c == 0)
        && nul >= GP.len()
        && h[nul + 1 - GP.len()..=nul] == *GP
    {
        return 0;
    }
    let recsum = from_oct(&h[148..156]);
    let mut sum: i32 = h.iter().map(|&c| i32::from(c)).sum();
    for &c in &h[148..156] {
        sum -= i32::from(c);
    }
    sum += 32 * 8;
    if sum != recsum {
        return 0;
    }
    let magic = &h[257..265];
    if magic == b"ustar  \0" {
        3
    } else if magic[..6] == *b"ustar\0" {
        2
    } else {
        1
    }
}

// ---- SIMH tape images (is_simh.c) ----

fn simh_len(b: &[u8], at: &mut usize) -> u32 {
    let n = u32::from_le_bytes([b[*at], b[*at + 1], b[*at + 2], b[*at + 3]]);
    *at += 4;
    if n == 0xffff_ffff {
        return n;
    }
    let mut n = n & 0x00ff_ffff;
    if n & 1 != 0 {
        n += 1;
    }
    n
}

pub fn is_simh(b: &[u8]) -> bool {
    let (mut nt, mut nr) = (0usize, 0usize);
    let mut at = 0usize;
    while b.len() - at >= 4 {
        let nbytes = simh_len(b, &mut at);
        if (nt > 0 || nr > 0) && nbytes == 0xffff_ffff {
            break;
        }
        if nbytes == 0 {
            nt += 1;
            if nt == 10 {
                break;
            }
            continue;
        }
        at += nbytes as usize;
        if b.len() < at || b.len() - at < 4 {
            break;
        }
        let cbytes = simh_len(b, &mut at);
        if nbytes != cbytes {
            return false;
        }
        nr += 1;
    }
    if nt * 4 == at {
        return false;
    }
    nr != 0
}
