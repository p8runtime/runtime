//! UTF-7 (RFC 2152) and its IMAP variant (RFC 3501), over UTF-16 units in
//! modified base64.
//!
//! Reference: $PHP_SRC_PATH/ext/mbstring/libmbfl/filters/mbfilter_utf7.c,
//! $PHP_SRC_PATH/ext/mbstring/libmbfl/filters/mbfilter_utf7imap.c

use crate::enc::BAD;

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_val(c: u8, imap: bool) -> Option<u32> {
    match c {
        b'A'..=b'Z' => Some((c - b'A') as u32),
        b'a'..=b'z' => Some((c - b'a') as u32 + 26),
        b'0'..=b'9' => Some((c - b'0') as u32 + 52),
        b'+' if !imap => Some(62),
        b'/' if !imap => Some(63),
        b',' if imap => Some(63),
        b'+' if imap => Some(62),
        _ => None,
    }
}

fn b64_char(v: u32, imap: bool) -> u8 {
    let c = B64[(v & 0x3F) as usize];
    if imap && c == b'/' { b',' } else { c }
}

/// The 16-bit units of a base64 run, the leftover bits non-zero meaning bad.
struct Run {
    bits: u32,
    n: u32,
    units: Vec<u32>,
}

impl Run {
    fn new() -> Run {
        Run { bits: 0, n: 0, units: Vec::new() }
    }

    fn push(&mut self, v: u32) {
        self.bits = (self.bits << 6) | v;
        self.n += 6;
        if self.n >= 16 {
            self.n -= 16;
            self.units.push((self.bits >> self.n) & 0xFFFF);
            self.bits &= (1 << self.n) - 1;
        }
    }

    /// The run ends: the characters, with BAD for a stray surrogate or leftover bits.
    fn finish(&mut self, out: &mut Vec<u32>) {
        let mut high: Option<u32> = None;
        for &u in &self.units {
            if let Some(h) = high.take() {
                if (0xDC00..0xE000).contains(&u) {
                    out.push(0x10000 + ((h - 0xD800) << 10) + (u - 0xDC00));
                    continue;
                }
                out.push(BAD);
            }
            if (0xD800..0xDC00).contains(&u) {
                high = Some(u);
            } else if (0xDC00..0xE000).contains(&u) {
                out.push(BAD);
            } else {
                out.push(u);
            }
        }
        if high.is_some() || self.n >= 6 || (self.n > 0 && self.bits != 0) {
            out.push(BAD);
        }
        *self = Run::new();
    }
}

pub fn decode(s: &[u8], imap: bool) -> Vec<u32> {
    let shift = if imap { b'&' } else { b'+' };
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        i += 1;
        if c != shift {
            if c < 0x80 && (!imap || (0x20..0x7F).contains(&c)) {
                out.push(c as u32);
            } else {
                out.push(BAD);
            }
            continue;
        }
        // A shift: "+-" is the shift character itself.
        if i < s.len() && s[i] == b'-' {
            out.push(shift as u32);
            i += 1;
            continue;
        }
        let mut run = Run::new();
        while i < s.len() {
            match b64_val(s[i], imap) {
                Some(v) => {
                    run.push(v);
                    i += 1;
                }
                None => break,
            }
        }
        run.finish(&mut out);
        if i < s.len() {
            if s[i] == b'-' {
                i += 1;
            } else if s[i] >= 0x80 {
                out.push(BAD);
                i += 1;
            }
        }
    }
    out
}

fn direct(c: u32, imap: bool) -> Option<u8> {
    if imap {
        return if (0x20..0x7F).contains(&c) && c != b'&' as u32 { Some(c as u8) } else { None };
    }
    let b = u8::try_from(c).ok()?;
    if b.is_ascii_alphanumeric() || matches!(b, 0 | b'/' | b'-' | b' ' | b'\t' | b'\r' | b'\n' | b'\'' | b'(' | b')' | b',' | b'.' | b':' | b'?') {
        Some(b)
    } else {
        None
    }
}

pub fn encode(cps: &[u32], imap: bool) -> Vec<u8> {
    let shift = if imap { b'&' } else { b'+' };
    let mut out = Vec::new();
    let mut units: Vec<u32> = Vec::new();
    let flush = |units: &mut Vec<u32>, out: &mut Vec<u8>, next: Option<u8>| {
        if units.is_empty() {
            return;
        }
        out.push(shift);
        let mut bits = 0u32;
        let mut n = 0u32;
        for &u in units.iter() {
            bits = (bits << 16) | u;
            n += 16;
            while n >= 6 {
                n -= 6;
                out.push(b64_char(bits >> n, imap));
                bits &= (1 << n) - 1;
            }
        }
        if n > 0 {
            out.push(b64_char(bits << (6 - n), imap));
        }
        // A character that is itself base64 (or '-') needs the terminator.
        let need_dash = imap
            || match next {
                None => true,
                Some(b) => b.is_ascii_alphanumeric() || b == b'/' || b == b'-' || b == 0 || b == b'+',
            };
        if need_dash {
            out.push(b'-');
        }
        units.clear();
    };
    for &w in cps {
        if w == BAD || w >= 0x110000 || (0xD800..0xE000).contains(&w) {
            flush(&mut units, &mut out, None);
            out.push(b'?');
            continue;
        }
        if imap && w == b'&' as u32 {
            flush(&mut units, &mut out, None);
            out.extend_from_slice(b"&-");
            continue;
        }
        if let Some(b) = direct(w, imap) {
            flush(&mut units, &mut out, Some(b));
            out.push(b);
        } else if w >= 0x10000 {
            let v = w - 0x10000;
            units.push(0xD800 + (v >> 10));
            units.push(0xDC00 + (v & 0x3FF));
        } else {
            units.push(w);
        }
    }
    flush(&mut units, &mut out, None);
    out
}
