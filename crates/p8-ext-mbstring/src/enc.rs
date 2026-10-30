//! The encodings: names and aliases, and the two directions of each between
//! bytes and a string of code points (`BAD` marks input that is not valid).
//!
//! Reference: $PHP_SRC_PATH/ext/mbstring/libmbfl/filters/*.c,
//! $PHP_SRC_PATH/ext/mbstring/libmbfl/mbfl/mbfl_convert.c - mbfl_filt_conv_illegal_output

use crate::gendata::ENTITIES;
use crate::sbtables::{CP1252, SB};
use std::sync::OnceLock;

/// The marker for input that is not valid in its encoding (MBFL_BAD_INPUT).
pub const BAD: u32 = u32::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Base64,
    Uuencode,
    HtmlEnt,
    Qprint,
    Bit7,
    Bit8,
    Ucs4,
    Ucs4Be,
    Ucs4Le,
    Ucs2,
    Ucs2Be,
    Ucs2Le,
    Utf32,
    Utf32Be,
    Utf32Le,
    Utf16,
    Utf16Be,
    Utf16Le,
    Utf8,
    Utf7,
    Utf7Imap,
    Ascii,
    Cp1252,
    Latin1,
    Sb(usize),
    /// A multi-byte encoding by its WHATWG table (encoding_rs): Big5,
    /// EUC-JP (the same mappings as libmbfl's for everything but its
    /// vendor extensions).
    Table(&'static encoding_rs::Encoding),
}

pub struct Enc {
    pub kind: Kind,
    pub name: &'static str,
    pub mime: Option<&'static str>,
    pub aliases: &'static [&'static str],
}

fn all() -> &'static [Enc] {
    static ALL: OnceLock<Vec<Enc>> = OnceLock::new();
    ALL.get_or_init(|| {
        use Kind::*;
        let e = |kind, name, mime, aliases| Enc { kind, name, mime, aliases };
        let mut v = vec![
            e(Base64, "BASE64", Some("BASE64"), &[][..]),
            e(Uuencode, "UUENCODE", Some("x-uuencode"), &[]),
            e(HtmlEnt, "HTML-ENTITIES", Some("HTML-ENTITIES"), &["HTML", "html"]),
            e(Qprint, "Quoted-Printable", Some("Quoted-Printable"), &["qprint"]),
            e(Bit7, "7bit", Some("7bit"), &[]),
            e(Bit8, "8bit", Some("8bit"), &["binary"]),
            e(Ucs4, "UCS-4", Some("UCS-4"), &["ISO-10646-UCS-4", "UCS4"]),
            e(Ucs4Be, "UCS-4BE", Some("UCS-4BE"), &["byte4be"]),
            e(Ucs4Le, "UCS-4LE", Some("UCS-4LE"), &["byte4le"]),
            e(Ucs2, "UCS-2", Some("UCS-2"), &["ISO-10646-UCS-2", "UCS2", "UNICODE"]),
            e(Ucs2Be, "UCS-2BE", Some("UCS-2BE"), &["byte2be"]),
            e(Ucs2Le, "UCS-2LE", Some("UCS-2LE"), &["byte2le"]),
            e(Utf32, "UTF-32", Some("UTF-32"), &["utf32"]),
            e(Utf32Be, "UTF-32BE", Some("UTF-32BE"), &[]),
            e(Utf32Le, "UTF-32LE", Some("UTF-32LE"), &[]),
            e(Utf16, "UTF-16", Some("UTF-16"), &["utf16"]),
            e(Utf16Be, "UTF-16BE", Some("UTF-16BE"), &[]),
            e(Utf16Le, "UTF-16LE", Some("UTF-16LE"), &[]),
            e(Utf8, "UTF-8", Some("UTF-8"), &["utf8"]),
            e(Utf7, "UTF-7", Some("UTF-7"), &["utf7"]),
            e(Utf7Imap, "UTF7-IMAP", None, &["mUTF-7"]),
            e(
                Ascii,
                "ASCII",
                Some("US-ASCII"),
                &[
                    "ANSI_X3.4-1968",
                    "iso-ir-6",
                    "ANSI_X3.4-1986",
                    "ISO_646.irv:1991",
                    "US-ASCII",
                    "ISO646-US",
                    "us",
                    "IBM367",
                    "IBM-367",
                    "cp367",
                    "csASCII",
                ],
            ),
            e(Table(encoding_rs::EUC_JP), "EUC-JP", Some("EUC-JP"), &["EUC", "EUC_JP", "eucJP", "x-euc-jp"]),
        ];
        // The order of mbfl_encoding_ptr_list: 1252, 1254, then 8859-1.. and
        // the rest after the CJK ones.
        let mut sb: Vec<Enc> = SB
            .iter()
            .enumerate()
            .map(|(i, s)| Enc { kind: Sb(i), name: s.name, mime: Some(s.mime), aliases: s.aliases })
            .collect();
        let pos = |sb: &Vec<Enc>, n: &str| sb.iter().position(|x| x.name == n).unwrap();
        let v1254 = sb.remove(pos(&sb, "Windows-1254"));
        let mut iso: Vec<Enc> = Vec::new();
        let mut rest: Vec<Enc> = Vec::new();
        for x in sb {
            if x.name.starts_with("ISO-8859") {
                iso.push(x)
            } else {
                rest.push(x)
            }
        }
        v.push(e(Cp1252, "Windows-1252", Some("Windows-1252"), &["cp1252"]));
        v.push(v1254);
        v.push(e(Latin1, "ISO-8859-1", Some("ISO-8859-1"), &["ISO8859-1", "latin1"]));
        v.extend(iso);
        // cp1251, cp866, koi8r, koi8u, armscii8, cp850 as libmbfl lists them.
        for n in ["Windows-1251", "CP866", "KOI8-R", "KOI8-U", "ArmSCII-8", "CP850"] {
            if let Some(p) = rest.iter().position(|x| x.name == n) {
                v.push(rest.remove(p));
            }
        }
        v.extend(rest);
        v.push(e(Table(encoding_rs::BIG5), "BIG-5", Some("BIG5"), &["CN-BIG5", "BIG-FIVE", "BIGFIVE"]));
        v
    })
}

/// The encodings in the order mb_list_encodings() gives them.
pub fn list() -> &'static [Enc] {
    all()
}

/// An encoding by name or alias (any case).
pub fn lookup(name: &str) -> Option<&'static Enc> {
    all().iter().find(|e| e.name.eq_ignore_ascii_case(name) || e.aliases.iter().any(|a| a.eq_ignore_ascii_case(name)))
}

pub fn utf8() -> &'static Enc {
    lookup("UTF-8").unwrap()
}

impl Enc {
    /// Bytes per character when every character is the same width (0: not).
    pub fn fixed_width(&self) -> usize {
        match self.kind {
            Kind::Ucs2 | Kind::Ucs2Be | Kind::Ucs2Le => 2,
            Kind::Ucs4 | Kind::Ucs4Be | Kind::Ucs4Le | Kind::Utf32 | Kind::Utf32Be | Kind::Utf32Le => 4,
            Kind::Ascii | Kind::Cp1252 | Kind::Latin1 | Kind::Sb(_) | Kind::Bit7 | Kind::Bit8 => 1,
            _ => 0,
        }
    }

    pub fn is_utf8(&self) -> bool {
        self.kind == Kind::Utf8
    }

    /// Whether the bytes are a transfer encoding, not text (base64 and the like).
    pub fn is_transfer(&self) -> bool {
        matches!(self.kind, Kind::Base64 | Kind::Qprint | Kind::Uuencode)
    }
}

pub fn mblen_utf8(b: u8) -> usize {
    match b {
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => 1,
    }
}

// ---- decoding ----

pub fn decode(enc: &Enc, s: &[u8]) -> Vec<u32> {
    use Kind::*;
    match enc.kind {
        Utf8 => decode_utf8(s),
        Ascii | Bit7 => s.iter().map(|&b| if b < 0x80 { b as u32 } else { BAD }).collect(),
        Latin1 | Bit8 => s.iter().map(|&b| b as u32).collect(),
        Base64 => crate::transfer::base64_decode(s),
        Qprint => crate::transfer::qprint_decode(s),
        Uuencode => crate::transfer::uu_decode(s),
        Cp1252 => s
            .iter()
            .map(|&b| {
                if (0x80..0xA0).contains(&b) {
                    match CP1252[b as usize - 0x80] {
                        0 => BAD,
                        c => c as u32,
                    }
                } else {
                    b as u32
                }
            })
            .collect(),
        Sb(i) => {
            let t = &SB[i];
            s.iter()
                .map(|&b| {
                    if (b as usize) < t.min {
                        b as u32
                    } else {
                        match t.table[b as usize - t.min] {
                            0 => BAD,
                            c => c as u32,
                        }
                    }
                })
                .collect()
        }
        Ucs2 | Ucs2Be | Ucs2Le => decode_ucs2(s, enc.kind),
        Ucs4 | Ucs4Be | Ucs4Le => decode_wide(s, enc.kind, false),
        Utf32 | Utf32Be | Utf32Le => decode_wide(s, enc.kind, true),
        Utf16 | Utf16Be | Utf16Le => decode_utf16(s, enc.kind),
        HtmlEnt => decode_html(s),
        Utf7 => crate::utf7::decode(s, false),
        Utf7Imap => crate::utf7::decode(s, true),
        Table(table) => decode_table(table, s),
    }
}

/// A multi-byte encoding's bytes as code points, each malformed sequence
/// BAD (as libmbfl marks them: one per byte it gives up on).
fn decode_table(table: &'static encoding_rs::Encoding, s: &[u8]) -> Vec<u32> {
    let mut decoder = table.new_decoder_without_bom_handling();
    let mut out = Vec::with_capacity(s.len());
    let mut buf = String::with_capacity(s.len() * 3 + 16);
    let mut rest = s;
    loop {
        buf.clear();
        let (result, read) = decoder.decode_to_string_without_replacement(rest, &mut buf, true);
        out.extend(buf.chars().map(|c| c as u32));
        rest = &rest[read..];
        match result {
            encoding_rs::DecoderResult::InputEmpty => return out,
            encoding_rs::DecoderResult::OutputFull => buf.reserve(rest.len() * 3 + 16),
            encoding_rs::DecoderResult::Malformed(bad, _) => {
                out.extend(std::iter::repeat_n(BAD, usize::from(bad)));
            }
        }
    }
}

fn decode_utf8(s: &[u8]) -> Vec<u32> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    let n = s.len();
    let cont = |i: usize| i < n && s[i] & 0xC0 == 0x80;
    while i < n {
        let c = s[i];
        if c < 0x80 {
            out.push(c as u32);
            i += 1;
        } else if (0xC2..=0xDF).contains(&c) {
            if cont(i + 1) {
                out.push(((c as u32 & 0x1F) << 6) | (s[i + 1] as u32 & 0x3F));
                i += 2;
            } else {
                out.push(BAD);
                i += 1;
            }
        } else if (0xE0..=0xEF).contains(&c) {
            let (lo, hi) = match c {
                0xE0 => (0xA0, 0xBF),
                0xED => (0x80, 0x9F),
                _ => (0x80, 0xBF),
            };
            if i + 1 < n && s[i + 1] >= lo && s[i + 1] <= hi {
                if cont(i + 2) {
                    out.push(((c as u32 & 0xF) << 12) | ((s[i + 1] as u32 & 0x3F) << 6) | (s[i + 2] as u32 & 0x3F));
                    i += 3;
                } else {
                    out.push(BAD);
                    i += 2;
                }
            } else {
                out.push(BAD);
                i += 1;
            }
        } else if (0xF0..=0xF4).contains(&c) {
            let (lo, hi) = match c {
                0xF0 => (0x90, 0xBF),
                0xF4 => (0x80, 0x8F),
                _ => (0x80, 0xBF),
            };
            if i + 1 < n && s[i + 1] >= lo && s[i + 1] <= hi {
                if cont(i + 2) {
                    if cont(i + 3) {
                        out.push(
                            ((c as u32 & 7) << 18)
                                | ((s[i + 1] as u32 & 0x3F) << 12)
                                | ((s[i + 2] as u32 & 0x3F) << 6)
                                | (s[i + 3] as u32 & 0x3F),
                        );
                        i += 4;
                    } else {
                        out.push(BAD);
                        i += 3;
                    }
                } else {
                    out.push(BAD);
                    i += 2;
                }
            } else {
                out.push(BAD);
                i += 1;
            }
        } else {
            out.push(BAD);
            i += 1;
        }
    }
    out
}

fn rd16(b: &[u8], le: bool) -> u32 {
    if le { b[0] as u32 | (b[1] as u32) << 8 } else { (b[0] as u32) << 8 | b[1] as u32 }
}

fn rd32(b: &[u8], le: bool) -> u32 {
    if le {
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    } else {
        u32::from_be_bytes([b[0], b[1], b[2], b[3]])
    }
}

fn decode_ucs2(s: &[u8], kind: Kind) -> Vec<u32> {
    let mut le = kind == Kind::Ucs2Le;
    let mut i = 0;
    if kind == Kind::Ucs2 && s.len() >= 2 {
        match (s[0], s[1]) {
            (0xFF, 0xFE) => {
                le = true;
                i = 2
            }
            (0xFE, 0xFF) => i = 2,
            _ => {}
        }
    }
    let mut out = Vec::new();
    while i + 2 <= s.len() {
        out.push(rd16(&s[i..], le));
        i += 2;
    }
    if i < s.len() {
        out.push(BAD);
    }
    out
}

fn decode_wide(s: &[u8], kind: Kind, strict: bool) -> Vec<u32> {
    let mut le = matches!(kind, Kind::Ucs4Le | Kind::Utf32Le);
    let mut i = 0;
    if matches!(kind, Kind::Ucs4 | Kind::Utf32) && s.len() >= 4 {
        if s[..4] == [0xFF, 0xFE, 0, 0] {
            le = true;
            i = 4;
        } else if s[..4] == [0, 0, 0xFE, 0xFF] {
            i = 4;
        }
    }
    let mut out = Vec::new();
    while i + 4 <= s.len() {
        let w = rd32(&s[i..], le);
        let bad = if strict { w >= 0x110000 || (0xD800..0xE000).contains(&w) } else { w == BAD };
        out.push(if bad { BAD } else { w });
        i += 4;
    }
    if i < s.len() {
        out.push(BAD);
    }
    out
}

fn decode_utf16(s: &[u8], kind: Kind) -> Vec<u32> {
    let mut le = kind == Kind::Utf16Le;
    let mut i = 0;
    if kind == Kind::Utf16 && s.len() >= 2 {
        match (s[0], s[1]) {
            (0xFF, 0xFE) => {
                le = true;
                i = 2
            }
            (0xFE, 0xFF) => i = 2,
            _ => {}
        }
    }
    let mut out = Vec::new();
    while i + 2 <= s.len() {
        let u = rd16(&s[i..], le);
        i += 2;
        if (0xD800..0xDC00).contains(&u) {
            if i + 2 <= s.len() {
                let l = rd16(&s[i..], le);
                if (0xDC00..0xE000).contains(&l) {
                    out.push(0x10000 + ((u - 0xD800) << 10) + (l - 0xDC00));
                    i += 2;
                    continue;
                }
            }
            out.push(BAD);
        } else if (0xDC00..0xE000).contains(&u) {
            out.push(BAD);
        } else {
            out.push(u);
        }
    }
    if i < s.len() {
        out.push(BAD);
    }
    out
}

fn decode_html(s: &[u8]) -> Vec<u32> {
    let is_ent = |c: u8| c.is_ascii_alphanumeric() || c == b'#';
    let mut out = Vec::new();
    let mut p = 0;
    let e = s.len();
    while p < e {
        let c = s[p];
        p += 1;
        if c != b'&' {
            out.push(c as u32);
            continue;
        }
        let mut t = p;
        while t < e && is_ent(s[t]) {
            t += 1;
        }
        let mut decoded = None;
        if t < e && s[t] == b';' {
            if p < e && s[p] == b'#' && e - p >= 2 {
                let mut d = p + 1;
                let mut value: u32 = 0;
                let mut ok = true;
                if s[d] == b'x' || s[d] == b'X' {
                    d += 1;
                    if d == t {
                        ok = false;
                    }
                    while ok && d < t {
                        match (s[d] as char).to_digit(16) {
                            Some(v) => value = value.wrapping_mul(16).wrapping_add(v),
                            None => ok = false,
                        }
                        d += 1;
                    }
                } else {
                    if d == t {
                        ok = false;
                    }
                    while ok && d < t {
                        match (s[d] as char).to_digit(10) {
                            Some(v) => value = value.wrapping_mul(10).wrapping_add(v),
                            None => ok = false,
                        }
                        d += 1;
                    }
                }
                if ok && value <= 0x10FFFF {
                    decoded = Some(value);
                }
            } else if t > p {
                let name = &s[p..t];
                decoded = ENTITIES.iter().find(|(n, _)| n.as_bytes() == name).map(|(_, c)| *c);
            }
        }
        match decoded {
            Some(v) => {
                out.push(v);
                p = t + 1;
            }
            None => {
                out.push(b'&' as u32);
                while p < t {
                    out.push(s[p] as u32);
                    p += 1;
                }
                if t < e && s[t] == b';' {
                    out.push(b';' as u32);
                    p += 1;
                }
            }
        }
    }
    out
}

// ---- encoding ----

/// What to do with a character an encoding can't have.
#[derive(Clone, Copy)]
pub struct Subst {
    /// 0: a character, 1: none, 2: long, 3: entity.
    pub mode: u8,
    pub ch: u32,
}

impl Subst {
    pub const DEFAULT: Subst = Subst { mode: 0, ch: b'?' as u32 };
}

/// Write one character; false if the encoding can't have it.
fn put(enc: &Enc, w: u32, out: &mut Vec<u8>) -> bool {
    use Kind::*;
    if w == BAD {
        return false;
    }
    match enc.kind {
        Utf8 => {
            if w >= 0x110000 || (0xD800..0xE000).contains(&w) {
                return false;
            }
            push_utf8(w, out);
        }
        Ascii | Bit7 => {
            if w >= 0x80 {
                return false;
            }
            out.push(w as u8)
        }
        Latin1 | Bit8 | Base64 | Qprint | Uuencode => {
            if w >= 0x100 {
                return false;
            }
            out.push(w as u8)
        }
        Cp1252 => {
            if w >= 0x100 {
                match CP1252.iter().position(|&c| c as u32 == w && c != 0) {
                    Some(n) => out.push(0x80 + n as u8),
                    None => return false,
                }
            } else if w <= 0x7F || w >= 0xA0 || matches!(w, 0x81 | 0x8D | 0x8F | 0x90 | 0x9D) {
                out.push(w as u8)
            } else {
                return false;
            }
        }
        Sb(i) => {
            let t = &SB[i];
            if (w as usize) < t.min {
                if t.name == "ArmSCII-8" && (0x28..=0x2F).contains(&w) {
                    out.push([0xA5, 0xA4, 0x2A, 0x2B, 0xAB, 0xAC, 0xA9, 0x2F][w as usize - 0x28]);
                } else {
                    out.push(w as u8)
                }
            } else {
                match t.table.iter().position(|&c| c as u32 == w && c != 0) {
                    Some(n) => out.push((t.min + n) as u8),
                    None => return false,
                }
            }
        }
        Ucs2 | Ucs2Be => {
            if w > 0xFFFF {
                return false;
            }
            out.extend_from_slice(&(w as u16).to_be_bytes())
        }
        Ucs2Le => {
            if w > 0xFFFF {
                return false;
            }
            out.extend_from_slice(&(w as u16).to_le_bytes())
        }
        Ucs4 | Ucs4Be => out.extend_from_slice(&w.to_be_bytes()),
        Ucs4Le => out.extend_from_slice(&w.to_le_bytes()),
        Utf32 | Utf32Be | Utf32Le => {
            if w >= 0x110000 || (0xD800..0xE000).contains(&w) {
                return false;
            }
            if enc.kind == Utf32Le {
                out.extend_from_slice(&w.to_le_bytes())
            } else {
                out.extend_from_slice(&w.to_be_bytes())
            }
        }
        Utf16 | Utf16Be | Utf16Le => {
            if w >= 0x110000 || (0xD800..0xE000).contains(&w) {
                return false;
            }
            let le = enc.kind == Utf16Le;
            let mut unit = |u: u16| out.extend_from_slice(&if le { u.to_le_bytes() } else { u.to_be_bytes() });
            if w >= 0x10000 {
                let v = w - 0x10000;
                unit(0xD800 + (v >> 10) as u16);
                unit(0xDC00 + (v & 0x3FF) as u16);
            } else {
                unit(w as u16);
            }
        }
        HtmlEnt => {
            if w < 0x80 {
                out.push(w as u8);
            } else {
                out.push(b'&');
                match ENTITIES.iter().find(|(_, c)| *c == w) {
                    Some((n, _)) => out.extend_from_slice(n.as_bytes()),
                    None => {
                        out.push(b'#');
                        out.extend_from_slice(w.to_string().as_bytes());
                    }
                }
                out.push(b';');
            }
        }
        Utf7 | Utf7Imap => return false,
        Table(table) => {
            let Some(c) = char::from_u32(w) else { return false };
            let mut text = [0u8; 4];
            let (bytes, _, unmappable) = table.encode(c.encode_utf8(&mut text));
            if unmappable {
                return false;
            }
            out.extend_from_slice(&bytes);
        }
    }
    true
}

pub fn push_utf8(w: u32, out: &mut Vec<u8>) {
    if w < 0x80 {
        out.push(w as u8)
    } else if w < 0x800 {
        out.push(0xC0 | (w >> 6) as u8);
        out.push(0x80 | (w & 0x3F) as u8);
    } else if w < 0x10000 {
        out.push(0xE0 | (w >> 12) as u8);
        out.push(0x80 | ((w >> 6) & 0x3F) as u8);
        out.push(0x80 | (w & 0x3F) as u8);
    } else {
        out.push(0xF0 | (w >> 18) as u8);
        out.push(0x80 | ((w >> 12) & 0x3F) as u8);
        out.push(0x80 | ((w >> 6) & 0x3F) as u8);
        out.push(0x80 | (w & 0x3F) as u8);
    }
}

fn put_ascii(enc: &Enc, s: &str, out: &mut Vec<u8>) {
    for b in s.bytes() {
        put(enc, b as u32, out);
    }
}

fn hex(w: u32) -> String {
    format!("{w:X}")
}

fn illegal(enc: &Enc, w: u32, sub: &Subst, errs: &mut usize, out: &mut Vec<u8>) {
    *errs += 1;
    match sub.mode {
        0 => {
            if !put(enc, sub.ch, out) && sub.ch != b'?' as u32 {
                put(enc, b'?' as u32, out);
            }
        }
        2 => {
            if w != BAD {
                put_ascii(enc, "U+", out);
                put_ascii(enc, &hex(w), out);
            } else if !put(enc, sub.ch, out) && sub.ch != b'?' as u32 {
                put(enc, b'?' as u32, out);
            }
        }
        3 => {
            if w != BAD {
                put_ascii(enc, "&#x", out);
                put_ascii(enc, &hex(w), out);
                put_ascii(enc, ";", out);
            } else if !put(enc, sub.ch, out) && sub.ch != b'?' as u32 {
                put(enc, b'?' as u32, out);
            }
        }
        _ => {}
    }
}

/// Characters to bytes; `errs` counts the characters that couldn't be had.
pub fn encode(enc: &Enc, cps: &[u32], sub: &Subst, errs: &mut usize) -> Vec<u8> {
    use Kind::*;
    let mut out = Vec::with_capacity(cps.len());
    match enc.kind {
        Base64 => return crate::transfer::base64_encode(cps),
        Qprint => return crate::transfer::qprint_encode(cps),
        Uuencode => return crate::transfer::uu_encode(cps),
        Utf7 => return crate::utf7::encode(cps, false),
        Utf7Imap => return crate::utf7::encode(cps, true),
        _ => {}
    }
    for &w in cps {
        if !put(enc, w, &mut out) {
            illegal(enc, w, sub, errs, &mut out);
        }
    }
    out
}
