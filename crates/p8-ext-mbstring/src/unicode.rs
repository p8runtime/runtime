//! Case mapping and the Unicode properties mbstring uses, over php-src's own
//! tables (so the results are PHP's, not Rust's).
//!
//! Reference: $PHP_SRC_PATH/ext/mbstring/php_unicode.c

use crate::enc::BAD;
use crate::unidata::*;

pub const UC_CASED: usize = 35;
pub const UC_CASE_IGNORABLE: usize = 36;

fn prop_lookup(code: u32, n: usize) -> bool {
    let mut l = _UCPROP_OFFSETS[n] as i64;
    let mut r = _UCPROP_OFFSETS[n + 1] as i64 - 1;
    while l <= r {
        let mut m = (l + r) >> 1;
        m -= m & 1;
        if code > _UCPROP_RANGES[m as usize + 1] {
            l = m + 2;
        } else if code < _UCPROP_RANGES[m as usize] {
            r = m - 2;
        } else {
            return true;
        }
    }
    false
}

pub fn is_prop(code: u32, prop: usize) -> bool {
    prop_lookup(code, prop)
}

pub fn is_cased(c: u32) -> bool {
    is_prop(c, UC_CASED)
}

pub fn is_case_ignorable(c: u32) -> bool {
    is_prop(c, UC_CASE_IGNORABLE)
}

fn mph_hash(d: u32, x: u32) -> u32 {
    let x = x ^ d;
    ((x >> 16) ^ x).wrapping_mul(0x45d9f3b)
}

const NOT_FOUND: u32 = u32::MAX;

fn mph_lookup(code: u32, g: &[i16], g_size: usize, table: &[u32], table_size: usize) -> u32 {
    let gv = g[mph_hash(0, code) as usize % g_size];
    let idx = if gv <= 0 { (-(gv as i32)) as usize } else { mph_hash(gv as u32, code) as usize % table_size };
    if table[2 * idx] == code { table[2 * idx + 1] } else { NOT_FOUND }
}

/// Whether the encoding is ISO-8859-9 (Turkish i has its own mapping there).
#[derive(Clone, Copy)]
pub struct Turkish(pub bool);

fn upper_raw(code: u32, tr: Turkish) -> u32 {
    if code < 0xB5 {
        if (0x61..=0x7A).contains(&code) {
            if tr.0 && code == 0x69 {
                return 0x130;
            }
            return code - 0x20;
        }
        code
    } else {
        match mph_lookup(code, &_UCCASE_UPPER_G, _UCCASE_UPPER_G_SIZE, &_UCCASE_UPPER_TABLE, _UCCASE_UPPER_TABLE_SIZE) {
            NOT_FOUND => code,
            n => n,
        }
    }
}

fn lower_raw(code: u32, tr: Turkish) -> u32 {
    if code < 0xC0 {
        if (0x41..=0x5A).contains(&code) {
            if tr.0 && code == 0x49 {
                return 0x131;
            }
            return code + 0x20;
        }
        code
    } else {
        match mph_lookup(code, &_UCCASE_LOWER_G, _UCCASE_LOWER_G_SIZE, &_UCCASE_LOWER_TABLE, _UCCASE_LOWER_TABLE_SIZE) {
            NOT_FOUND => code,
            n => {
                if tr.0 && code == 0x130 {
                    0x69
                } else {
                    n
                }
            }
        }
    }
}

fn title_raw(code: u32, tr: Turkish) -> u32 {
    match mph_lookup(code, &_UCCASE_TITLE_G, _UCCASE_TITLE_G_SIZE, &_UCCASE_TITLE_TABLE, _UCCASE_TITLE_TABLE_SIZE) {
        NOT_FOUND => upper_raw(code, tr),
        n => n,
    }
}

fn fold_raw(code: u32, tr: Turkish) -> u32 {
    if code < 0x80 {
        if (0x41..=0x5A).contains(&code) {
            if tr.0 && code == 0x49 {
                return 0x131;
            }
            return code + 0x20;
        }
        code
    } else {
        match mph_lookup(code, &_UCCASE_FOLD_G, _UCCASE_FOLD_G_SIZE, &_UCCASE_FOLD_TABLE, _UCCASE_FOLD_TABLE_SIZE) {
            NOT_FOUND => code,
            n => {
                if tr.0 && code == 0x130 {
                    0x69
                } else {
                    n
                }
            }
        }
    }
}

/// A mapping that is a sequence (code > 0xFFFFFF) becomes its characters.
fn emit(w: u32, out: &mut Vec<u32>) {
    if w > 0xFF_FFFF {
        let len = (w >> 24) as usize;
        let at = (w & 0xFF_FFFF) as usize;
        for k in 0..len {
            out.push(_UCCASE_EXTRA_TABLE[at + 1 + k]);
        }
    } else {
        out.push(w);
    }
}

fn simple(w: u32) -> u32 {
    if w > 0xFF_FFFF { _UCCASE_EXTRA_TABLE[(w & 0xFF_FFFF) as usize] } else { w }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Upper = 0,
    Lower,
    Title,
    Fold,
    UpperSimple,
    LowerSimple,
    TitleSimple,
    FoldSimple,
}

impl Mode {
    pub fn from_int(i: i64) -> Option<Mode> {
        Some(match i {
            0 => Mode::Upper,
            1 => Mode::Lower,
            2 => Mode::Title,
            3 => Mode::Fold,
            4 => Mode::UpperSimple,
            5 => Mode::LowerSimple,
            6 => Mode::TitleSimple,
            7 => Mode::FoldSimple,
            _ => return None,
        })
    }
}
/// Whether a final sigma applies at chunk index `i`: preceded by a cased
/// letter (through case-ignorable ones), and not followed by one. Like
/// php-src, the look-back past the chunk's start only sees the previous
/// chunk's output that this chunk has not written over yet.
fn final_sigma(chunk: &[u32], i: usize, rest: &[u32], prev_out: &[u32], written: usize) -> bool {
    let mut j = i as isize - 1;
    while j >= 0 && is_case_ignorable(chunk[j as usize]) {
        j -= 1;
    }
    let preceded = if j >= 0 {
        is_cased(chunk[j as usize])
    } else {
        let mut cased = false;
        let mut k = prev_out.len();
        while k > written {
            k -= 1;
            if is_cased(prev_out[k]) {
                cased = true;
                break;
            }
            if !is_case_ignorable(prev_out[k]) {
                break;
            }
        }
        cased
    };
    if !preceded {
        return false;
    }
    let mut j = i + 1;
    while j < chunk.len() && is_case_ignorable(chunk[j]) {
        j += 1;
    }
    if j < chunk.len() {
        return !is_cased(chunk[j]);
    }
    for &w in rest {
        if is_cased(w) {
            return false;
        }
        if !is_case_ignorable(w) {
            return true;
        }
    }
    true
}

/// php-src converts 64 characters at a time.
const CHUNK: usize = 64;

pub fn convert_case(mode: Mode, src: &[u32], tr: Turkish) -> Vec<u32> {
    let mut out = Vec::with_capacity(src.len());
    let mut title_mode = false;
    let mut prev_out: Vec<u32> = Vec::new();
    let mut start = 0;
    while start < src.len() {
        let end = (start + CHUNK).min(src.len());
        let chunk = &src[start..end];
        let rest = &src[end..];
        let mut cur: Vec<u32> = Vec::with_capacity(chunk.len());
        for (i, &w) in chunk.iter().enumerate() {
            if w > 0xFF_FFFF || w == BAD {
                cur.push(w);
                continue;
            }
            match mode {
                Mode::UpperSimple => cur.push(simple(upper_raw(w, tr))),
                Mode::LowerSimple => cur.push(simple(lower_raw(w, tr))),
                Mode::FoldSimple => cur.push(simple(fold_raw(w, tr))),
                Mode::TitleSimple => {
                    cur.push(if title_mode { simple(lower_raw(w, tr)) } else { simple(title_raw(w, tr)) });
                    if !is_case_ignorable(w) {
                        title_mode = is_cased(w);
                    }
                }
                Mode::Upper => emit(upper_raw(w, tr), &mut cur),
                Mode::Lower => {
                    if w == 0x3A3 && final_sigma(chunk, i, rest, &prev_out, cur.len()) {
                        cur.push(0x3C2);
                    } else {
                        emit(lower_raw(w, tr), &mut cur);
                    }
                }
                Mode::Fold => emit(fold_raw(w, tr), &mut cur),
                Mode::Title => {
                    if title_mode {
                        if w == 0x3A3 && final_sigma(chunk, i, rest, &prev_out, cur.len()) {
                            cur.push(0x3C2);
                        } else {
                            emit(lower_raw(w, tr), &mut cur);
                        }
                    } else {
                        emit(title_raw(w, tr), &mut cur);
                    }
                    if !is_case_ignorable(w) {
                        title_mode = is_cased(w);
                    }
                }
            }
        }
        out.extend_from_slice(&cur);
        prev_out = cur;
        start = end;
    }
    out
}
