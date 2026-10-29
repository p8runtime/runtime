//! The transfer encodings mbstring still handles: base64, quoted-printable
//! and uuencode.
//!
//! Reference: $PHP_SRC_PATH/ext/mbstring/libmbfl/filters/mbfilter_base64.c,
//! mbfilter_qprint.c, mbfilter_uuencode.c

use crate::enc::BAD;

fn hexv(c: u8) -> Option<u32> {
    (c as char).to_digit(16)
}

fn base64_val(c: u8) -> Option<u32> {
    match c {
        b'A'..=b'Z' => Some((c - b'A') as u32),
        b'a'..=b'z' => Some((c - b'a') as u32 + 26),
        b'0'..=b'9' => Some((c - b'0') as u32 + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

pub fn base64_decode(s: &[u8]) -> Vec<u32> {
    let mut out = Vec::new();
    let (mut bits, mut cache) = (0u32, 0u32);
    for &c in s {
        if matches!(c, b'\r' | b'\n' | b' ' | b'\t' | b'=') {
            continue;
        }
        match base64_val(c) {
            None => out.push(BAD),
            Some(v) => {
                bits += 6;
                cache = (cache << 6) | v;
                if bits == 24 {
                    out.extend([(cache >> 16) & 0xFF, (cache >> 8) & 0xFF, cache & 0xFF]);
                    bits = 0;
                    cache = 0;
                }
            }
        }
    }
    if bits == 18 {
        out.extend([(cache >> 10) & 0xFF, (cache >> 2) & 0xFF]);
    } else if bits == 12 {
        out.push((cache >> 4) & 0xFF);
    }
    out
}

pub fn base64_encode(cps: &[u32]) -> Vec<u8> {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let mut chars = 0;
    for chunk in cps.chunks(3) {
        if chars > 72 {
            out.extend_from_slice(b"\r\n");
            chars = 0;
        }
        let b: Vec<u32> = chunk.iter().map(|w| w & 0xFF).collect();
        let n = (b[0] << 16) | (b.get(1).copied().unwrap_or(0) << 8) | b.get(2).copied().unwrap_or(0);
        out.push(T[((n >> 18) & 63) as usize]);
        out.push(T[((n >> 12) & 63) as usize]);
        match b.len() {
            1 => out.extend_from_slice(b"=="),
            2 => {
                out.push(T[((n >> 6) & 63) as usize]);
                out.push(b'=');
            }
            _ => {
                out.push(T[((n >> 6) & 63) as usize]);
                out.push(T[(n & 63) as usize]);
            }
        }
        chars += 4;
    }
    out
}

pub fn qprint_decode(s: &[u8]) -> Vec<u32> {
    let mut out = Vec::new();
    let mut p = 0;
    let e = s.len();
    while p < e {
        let c = s[p];
        p += 1;
        if c == b'=' && p < e {
            let c2 = s[p];
            p += 1;
            if hexv(c2).is_some() && p < e {
                let c3 = s[p];
                p += 1;
                if let (Some(a), Some(b)) = (hexv(c2), hexv(c3)) {
                    out.push(a << 4 | b);
                } else {
                    out.extend([b'=' as u32, c2 as u32, c3 as u32]);
                }
            } else if c2 == b'\r' && p < e {
                let c3 = s[p];
                p += 1;
                if c3 != b'\n' {
                    out.push(c3 as u32);
                }
            } else if c2 != b'\n' {
                out.extend([b'=' as u32, c2 as u32]);
            }
        } else {
            out.push(c as u32);
        }
    }
    out
}

pub fn qprint_encode(cps: &[u32]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut chars = 0;
    for &w in cps {
        if w == 0 {
            out.push(0);
            chars = 0;
            continue;
        } else if w == b'\n' as u32 {
            out.extend_from_slice(b"\r\n");
            chars = 0;
            continue;
        } else if w == b'\r' as u32 {
            continue;
        }
        if chars >= 72 {
            out.extend_from_slice(b"=\r\n");
            chars = 0;
        }
        if w >= 0x80 || w == b'=' as u32 {
            out.push(b'=');
            out.push(b"0123456789ABCDEF"[((w >> 4) & 0xF) as usize]);
            out.push(b"0123456789ABCDEF"[(w & 0xF) as usize]);
            chars += 3;
        } else {
            out.push(w as u8);
            chars += 1;
        }
    }
    out
}

fn uu_dec(c: u8) -> u32 {
    (c.wrapping_sub(b' ') & 0o77) as u32
}

pub fn uu_decode(s: &[u8]) -> Vec<u32> {
    let mut out = Vec::new();
    let mut p = 0;
    let e = s.len();
    // 0: looking for "begin ", 1: size, 2: a group, 3: skip the newline
    let mut state = 0;
    let mut size = 0u32;
    while p < e {
        let c = s[p];
        p += 1;
        match state {
            0 => {
                if c == b'b' && e - p >= 5 && &s[p..p + 5] == b"egin " {
                    p += 5;
                    while p < e {
                        let x = s[p];
                        p += 1;
                        if x == b'\n' {
                            break;
                        }
                    }
                    state = 1;
                }
            }
            1 => {
                size = uu_dec(c);
                state = 2;
            }
            2 => {
                if e - p < 3 {
                    p = e;
                    continue;
                }
                let a = uu_dec(c);
                let b = uu_dec(s[p]);
                let cc = uu_dec(s[p + 1]);
                let d = uu_dec(s[p + 2]);
                p += 3;
                if size > 0 {
                    out.push(((a << 2) | (b >> 4)) & 0xFF);
                    size -= 1;
                }
                if size > 0 {
                    out.push(((b << 4) | (cc >> 2)) & 0xFF);
                    size -= 1;
                }
                if size > 0 {
                    out.push(((cc << 6) | d) & 0xFF);
                    size -= 1;
                }
                state = if size > 0 { 2 } else { 3 };
            }
            _ => state = 1,
        }
    }
    out
}

pub fn uu_encode(cps: &[u32]) -> Vec<u8> {
    if cps.is_empty() {
        return Vec::new();
    }
    let six = |b: u32| if b == 0 { b'`' } else { (b + 32) as u8 };
    let mut out = b"begin 0644 filename\n".to_vec();
    for line in cps.chunks(45) {
        out.push(line.len() as u8 + 32);
        for g in line.chunks(3) {
            let w = g[0] & 0xFF;
            let w2 = g.get(1).map_or(0, |x| x & 0xFF);
            let w3 = g.get(2).map_or(0, |x| x & 0xFF);
            out.push(six((w >> 2) & 0x3F));
            out.push(six(((w & 3) << 4) + ((w2 >> 4) & 0xF)));
            out.push(six(((w2 & 0xF) << 2) + ((w3 >> 6) & 3)));
            out.push(six(w3 & 0x3F));
        }
        out.push(b'\n');
    }
    out
}
