//! A small interpreter for the magic(5) rule language, enough for the rules in
//! `rules/` (an excerpt of file(1) 5.46's Magdir, the database PHP 8.4 ships).
//! It follows libmagic's softmagic.c: the same offsets (absolute, relative,
//! indirect), the same type and string/search/regex tests, the same
//! continuation levels, `name`/`use`/`indirect`/`default`/`clear`, strengths
//! (rules are tried strongest first) and the same output rules for descriptions
//! and MIME types.

use p8_pcre::{MatchOptions, Options, Regex};
use std::collections::HashMap;
use std::rc::Rc;

pub const F_MIME_TYPE: i64 = 0x10;
pub const F_MIME_ENC: i64 = 0x400;
pub const F_CONTINUE: i64 = 0x20;
pub const F_APPLE: i64 = 0x800;
pub const F_EXTENSION: i64 = 0x100_0000;
/// What the caller knows about the input besides its bytes.
#[derive(Clone, Copy, Default)]
pub struct Env {
    /// st_mode: `${x?...}` in a description tests the execute bits
    pub mode: u32,
    /// the whole file can be read back, so offsets from its end work
    pub tail: bool,
}

pub const F_NODESC: i64 = F_MIME_TYPE | F_MIME_ENC | F_APPLE | F_EXTENSION;

const MAXSTRING: usize = 128;
const INDIR_MAX: u16 = 50;
const NAME_MAX: u16 = 50;
const REGEX_MAX: usize = 8192;
const MATCH_LIMIT: u64 = 500_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ty {
    Byte,
    Short,
    Long,
    Quad,
    BeShort,
    LeShort,
    BeLong,
    LeLong,
    BeQuad,
    LeQuad,
    /// 32-bit time_t values (date, bedate, ledate; the local-time variants too: the zone is UTC)
    Date,
    BeDate,
    LeDate,
    /// 64-bit time_t values (qdate...)
    QDate,
    BeQDate,
    LeQDate,
    /// 64-bit Windows FILETIMEs (qwdate...)
    WDate,
    BeWDate,
    LeWDate,
    /// MS-DOS date and time words
    DosDate,
    BeDosDate,
    DosTime,
    BeDosTime,
    Str,
    BeStr16,
    LeStr16,
    PStr,
    Search,
    Regex,
    Default,
    Clear,
    Name,
    Use,
    Indirect,
}

impl Ty {
    fn size(self) -> usize {
        match self {
            Ty::Byte => 1,
            Ty::Short | Ty::BeShort | Ty::LeShort => 2,
            Ty::DosDate | Ty::BeDosDate | Ty::DosTime | Ty::BeDosTime => 2,
            Ty::Long | Ty::BeLong | Ty::LeLong | Ty::Date | Ty::BeDate | Ty::LeDate => 4,
            Ty::Quad | Ty::BeQuad | Ty::LeQuad | Ty::QDate | Ty::BeQDate | Ty::LeQDate | Ty::WDate | Ty::BeWDate | Ty::LeWDate => 8,
            _ => 0,
        }
    }
    fn is_numeric(self) -> bool {
        self.size() != 0
    }
    fn is_date(self) -> bool {
        matches!(self, Ty::Date | Ty::BeDate | Ty::LeDate | Ty::QDate | Ty::BeQDate | Ty::LeQDate | Ty::WDate | Ty::BeWDate | Ty::LeWDate)
    }
    fn is_string(self) -> bool {
        matches!(self, Ty::Str | Ty::BeStr16 | Ty::LeStr16 | Ty::PStr | Ty::Search | Ty::Regex | Ty::Name | Ty::Use | Ty::Indirect)
    }
    fn flip(self) -> Ty {
        match self {
            Ty::BeShort => Ty::LeShort,
            Ty::LeShort => Ty::BeShort,
            Ty::BeLong => Ty::LeLong,
            Ty::LeLong => Ty::BeLong,
            Ty::BeQuad => Ty::LeQuad,
            Ty::LeQuad => Ty::BeQuad,
            Ty::BeDate => Ty::LeDate,
            Ty::LeDate => Ty::BeDate,
            Ty::BeDosDate => Ty::DosDate,
            Ty::DosDate => Ty::BeDosDate,
            Ty::BeDosTime => Ty::DosTime,
            Ty::DosTime => Ty::BeDosTime,
            Ty::BeQDate => Ty::LeQDate,
            Ty::LeQDate => Ty::BeQDate,
            Ty::BeWDate => Ty::LeWDate,
            Ty::LeWDate => Ty::BeWDate,
            t => t,
        }
    }
}

const SF_COMPACT_WS: u32 = 1;
const SF_OPT_WS: u32 = 2;
const SF_LOWER: u32 = 4;
const SF_UPPER: u32 = 8;
const SF_BIN: u32 = 0x10;
const SF_TEXT: u32 = 0x20;
const SF_TRIM: u32 = 0x40;
const SF_FULL_WORD: u32 = 0x80;
const SF_REGEX_START: u32 = 0x100;
const SF_REGEX_LINES: u32 = 0x200;
const SF_INDIRECT_REL: u32 = 0x400;
const SF_PSTR_2BE: u32 = 0x800;
const SF_PSTR_2LE: u32 = 0x1000;
const SF_PSTR_4BE: u32 = 0x2000;
const SF_PSTR_4LE: u32 = 0x4000;
const SF_PSTR_SELF: u32 = 0x8000;
const SF_IGNORE_CASE: u32 = SF_LOWER | SF_UPPER;

const OP_AND: u8 = 1;
const OP_OR: u8 = 2;
const OP_XOR: u8 = 3;
const OP_ADD: u8 = 4;
const OP_SUB: u8 = 5;
const OP_MUL: u8 = 6;
const OP_DIV: u8 = 7;
const OP_MOD: u8 = 8;

#[derive(Clone, Debug)]
struct Indir {
    ty: Ty,
    signed: bool,
    op: u8,
    inverse: bool,
    via_pointer: bool,
    value: i64,
    /// `.I` and `.i`: an ID3 "syncsafe" length
    id3: bool,
}

#[derive(Clone)]
struct Entry {
    level: usize,
    offset: i64,
    negative: bool,
    /// `&N` or `(&N...)`: relative to the previous level's offset.
    rel: bool,
    indir: Option<Indir>,
    indir_rel: bool,
    ty: Ty,
    unsigned: bool,
    mask_op: u8,
    mask_inverse: bool,
    mask: u64,
    reln: u8,
    num: u64,
    s: Vec<u8>,
    flags: u32,
    range: usize,
    regex: Option<Rc<Regex>>,
    desc: Vec<u8>,
    nospace: bool,
    mime: String,
    ext: String,
    apple: String,
    factor: Option<(u8, i64)>,
    /// set_test_type: which pass (BINTEST, TEXTTEST) may use a top-level entry.
    testflag: u8,
}

impl Entry {
    fn new() -> Entry {
        Entry {
            level: 0,
            offset: 0,
            negative: false,
            rel: false,
            indir: None,
            indir_rel: false,
            ty: Ty::Byte,
            unsigned: false,
            mask_op: 0,
            mask_inverse: false,
            mask: 0,
            reln: b'=',
            num: 0,
            s: Vec::new(),
            flags: 0,
            range: 0,
            regex: None,
            desc: Vec::new(),
            nospace: false,
            mime: String::new(),
            ext: String::new(),
            apple: String::new(),
            factor: None,
            testflag: 0,
        }
    }
}

pub struct Rules {
    /// Top-level entries and their continuations, strongest rule first.
    flat: Vec<Entry>,
    names: HashMap<String, Vec<Entry>>,
}

// ---- parsing ----

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

fn strtoull(s: &[u8]) -> Option<(u64, usize)> {
    let mut i = 0;
    while s.get(i).is_some_and(|&c| is_space(c)) {
        i += 1;
    }
    let mut neg = false;
    if s.get(i) == Some(&b'-') {
        neg = true;
        i += 1;
    } else if s.get(i) == Some(&b'+') {
        i += 1;
    }
    let (radix, start) = if s[i..].starts_with(b"0x") || s[i..].starts_with(b"0X") {
        (16, i + 2)
    } else if s.get(i) == Some(&b'0') && s.get(i + 1).is_some_and(|c| c.is_ascii_digit()) {
        (8, i + 1)
    } else {
        (10, i)
    };
    let mut j = start;
    let mut v: u64 = 0;
    while j < s.len() {
        let Some(d) = (s[j] as char).to_digit(radix) else { break };
        v = v.wrapping_mul(u64::from(radix)).wrapping_add(u64::from(d));
        j += 1;
    }
    if j == start && !(radix == 8 && start == i + 1) {
        if radix == 16 {
            return Some((0, i + 1));
        }
        return None;
    }
    Some((if neg { v.wrapping_neg() } else { v }, j))
}

/// libmagic's getstr: C escapes, stops at unescaped whitespace.
fn getstr(s: &[u8]) -> (Vec<u8>, usize) {
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        i += 1;
        if is_space(c) {
            i -= 1;
            break;
        }
        if c != b'\\' {
            out.push(c);
            continue;
        }
        let Some(&e) = s.get(i) else { break };
        i += 1;
        match e {
            b'a' => out.push(7),
            b'b' => out.push(8),
            b'f' => out.push(12),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'v' => out.push(11),
            b'0'..=b'7' => {
                let mut v = u32::from(e - b'0');
                for _ in 0..2 {
                    match s.get(i) {
                        Some(&d) if (b'0'..=b'7').contains(&d) => {
                            v = v * 8 + u32::from(d - b'0');
                            i += 1;
                        }
                        _ => break,
                    }
                }
                out.push(v as u8);
            }
            b'x' => {
                let mut v = 0u32;
                let mut n = 0;
                while n < 2 {
                    match s.get(i).and_then(|&d| (d as char).to_digit(16)) {
                        Some(d) => {
                            v = v * 16 + d;
                            i += 1;
                            n += 1;
                        }
                        None => break,
                    }
                }
                out.push(v as u8);
            }
            other => out.push(other),
        }
    }
    (out, i)
}

fn type_names() -> &'static [(&'static str, Ty, bool)] {
    // (name, type, unsigned-implied-by-name)
    &[
        ("byte", Ty::Byte, false),
        ("short", Ty::Short, false),
        ("long", Ty::Long, false),
        ("quad", Ty::Quad, false),
        ("beshort", Ty::BeShort, false),
        ("leshort", Ty::LeShort, false),
        ("belong", Ty::BeLong, false),
        ("lelong", Ty::LeLong, false),
        ("bequad", Ty::BeQuad, false),
        ("lequad", Ty::LeQuad, false),
        ("msdosdate", Ty::DosDate, false),
        ("lemsdosdate", Ty::DosDate, false),
        ("bemsdosdate", Ty::BeDosDate, false),
        ("msdostime", Ty::DosTime, false),
        ("lemsdostime", Ty::DosTime, false),
        ("bemsdostime", Ty::BeDosTime, false),
        ("date", Ty::Date, false),
        ("bedate", Ty::BeDate, false),
        ("ledate", Ty::LeDate, false),
        ("ldate", Ty::Date, false),
        ("beldate", Ty::BeDate, false),
        ("leldate", Ty::LeDate, false),
        ("qdate", Ty::QDate, false),
        ("beqdate", Ty::BeQDate, false),
        ("leqdate", Ty::LeQDate, false),
        ("qldate", Ty::QDate, false),
        ("beqldate", Ty::BeQDate, false),
        ("leqldate", Ty::LeQDate, false),
        ("qwdate", Ty::WDate, false),
        ("beqwdate", Ty::BeWDate, false),
        ("leqwdate", Ty::LeWDate, false),
        ("pstring", Ty::PStr, false),
        ("string", Ty::Str, false),
        ("bestring16", Ty::BeStr16, false),
        ("lestring16", Ty::LeStr16, false),
        ("search", Ty::Search, false),
        ("regex", Ty::Regex, false),
        ("default", Ty::Default, false),
        ("clear", Ty::Clear, false),
        ("name", Ty::Name, false),
        ("use", Ty::Use, false),
        ("indirect", Ty::Indirect, false),
    ]
}

/// Longest type keyword at the start of `l`.
fn get_type(l: &[u8]) -> Option<(Ty, usize)> {
    let mut best: Option<(Ty, usize)> = None;
    for (name, ty, _) in type_names() {
        let n = name.len();
        if l.starts_with(name.as_bytes()) && l.get(n).is_none_or(|c| !c.is_ascii_alphanumeric()) && best.is_none_or(|(_, bl)| n > bl) {
            best = Some((*ty, n));
        }
    }
    best
}

fn get_op(c: u8) -> Option<u8> {
    Some(match c {
        b'&' => OP_AND,
        b'|' => OP_OR,
        b'^' => OP_XOR,
        b'+' => OP_ADD,
        b'-' => OP_SUB,
        b'*' => OP_MUL,
        b'/' => OP_DIV,
        b'%' => OP_MOD,
        _ => return None,
    })
}

fn parse_entry(line: &[u8], lineno: usize) -> Result<Entry, String> {
    let bytes = line;
    let mut e = Entry::new();
    let mut l = 0;
    while bytes.get(l) == Some(&b'>') {
        l += 1;
        e.level += 1;
    }
    let err = |m: &str| format!("line {lineno}: {m}: {}", String::from_utf8_lossy(line));
    if bytes.get(l) == Some(&b'&') {
        l += 1;
        e.rel = true;
    }
    let mut indirect = false;
    if bytes.get(l) == Some(&b'(') {
        l += 1;
        indirect = true;
        if e.rel {
            e.indir_rel = true;
            e.rel = false;
        }
        if bytes.get(l) == Some(&b'&') {
            l += 1;
            e.rel = true;
        }
    }
    if bytes.get(l) == Some(&b'-') {
        l += 1;
        e.negative = true;
    }
    let (v, n) = strtoull(&bytes[l..]).ok_or_else(|| err("bad offset"))?;
    e.offset = v as i64;
    l += n;
    if indirect {
        let mut ind = Indir { ty: Ty::Long, signed: false, op: 0, inverse: false, via_pointer: false, value: 0, id3: false };
        // The default `(off)` type is a native long (little-endian here).
        if matches!(bytes.get(l), Some(b'.') | Some(b',')) {
            if bytes[l] == b',' {
                ind.signed = true;
            }
            l += 1;
            ind.ty = match bytes.get(l) {
                Some(b'l') => Ty::LeLong,
                Some(b'L') => Ty::BeLong,
                Some(b'h') | Some(b's') => Ty::LeShort,
                Some(b'H') | Some(b'S') => Ty::BeShort,
                Some(b'c') | Some(b'b') | Some(b'C') | Some(b'B') => Ty::Byte,
                Some(b'q') => Ty::LeQuad,
                Some(b'Q') => Ty::BeQuad,
                Some(b'I') => {
                    ind.id3 = true;
                    Ty::BeLong
                }
                Some(b'i') => {
                    ind.id3 = true;
                    Ty::LeLong
                }
                _ => return Err(err("bad indirect type")),
            };
            l += 1;
        }
        if bytes.get(l) == Some(&b'~') {
            ind.inverse = true;
            l += 1;
        }
        if let Some(op) = bytes.get(l).and_then(|&c| get_op(c)) {
            ind.op = op;
            l += 1;
        }
        if bytes.get(l) == Some(&b'(') {
            ind.via_pointer = true;
            l += 1;
        }
        if bytes.get(l).is_some_and(|c| c.is_ascii_digit() || *c == b'-')
            && let Some((v, n)) = strtoull(&bytes[l..])
        {
            ind.value = v as i64;
            l += n;
        }
        if bytes.get(l) != Some(&b')') {
            return Err(err("missing )"));
        }
        l += 1;
        if ind.via_pointer {
            if bytes.get(l) != Some(&b')') {
                return Err(err("missing )"));
            }
            l += 1;
        }
        e.indir = Some(ind);
    }
    while bytes.get(l).is_some_and(|&c| c == b' ' || c == b'\t') {
        l += 1;
    }
    // type
    if bytes.get(l) == Some(&b'u') && get_type(&bytes[l + 1..]).is_some() {
        e.unsigned = true;
        l += 1;
    }
    let (ty, n) = get_type(&bytes[l..]).ok_or_else(|| err("unknown type"))?;
    e.ty = ty;
    l += n;
    if bytes.get(l) == Some(&b'~') && !ty.is_string() {
        e.mask_inverse = true;
        l += 1;
    }
    if let Some(op) = bytes.get(l).and_then(|&c| get_op(c)) {
        if ty.is_string() {
            if op != OP_DIV {
                return Err(err("bad string op"));
            }
            // /flags
            l += 1;
            while l < bytes.len() && !is_space(bytes[l]) {
                match bytes[l] {
                    b'0'..=b'9' => {
                        let (v, n) = strtoull(&bytes[l..]).ok_or_else(|| err("bad range"))?;
                        e.range = v as usize;
                        l += n;
                        continue;
                    }
                    b'/' => {}
                    b'W' => e.flags |= SF_COMPACT_WS,
                    b'w' => e.flags |= SF_OPT_WS,
                    b'c' => e.flags |= if ty == Ty::Indirect { 0 } else { SF_LOWER },
                    b'C' => e.flags |= SF_UPPER,
                    b's' => e.flags |= SF_REGEX_START,
                    b'l' if ty == Ty::PStr => e.flags |= SF_PSTR_4LE,
                    b'l' => e.flags |= SF_REGEX_LINES,
                    b'b' => e.flags |= SF_BIN,
                    b't' => e.flags |= SF_TEXT,
                    b'T' => e.flags |= SF_TRIM,
                    b'f' => e.flags |= SF_FULL_WORD,
                    b'r' if ty == Ty::Indirect => e.flags |= SF_INDIRECT_REL,
                    b'B' if ty == Ty::PStr => {}
                    b'H' if ty == Ty::PStr => e.flags |= SF_PSTR_2BE,
                    b'h' if ty == Ty::PStr => e.flags |= SF_PSTR_2LE,
                    b'L' if ty == Ty::PStr => e.flags |= SF_PSTR_4BE,
                    b'J' if ty == Ty::PStr => e.flags |= SF_PSTR_SELF,
                    c => return Err(err(&format!("bad string modifier {}", c as char))),
                }
                l += 1;
            }
        } else {
            e.mask_op = op;
            l += 1;
            let (v, n) = strtoull(&bytes[l..]).ok_or_else(|| err("bad mask"))?;
            e.mask = v;
            l += n;
        }
    }
    while bytes.get(l).is_some_and(|&c| c == b' ' || c == b'\t') {
        l += 1;
    }
    match bytes.get(l) {
        Some(&c @ (b'>' | b'<')) => {
            e.reln = c;
            l += 1;
            if bytes.get(l) == Some(&b'=') {
                l += 1;
            }
        }
        Some(&c @ (b'&' | b'^' | b'=')) => {
            e.reln = c;
            l += 1;
            if bytes.get(l) == Some(&b'=') {
                l += 1;
            }
        }
        Some(b'!') => {
            e.reln = b'!';
            l += 1;
        }
        _ => {
            e.reln = b'=';
            if bytes.get(l) == Some(&b'x') && bytes.get(l + 1).is_none_or(|&c| is_space(c)) {
                e.reln = b'x';
                l += 1;
            }
        }
    }
    if e.reln != b'x' {
        if ty.is_string() {
            let (s, n) = getstr(&bytes[l..]);
            e.s = s;
            l += n;
            if ty == Ty::Regex {
                let opts = Options { multiline: true, caseless: e.flags & SF_IGNORE_CASE != 0, ..Options::default() };
                match Regex::new(&e.s, &opts) {
                    Ok(r) => e.regex = Some(Rc::new(r)),
                    Err(er) => return Err(err(&format!("regex: {er}"))),
                }
            }
        } else if ty != Ty::Default && ty != Ty::Clear {
            let (v, n) = strtoull(&bytes[l..]).ok_or_else(|| err("bad number"))?;
            e.num = sign_extend(&e, v);
            l += n;
            // size suffixes (l, L, u, ...) are ignored
            while bytes.get(l).is_some_and(|c| matches!(c, b'l' | b'L' | b'u' | b'U')) {
                l += 1;
            }
        }
    }
    while bytes.get(l).is_some_and(|&c| c == b' ' || c == b'\t') {
        l += 1;
    }
    if bytes.get(l) == Some(&8) {
        l += 1;
        e.nospace = true;
    } else if bytes.get(l) == Some(&b'\\') && bytes.get(l + 1) == Some(&b'b') {
        l += 2;
        e.nospace = true;
    }
    e.desc = bytes[l..].to_vec();
    Ok(e)
}

fn sign_extend(e: &Entry, v: u64) -> u64 {
    if e.unsigned {
        return mask_to(e.ty, v);
    }
    match e.ty.size() {
        1 => v as u8 as i8 as i64 as u64,
        2 => v as u16 as i16 as i64 as u64,
        4 => v as u32 as i32 as i64 as u64,
        _ => v,
    }
}

fn mask_to(ty: Ty, v: u64) -> u64 {
    match ty.size() {
        1 => v & 0xff,
        2 => v & 0xffff,
        4 => v & 0xffff_ffff,
        _ => v,
    }
}

fn base_strength(e: &Entry) -> i64 {
    const MULT: i64 = 10;
    let mut val = 2 * MULT;
    match e.ty {
        Ty::Default => return 0,
        t if t.is_numeric() => val += e.ty.size() as i64 * MULT,
        Ty::PStr | Ty::Str => val += e.s.len() as i64 * MULT,
        Ty::BeStr16 | Ty::LeStr16 => val += e.s.len() as i64 * MULT / 2,
        Ty::Search => {
            let n = e.s.len() as i64;
            if n != 0 {
                val += n * (MULT / n).max(1);
            }
        }
        Ty::Regex => {
            let v = nonmagic(&e.s) as i64;
            val += v * (MULT / v).max(1);
        }
        _ => {}
    }
    match e.reln {
        b'x' | b'!' => val = 0,
        b'=' => val += MULT,
        b'>' | b'<' => val -= 2 * MULT,
        b'^' | b'&' => val -= MULT,
        _ => {}
    }
    val
}

/// libmagic's nonmagic: what a regex counts for in its strength (literal characters).
fn nonmagic(s: &[u8]) -> usize {
    let mut rv = 0usize;
    let at = |i: isize| -> u8 { if i >= 0 && (i as usize) < s.len() { s[i as usize] } else { 0 } };
    let mut i: isize = 0;
    while at(i) != 0 {
        match at(i) {
            b'\\' => {
                i += 1;
                if at(i) == 0 {
                    i -= 1;
                }
                rv += 1;
            }
            b'?' | b'*' | b'.' | b'+' | b'^' | b'$' => {}
            b'[' => {
                while at(i) != 0 && at(i) != b']' {
                    i += 1;
                }
                i -= 1;
            }
            b'{' => {
                while at(i) != 0 && at(i) != b'}' {
                    i += 1;
                }
                if at(i) == 0 {
                    i -= 1;
                }
            }
            _ => rv += 1,
        }
        i += 1;
    }
    if rv == 0 { 1 } else { rv }
}

fn strength(e: &Entry) -> i64 {
    let mut val = base_strength(e);
    if let Some((op, f)) = e.factor {
        match op {
            b'+' => val += f,
            b'-' => val -= f,
            b'*' => val *= f,
            b'/' => val /= f.max(1),
            _ => {}
        }
    }
    if val <= 0 && e.ty != Ty::Default {
        val = 1;
    }
    if e.desc.is_empty() {
        val += 1;
    }
    val
}

impl Rules {
    /// Parse rule files. A rule using something this interpreter does not
    /// implement (dates, floats, ID3 lengths...) is left out, with its
    /// continuations; the count of such entries is returned too.
    pub fn parse(sources: &[&[u8]]) -> Result<(Rules, Vec<String>), String> {
        let mut blocks: Vec<Vec<Entry>> = Vec::new();
        let mut names: HashMap<String, Vec<Entry>> = HashMap::new();
        let mut cur: Vec<Entry> = Vec::new();
        let mut lineno = 0;
        let mut skipped: Vec<String> = Vec::new();
        let mut skip_level: Option<usize> = None;
        let mut last_ok = false;
        let flush = |cur: &mut Vec<Entry>, blocks: &mut Vec<Vec<Entry>>, names: &mut HashMap<String, Vec<Entry>>| {
            if cur.is_empty() {
                return;
            }
            let mut block = std::mem::take(cur);
            set_test_type(&mut block);
            if block[0].ty == Ty::Name {
                let n = String::from_utf8_lossy(&block[0].s).into_owned();
                names.insert(n, block);
            } else {
                blocks.push(block);
            }
        };
        for src in sources {
            for line in src.split(|&c| c == b'\n') {
                lineno += 1;
                if line.is_empty() || line[0] == b'#' {
                    continue;
                }
                if let Some(rest) = line.strip_prefix(b"!:") {
                    if !last_ok {
                        continue;
                    }
                    let rest = String::from_utf8_lossy(rest).into_owned();
                    let mut it = rest.splitn(2, |c: char| c.is_whitespace());
                    let key = it.next().unwrap_or("");
                    let val = it.next().unwrap_or("").trim();
                    match key {
                        "mime" => {
                            if let Some(last) = cur.last_mut() {
                                last.mime = val.to_string();
                            }
                        }
                        "ext" => {
                            if let Some(last) = cur.last_mut() {
                                last.ext = val.to_string();
                            }
                        }
                        "apple" => {
                            if let Some(last) = cur.last_mut() {
                                last.apple = val.to_string();
                            }
                        }
                        "strength" => {
                            let compact: String = val.split_whitespace().collect();
                            let op = compact.as_bytes().first().copied().unwrap_or(b'+');
                            if let (Some(n), Some(first)) = (compact.get(1..).and_then(|n| n.parse::<i64>().ok()), cur.first_mut()) {
                                first.factor = Some((op, n));
                            }
                        }
                        _ => {}
                    }
                    continue;
                }
                let level = line.iter().take_while(|&&c| c == b'>').count();
                if let Some(sl) = skip_level {
                    if level > sl {
                        last_ok = false;
                        continue;
                    }
                    skip_level = None;
                }
                match parse_entry(line, lineno) {
                    Ok(e) => {
                        if e.level == 0 {
                            flush(&mut cur, &mut blocks, &mut names);
                        }
                        cur.push(e);
                        last_ok = true;
                    }
                    Err(msg) => {
                        skipped.push(msg);
                        skip_level = Some(level);
                        last_ok = false;
                        if level == 0 {
                            flush(&mut cur, &mut blocks, &mut names);
                        }
                    }
                }
            }
            flush(&mut cur, &mut blocks, &mut names);
            skip_level = None;
            last_ok = false;
        }
        // A `use` of a name that is not there is left out, with its continuations.
        let known: std::collections::HashSet<String> = names.keys().cloned().collect();
        let strip = |block: Vec<Entry>, skipped: &mut Vec<String>| -> Vec<Entry> {
            let mut out: Vec<Entry> = Vec::with_capacity(block.len());
            let mut skip_below: Option<usize> = None;
            for e in block {
                if let Some(l) = skip_below {
                    if e.level > l {
                        continue;
                    }
                    skip_below = None;
                }
                if e.ty == Ty::Use {
                    let n = String::from_utf8_lossy(&e.s).into_owned();
                    if !known.contains(n.strip_prefix('^').unwrap_or(&n)) {
                        skipped.push(format!("use of missing name {n}"));
                        skip_below = Some(e.level);
                        continue;
                    }
                }
                out.push(e);
            }
            out
        };
        let blocks: Vec<Vec<Entry>> = blocks.into_iter().map(|b| strip(b, &mut skipped)).filter(|b| !b.is_empty() && b[0].level == 0).collect();
        let names: HashMap<String, Vec<Entry>> = names.into_iter().map(|(k, b)| (k, strip(b, &mut skipped))).collect();
        // Strongest first, equal strengths in file order (qsort of glibc is stable).
        let mut keyed: Vec<(i64, Vec<Entry>)> = blocks.into_iter().map(|b| (strength(&b[0]), b)).collect();
        keyed.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
        let flat: Vec<Entry> = keyed.into_iter().flat_map(|(_, b)| b).collect();
        Ok((Rules { flat, names }, skipped))
    }
}

// ---- matching ----

struct St {
    printed_something: bool,
    need_separator: bool,
    firstline: bool,
    returnval: i32,
    found_match: bool,
    indir_count: u16,
    name_count: u16,
}

impl St {
    fn new() -> St {
        St { printed_something: false, need_separator: false, firstline: true, returnval: 0, found_match: false, indir_count: 0, name_count: 0 }
    }
}

#[derive(Clone, Default)]
struct Value {
    q: u64,
    s: Vec<u8>,
}

#[derive(Default)]
struct Search {
    /// start of the search window in the buffer
    start: usize,
    len: usize,
    offset: usize,
    rm_len: usize,
    /// the text a regex matched
    text: Vec<u8>,
}

struct Ctx<'a> {
    rules: &'a Rules,
    flags: i64,
    out: Vec<u8>,
    ms_offset: i64,
    value: Value,
    search: Search,
    st: St,
    env: Env,
    /// a regex ran out of steps: libmagic gives up on the whole rule list
    failed: bool,
}

pub enum Failure {
    /// libmagic reported an error (the finfo call returns false).
    Error(String),
}

type MR<T> = Result<T, Failure>;

/// Run the strongest-first rule list over `buf`. `text` is the looks-text
/// flag, `bintest` selects the binary (true) or the text pass.
/// Returns the output libmagic would have printed (empty when nothing matched).
pub fn softmagic(rules: &Rules, buf: &[u8], bintest: bool, text: bool, flags: i64, out: &mut Vec<u8>, env: Env) -> MR<bool> {
    let mut cx = Ctx { rules, flags, out: std::mem::take(out), ms_offset: 0, value: Value::default(), search: Search::default(), st: St::new(), env, failed: false };
    let rv = do_match(&mut cx, &rules.flat, buf, 0, bintest, text, false);
    *out = cx.out;
    Ok(rv? > 0)
}

fn do_match(cx: &mut Ctx, magic: &[Entry], b: &[u8], base: usize, bintest: bool, text: bool, flip: bool) -> MR<i32> {
    let print = cx.flags & F_NODESC == 0;
    let mut li: Vec<(i64, bool)> = vec![(0, false); 8];
    let mut idx = 0;
    while idx < magic.len() {
        let m = &magic[idx];
        if m.level != 0 {
            idx += 1;
            continue;
        }
        // test-type skipping
        let skip = {
            let libmagic_string = matches!(m.ty, Ty::Str | Ty::BeStr16 | Ty::LeStr16 | Ty::PStr | Ty::Search | Ty::Regex | Ty::Indirect | Ty::Name | Ty::Use);
            let mode = if bintest { TEST_BIN } else { TEST_TEXT };
            (libmagic_string && ((text && m.flags & (SF_BIN | SF_TEXT) == SF_BIN) || (!text && m.flags & (SF_BIN | SF_TEXT) == SF_TEXT))) || m.testflag & mode != mode
        };
        if m.ty != Ty::Name && skip {
            idx = skip_block(magic, idx);
            continue;
        }
        if m.negative && !cx.env.tail {
            idx = skip_block(magic, idx);
            continue;
        }
        cx.ms_offset = if m.negative { b.len() as i64 - m.offset } else { m.offset };
        let mut flush = false;
        match mget(cx, m, b, base, 0, &li, bintest, text, flip)? {
            0 => flush = m.reln != b'!',
            _ => {
                if m.ty == Ty::Indirect {
                    cx.st.found_match = true;
                    cx.st.returnval = 1;
                }
                if !magiccheck(cx, m, b) {
                    flush = true;
                }
                if cx.failed {
                    return Err(Failure::Error("(null)".into()));
                }
            }
        }
        if flush {
            idx = skip_block(magic, idx);
            continue;
        }
        if let Some(r) = handle_annotation(cx, m) {
            cx.st.found_match = true;
            cx.st.need_separator = true;
            cx.st.printed_something = true;
            cx.st.returnval = 1;
            cx.st.firstline = false;
            return Ok(r);
        }
        if !m.desc.is_empty() {
            cx.st.found_match = true;
            if print {
                cx.st.returnval = 1;
                cx.st.need_separator = true;
                cx.st.printed_something = true;
                print_sep(cx);
                mprint(cx, m);
            }
        }
        if !moffset(cx, m, b, &mut li[0].0) {
            // goto flush: the rest of the rule is dropped, and the search goes on
            idx = skip_block(magic, idx);
            continue;
        }
        let mut cont_level = 1usize;
        while li.len() <= cont_level + 1 {
            li.push((0, false));
        }
        li[cont_level].1 = false;
        // continuations
        let mut aborted = false;
        {
            while idx + 1 < magic.len() && magic[idx + 1].level != 0 {
                idx += 1;
                let c = &magic[idx];
                if cont_level < c.level {
                    continue;
                }
                if cont_level > c.level {
                    cont_level = c.level;
                }
                // msetoffset
                cx.ms_offset = c.offset;
                if c.negative {
                    cx.ms_offset = if c.rel || c.indir_rel { -c.offset } else { b.len() as i64 - c.offset };
                }
                if c.rel && cont_level > 0 {
                    cx.ms_offset += li[cont_level - 1].0;
                }
                if c.negative && !(c.rel || c.indir_rel) && (!cx.env.tail || c.offset > b.len() as i64) {
                    // msetoffset failed: goto flush
                    aborted = true;
                    break;
                }
                let mut fl = false;
                match mget(cx, c, b, base, cont_level, &li, bintest, text, flip)? {
                    0 => {
                        if c.reln != b'!' {
                            continue;
                        }
                        fl = true;
                    }
                    _ => {
                        if c.ty == Ty::Indirect {
                            cx.st.found_match = true;
                            cx.st.returnval = 1;
                        }
                    }
                }
                let matched = if fl { true } else { magiccheck(cx, c, b) };
                if cx.failed {
                    return Err(Failure::Error("(null)".into()));
                }
                if !matched {
                    continue;
                }
                if c.ty == Ty::Clear {
                    li[cont_level].1 = false;
                } else if li[cont_level].1 {
                    if c.ty == Ty::Default {
                        continue;
                    }
                } else {
                    li[cont_level].1 = true;
                }
                if let Some(r) = handle_annotation(cx, c) {
                    cx.st.found_match = true;
                    cx.st.need_separator = true;
                    cx.st.printed_something = true;
                    cx.st.returnval = 1;
                    return Ok(r);
                }
                if !c.desc.is_empty() {
                    cx.st.found_match = true;
                }
                if print && !c.desc.is_empty() {
                    cx.st.returnval = 1;
                    if !cx.st.printed_something {
                        cx.st.printed_something = true;
                        print_sep(cx);
                    }
                    if cx.st.need_separator && !c.nospace {
                        cx.out.push(b' ');
                    }
                    mprint(cx, c);
                    cx.st.need_separator = true;
                }
                let mut o = 0;
                if !moffset(cx, c, b, &mut o) {
                    cont_level -= 1;
                } else {
                    li[cont_level].0 = o;
                }
                cont_level += 1;
                while li.len() <= cont_level + 1 {
                    li.push((0, false));
                }
                li[cont_level].1 = false;
            }
        }
        if aborted {
            idx = skip_block(magic, idx);
            continue;
        }
        if cx.st.printed_something {
            cx.st.firstline = false;
        }
        if cx.st.found_match {
            if cx.flags & F_CONTINUE == 0 {
                return Ok(cx.st.returnval);
            }
            // So that a separator is printed
            cx.st.printed_something = false;
            cx.st.firstline = false;
        }
        idx += 1;
    }
    Ok(cx.st.returnval)
}

const TEST_BIN: u8 = 1;
const TEST_TEXT: u8 = 2;

/// set_test_type of apprentice.c: which passes may try this rule. (libmagic looks at the
/// top-level line only; the continuations do not count.)
fn set_test_type(block: &mut [Entry]) {
    let top_flags = block[0].flags;
    let mut flag = 0u8;
    for m in block.iter().take(1) {
        match m.ty {
            t if t.is_numeric() => flag |= TEST_BIN,
            Ty::Str | Ty::PStr | Ty::BeStr16 | Ty::LeStr16 => {
                if top_flags & SF_TEXT != 0 {
                    flag |= TEST_TEXT;
                } else {
                    flag |= TEST_BIN;
                }
            }
            Ty::Regex | Ty::Search => {
                if top_flags & SF_BIN != 0 {
                    flag |= TEST_BIN;
                }
                if top_flags & SF_TEXT != 0 {
                    flag |= TEST_TEXT;
                }
                if flag & (TEST_BIN | TEST_TEXT) == 0 {
                    flag |= if looks_utf8_text(&m.s) { TEST_TEXT } else { TEST_BIN };
                }
            }
            _ => {}
        }
    }
    block[0].testflag = flag;
}

/// file_looks_utf8(...) > 0 on a pattern.
fn looks_utf8_text(s: &[u8]) -> bool {
    crate::text::looks_utf8(s) > 0
}

fn skip_block(magic: &[Entry], mut idx: usize) -> usize {
    idx += 1;
    while idx < magic.len() && magic[idx].level != 0 {
        idx += 1;
    }
    idx
}

fn handle_annotation(cx: &mut Ctx, m: &Entry) -> Option<i32> {
    if cx.flags & F_APPLE != 0 && !m.apple.is_empty() {
        print_sep(cx);
        cx.out.extend(m.apple.bytes().take(8));
        return Some(1);
    }
    if cx.flags & F_EXTENSION != 0 && !m.ext.is_empty() {
        print_sep(cx);
        cx.out.extend(m.ext.bytes());
        return Some(1);
    }
    if cx.flags & F_MIME_TYPE != 0 && !m.mime.is_empty() {
        print_sep(cx);
        let mime = varexpand(m.mime.as_bytes(), cx.env.mode);
        cx.out.extend(mime);
        return Some(1);
    }
    None
}

fn print_sep(cx: &mut Ctx) {
    if cx.st.firstline {
        return;
    }
    cx.out.extend(b"\n- ");
}

fn moffset(cx: &Ctx, m: &Entry, b: &[u8], out: &mut i64) -> bool {
    let o = match m.ty {
        t if t.is_numeric() => cx.ms_offset + m.ty.size() as i64,
        Ty::Str | Ty::PStr | Ty::BeStr16 | Ty::LeStr16 => {
            if m.reln == b'=' || m.reln == b'!' {
                cx.ms_offset + m.s.len() as i64
            } else {
                let mut s = cx.value.s.clone();
                if m.s.first().is_none_or(|&c| c == 0)
                    && let Some(p) = s.iter().position(|&c| c == b'\r' || c == b'\n')
                {
                    s.truncate(p);
                }
                let mut o = cx.ms_offset + c_strlen(&s) as i64;
                if m.ty == Ty::PStr {
                    o += pstr_size(m) as i64;
                }
                o
            }
        }
        Ty::Regex => {
            if m.flags & SF_REGEX_START != 0 {
                cx.search.offset as i64
            } else {
                (cx.search.offset + cx.search.rm_len) as i64
            }
        }
        Ty::Search => {
            if m.flags & SF_REGEX_START != 0 {
                cx.search.offset as i64
            } else {
                (cx.search.offset + m.s.len()) as i64
            }
        }
        _ => cx.ms_offset,
    };
    if o < 0 || o as usize > b.len() {
        return false;
    }
    *out = o;
    true
}

fn c_strlen(s: &[u8]) -> usize {
    s.iter().position(|&c| c == 0).unwrap_or(s.len())
}

fn pstr_size(m: &Entry) -> usize {
    if m.flags & SF_PSTR_2BE != 0 || m.flags & SF_PSTR_2LE != 0 {
        2
    } else if m.flags & SF_PSTR_4BE != 0 || m.flags & SF_PSTR_4LE != 0 {
        4
    } else {
        1
    }
}

fn rd(b: &[u8], off: usize, n: usize) -> Option<&[u8]> {
    if off.checked_add(n)? > b.len() { None } else { Some(&b[off..off + n]) }
}

fn read_num(b: &[u8], off: usize, ty: Ty, flip: bool) -> Option<u64> {
    let ty = if flip { ty.flip() } else { ty };
    let n = ty.size();
    let s = rd(b, off, n)?;
    let mut v: u64 = 0;
    let be = matches!(ty, Ty::BeShort | Ty::BeLong | Ty::BeQuad | Ty::BeDate | Ty::BeQDate | Ty::BeWDate | Ty::BeDosDate | Ty::BeDosTime);
    if be {
        for &c in s {
            v = (v << 8) | u64::from(c);
        }
    } else {
        for &c in s.iter().rev() {
            v = (v << 8) | u64::from(c);
        }
    }
    Some(v)
}

fn sext(v: u64, bits: u32, signed: bool) -> i64 {
    if !signed {
        return v as i64;
    }
    match bits {
        8 => v as u8 as i8 as i64,
        16 => v as u16 as i16 as i64,
        32 => v as u32 as i32 as i64,
        _ => v as i64,
    }
}

/// Resolve the offset, read the value and (for use/indirect) run the
/// sub-match. Returns 0 when the data is not there.
#[allow(clippy::too_many_arguments)] // mget carries libmagic's whole match state, as softmagic.c's does
fn mget(cx: &mut Ctx, m: &Entry, b: &[u8], base: usize, cont_level: usize, li: &[(i64, bool)], bintest: bool, text: bool, flip: bool) -> MR<i32> {
    if cx.st.indir_count >= INDIR_MAX {
        return Err(Failure::Error(format!("indirect count ({}) exceeded", cx.st.indir_count)));
    }
    if cx.st.name_count >= NAME_MAX {
        return Err(Failure::Error(format!("name use count ({}) exceeded", cx.st.name_count)));
    }
    let mut offset = cx.ms_offset;
    if let Some(ind) = &m.indir {
        // read the pointer
        let at = offset + base as i64;
        if at < 0 {
            return Ok(0);
        }
        let ty = if flip { ind.ty.flip() } else { ind.ty };
        let Some(raw) = read_num(b, at as usize, ty, false) else { return Ok(0) };
        let raw = if ind.id3 { (((raw >> 24) & 0x7f) << 21) | (((raw >> 16) & 0x7f) << 14) | (((raw >> 8) & 0x7f) << 7) | (raw & 0x7f) } else { raw };
        let mut off = sext(raw, (ty.size() * 8) as u32, ind.signed);
        let mut val = ind.value;
        if ind.via_pointer {
            let q = (offset + base as i64 + val).max(0) as usize;
            let Some(r) = read_num(b, q, ty, false) else { return Ok(0) };
            val = sext(r, (ty.size() * 8) as u32, ind.signed);
        }
        if ind.op != 0 {
            off = match ind.op {
                OP_AND => off & val,
                OP_OR => off | val,
                OP_XOR => off ^ val,
                OP_ADD => off.wrapping_add(val),
                OP_SUB => off.wrapping_sub(val),
                OP_MUL => off.wrapping_mul(val),
                OP_DIV if val != 0 => off / val,
                OP_MOD if val != 0 => off % val,
                _ => off,
            };
            if ind.inverse {
                off = !off;
            }
        } else if ind.inverse {
            off = !off;
        }
        offset = off;
        if m.indir_rel {
            if cont_level == 0 {
                return Ok(0);
            }
            offset += li[cont_level - 1].0;
            if offset == 0 {
                return Ok(0);
            }
        }
        cx.ms_offset = offset;
    }
    let abs = if m.indir.is_some() { offset } else { offset + base as i64 };
    let off_neg = abs < 0;
    let abs_u = if off_neg { 0 } else { abs as usize };
    let nbytes = b.len();
    // data checks and copy
    match m.ty {
        Ty::Search => {
            if off_neg {
                return Ok(0);
            }
            let o = abs_u.min(nbytes);
            cx.search = Search { start: o, len: nbytes - o, offset: o, rm_len: 0, text: Vec::new() };
            if abs_u.saturating_add(m.s.len()) > nbytes {
                return Ok(0);
            }
            return Ok(1);
        }
        Ty::Regex => {
            if off_neg || nbytes < abs_u {
                return Ok(0);
            }
            let (linecnt, mut bytecnt) = if m.flags & SF_REGEX_LINES != 0 { (m.range, m.range * 80) } else { (0, m.range) };
            if bytecnt == 0 || bytecnt > nbytes - abs_u {
                bytecnt = nbytes - abs_u;
            }
            if bytecnt > REGEX_MAX {
                bytecnt = REGEX_MAX;
            }
            let end = abs_u + bytecnt;
            let mut last = end;
            if linecnt > 0 {
                let mut lines = linecnt;
                let mut p = abs_u;
                last = end;
                while lines > 0 && p < end {
                    let Some(q) = b[p..end].iter().position(|&c| c == b'\n').or_else(|| b[p..end].iter().position(|&c| c == b'\r')) else { break };
                    let mut q = p + q;
                    if q + 1 < end && b[q] == b'\r' && b[q + 1] == b'\n' {
                        q += 1;
                    }
                    if q + 1 < end && b[q] == b'\n' {
                        q += 1;
                    }
                    last = q;
                    lines -= 1;
                    p = q + 1;
                }
                if lines > 0 {
                    last = end;
                }
            }
            cx.search = Search { start: abs_u, len: last - abs_u, offset: abs_u, rm_len: 0, text: Vec::new() };
            return Ok(1);
        }
        Ty::Indirect => {
            let mut o = offset.max(0) as usize;
            if m.flags & SF_INDIRECT_REL != 0 {
                o += base;
            }
            if off_neg || o == 0 || nbytes < o {
                return Ok(0);
            }
            cx.st.indir_count += 1;
            let sub = &b[o..];
            let saved_out = std::mem::take(&mut cx.out);
            let saved_ms = cx.ms_offset;
            let mut inner_st = St::new();
            inner_st.printed_something = cx.st.printed_something;
            inner_st.need_separator = cx.st.need_separator;
            inner_st.firstline = cx.st.firstline;
            inner_st.indir_count = cx.st.indir_count;
            inner_st.name_count = cx.st.name_count;
            let outer_st = std::mem::replace(&mut cx.st, inner_st);
            let rules = cx.rules;
            let rv = do_match(cx, &rules.flat, sub, 0, true, text, false);
            let inner = std::mem::replace(&mut cx.st, outer_st);
            let rbuf = std::mem::replace(&mut cx.out, saved_out);
            cx.ms_offset = saved_ms;
            cx.st.printed_something = inner.printed_something;
            cx.st.need_separator = inner.need_separator;
            cx.st.firstline = inner.firstline;
            let rv = rv?;
            if rv == 1 {
                if cx.flags & F_NODESC == 0 {
                    let mut d = Vec::new();
                    printf(&m.desc, &Arg::Num(o as u64, false), &mut d);
                    cx.out.extend(d);
                }
                cx.out.extend(rbuf);
            }
            return Ok(rv);
        }
        Ty::Use => {
            if off_neg || nbytes < abs_u {
                return Ok(0);
            }
            let name = String::from_utf8_lossy(&m.s).into_owned();
            let (name, flip2) = match name.strip_prefix('^') {
                Some(n) => (n.to_string(), !flip),
                None => (name, flip),
            };
            let rules = cx.rules;
            let Some(sub) = rules.names.get(&name) else { return Err(Failure::Error(format!("cannot find entry `{name}'"))) };
            let oneed = cx.st.need_separator;
            if m.nospace {
                cx.st.need_separator = false;
            }
            cx.st.name_count += 1;
            let saved_found = cx.st.found_match;
            cx.st.found_match = false;
            let saved_ms = cx.ms_offset;
            let rv = do_match(cx, sub, b, (offset + base as i64).max(0) as usize, bintest, text, flip2)?;
            let nf = cx.st.found_match;
            cx.st.name_count -= 1;
            cx.value.q = u64::from(nf);
            cx.st.found_match = saved_found || nf;
            if rv != 1 {
                cx.st.need_separator = oneed;
            }
            cx.ms_offset = saved_ms;
            return Ok(i32::from(rv != 0 || cx.st.found_match));
        }
        Ty::Name => {
            if cx.flags & F_NODESC != 0 {
                return Ok(1);
            }
            cx.out.extend(&m.desc);
            return Ok(1);
        }
        Ty::Default | Ty::Clear => return Ok(1),
        _ => {}
    }
    if off_neg {
        cx.value = Value::default();
        return Ok(0);
    }
    if m.ty == Ty::BeStr16 || m.ty == Ty::LeStr16 {
        if abs_u >= nbytes {
            return Ok(0);
        }
        let mut s = Vec::new();
        let mut i = abs_u + usize::from(m.ty == Ty::BeStr16);
        while i < nbytes && s.len() < MAXSTRING - 1 {
            let c = b[i];
            let mut c2 = c;
            if c == 0 {
                let next_nonzero = if m.ty == Ty::BeStr16 { i >= 1 && b[i - 1] != 0 } else { i + 1 < nbytes && b[i + 1] != 0 };
                if next_nonzero {
                    c2 = b' ';
                }
            }
            s.push(c2);
            if c == 0 && c2 == 0 {
                break;
            }
            i += 2;
        }
        s.resize(MAXSTRING, 0);
        cx.value = Value { q: 0, s };
        return Ok(1);
    }
    // string and numeric reads
    if m.ty == Ty::Str || m.ty == Ty::PStr {
        if abs_u >= nbytes && !(abs_u == nbytes && m.s.is_empty()) {
            // OFFSET_OOB
            if abs_u.saturating_add(m.s.len()) > nbytes || abs_u >= nbytes {
                return Ok(0);
            }
        }
        if abs_u.saturating_add(m.s.len()) > nbytes {
            return Ok(0);
        }
        let mut size = MAXSTRING;
        if m.range != 0 && m.range < size {
            size = m.range;
        }
        let end = (abs_u + size).min(nbytes);
        let mut s = b[abs_u..end].to_vec();
        if m.ty == Ty::PStr {
            let sz = pstr_size(m);
            let Some(hdr) = rd(&s, 0, sz) else { return Ok(0) };
            let mut len: usize = 0;
            let be = m.flags & (SF_PSTR_2BE | SF_PSTR_4BE) != 0;
            if be {
                for &c in hdr {
                    len = (len << 8) | c as usize;
                }
            } else {
                for &c in hdr.iter().rev() {
                    len = (len << 8) | c as usize;
                }
            }
            if m.flags & SF_PSTR_SELF != 0 {
                len = len.saturating_sub(sz);
            }
            let len = len.min(MAXSTRING - 1);
            let body: Vec<u8> = s.iter().skip(sz).take(len).copied().collect();
            s = body;
        }
        s.resize(s.len().max(MAXSTRING), 0);
        s[MAXSTRING - 1] = 0;
        cx.value = Value { q: 0, s };
        return Ok(1);
    }
    // numeric
    let size = m.ty.size();
    if abs_u.saturating_add(size) > nbytes {
        return Ok(0);
    }
    let Some(mut v) = read_num(b, abs_u, m.ty, flip) else { return Ok(0) };
    // mask op
    if m.mask != 0 {
        v = match m.mask_op {
            OP_AND => v & m.mask,
            OP_OR => v | m.mask,
            OP_XOR => v ^ m.mask,
            OP_ADD => v.wrapping_add(m.mask),
            OP_SUB => v.wrapping_sub(m.mask),
            OP_MUL => v.wrapping_mul(m.mask),
            OP_DIV if m.mask != 0 => v / m.mask,
            OP_MOD if m.mask != 0 => v % m.mask,
            _ => v,
        };
    }
    if m.mask_inverse {
        v = !v;
    }
    cx.value = Value { q: v, s: Vec::new() };
    Ok(1)
}

fn magiccheck(cx: &mut Ctx, m: &Entry, b: &[u8]) -> bool {
    match m.ty {
        Ty::Use => return cx.value.q != 0,
        Ty::Name | Ty::Indirect => return true,
        Ty::Default | Ty::Clear => return true,
        _ => {}
    }
    if m.reln == b'x' {
        return true;
    }
    let v: u64;
    let l: u64;
    match m.ty {
        Ty::Str | Ty::PStr | Ty::BeStr16 | Ty::LeStr16 => {
            let eq = if m.flags == 0 || m.flags & !(SF_BIN | SF_TEXT) == 0 {
                cx.value.s.len() >= m.s.len() && cx.value.s[..m.s.len()] == m.s[..]
            } else {
                str_match(&m.s, &cx.value.s, m.flags)
            };
            // strings compare as ordered values for < and >
            return match m.reln {
                b'=' => eq,
                b'!' => !eq,
                b'>' | b'<' => {
                    let win = &cx.value.s;
                    let ord = win.iter().zip(m.s.iter()).map(|(a, c)| i32::from(*a) - i32::from(*c)).find(|d| *d != 0).unwrap_or(0);
                    if m.reln == b'>' { ord > 0 } else { ord < 0 }
                }
                _ => false,
            };
        }
        Ty::Search => {
            let slen = m.s.len();
            let s = &b[cx.search.start..cx.search.start + cx.search.len];
            let mut found = None;
            let mut idx = 0;
            while m.range == 0 || idx < m.range {
                if slen + idx > s.len() {
                    break;
                }
                if str_match(&m.s, &s[idx..], m.flags) {
                    found = Some(idx);
                    break;
                }
                idx += 1;
            }
            return match found {
                Some(i) => {
                    cx.search.offset += i;
                    cx.search.rm_len = s.len() - i;
                    m.reln != b'!'
                }
                None => m.reln == b'!',
            };
        }
        Ty::Regex => {
            let s = &b[cx.search.start..cx.search.start + cx.search.len];
            let r = m.regex.as_ref().expect("compiled");
            // A rule whose pattern backtracks without end finds no match, as PHP's pcre.backtrack_limit makes it.
            let hit = match r.exec(s, 0, &MatchOptions { match_limit: MATCH_LIMIT, ..MatchOptions::default() }) {
                Ok(Some(c)) => c.get(0),
                Ok(None) => None,
                Err(_) => {
                    cx.failed = true;
                    return false;
                }
            };
            return match hit {
                Some((a, e)) => {
                    cx.search.offset += a;
                    cx.search.rm_len = e - a;
                    cx.search.text = s[a..e].to_vec();
                    m.reln != b'!'
                }
                None => m.reln == b'!',
            };
        }
        _ => {
            v = sign_extend(m, cx.value.q);
            l = m.num;
        }
    }
    let unsigned = m.unsigned;
    match m.reln {
        b'x' => true,
        b'!' => v != l,
        b'=' => v == l,
        b'>' => {
            if unsigned { v > l } else { (v as i64) > (l as i64) }
        }
        b'<' => {
            if unsigned { v < l } else { (v as i64) < (l as i64) }
        }
        b'&' => v & l == l,
        b'^' => v & l != l,
        _ => false,
    }
}

/// libmagic's file_strncmp: does `pat` match the start of `text`?
pub fn str_match(pat: &[u8], text: &[u8], flags: u32) -> bool {
    let flags = flags & !(SF_BIN | SF_TEXT | SF_REGEX_START | SF_REGEX_LINES | SF_INDIRECT_REL);
    if flags == 0 {
        return text.len() >= pat.len() && text[..pat.len()] == *pat;
    }
    let eb = text.len();
    let mut a = 0;
    let mut bi = 0;
    while a < pat.len() {
        if bi >= eb {
            return false;
        }
        let pc = pat[a];
        if flags & SF_LOWER != 0 && pc.is_ascii_lowercase() {
            if text[bi].to_ascii_lowercase() != pc {
                return false;
            }
            a += 1;
            bi += 1;
        } else if flags & SF_UPPER != 0 && pc.is_ascii_uppercase() {
            if text[bi].to_ascii_uppercase() != pc {
                return false;
            }
            a += 1;
            bi += 1;
        } else if flags & SF_COMPACT_WS != 0 && is_space(pc) {
            a += 1;
            if is_space(text[bi]) {
                bi += 1;
                if !pat.get(a).is_some_and(|&c| is_space(c)) {
                    let mut n = 0;
                    while n < 2048 && bi < eb && is_space(text[bi]) {
                        bi += 1;
                        n += 1;
                    }
                }
            } else {
                return false;
            }
        } else if flags & SF_OPT_WS != 0 && is_space(pc) {
            a += 1;
            while bi < eb && is_space(text[bi]) {
                bi += 1;
            }
        } else {
            if text[bi] != pc {
                return false;
            }
            a += 1;
            bi += 1;
        }
    }
    if flags & SF_FULL_WORD != 0 && bi < eb && text[bi] != 0 && !is_space(text[bi]) {
        return false;
    }
    true
}

// ---- printing ----

fn file_printable(s: &[u8], out: &mut Vec<u8>) {
    for &c in s {
        if (0x20..0x7f).contains(&c) {
            out.push(c);
        } else {
            out.extend(format!("\\{c:03o}").bytes());
        }
    }
}

fn mprint(cx: &mut Ctx, m: &Entry) {
    let expanded = varexpand(&m.desc, cx.env.mode);
    let desc: &[u8] = &expanded;
    let mut out = Vec::new();
    match m.ty {
        Ty::Str | Ty::PStr | Ty::BeStr16 | Ty::LeStr16 => {
            let mut s = if m.reln == b'=' || m.reln == b'!' {
                m.s.clone()
            } else {
                let mut s = cx.value.s.clone();
                s.truncate(c_strlen(&s));
                if m.s.first().is_none_or(|&c| c == 0)
                    && let Some(p) = s.iter().position(|&c| c == b'\r' || c == b'\n')
                {
                    s.truncate(p);
                }
                if m.flags & SF_TRIM != 0 {
                    while s.last().is_some_and(|c| is_space(*c)) {
                        s.pop();
                    }
                    let lead = s.iter().take_while(|&&c| is_space(c)).count();
                    s.drain(..lead);
                }
                s
            };
            s.truncate(c_strlen(&s));
            let mut p = Vec::new();
            file_printable(&s, &mut p);
            printf(desc, &Arg::Str(&p), &mut out);
        }
        Ty::Search => {
            let mut p = Vec::new();
            file_printable(&m.s, &mut p);
            printf(desc, &Arg::Str(&p), &mut out);
        }
        Ty::Regex => {
            let mut p = Vec::new();
            file_printable(&cx.search.text, &mut p);
            printf(desc, &Arg::Str(&p), &mut out);
        }
        Ty::Default | Ty::Clear => out.extend(desc),
        Ty::Name | Ty::Use | Ty::Indirect => {}
        Ty::DosDate | Ty::BeDosDate => {
            let v = (cx.value.q & 0xffff) as u32;
            const MO: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
            let mon = (v >> 5) & 0xf;
            let name = if (1..=12).contains(&mon) { MO[mon as usize - 1] } else { "?" };
            let text = format!("Sun, {name} {:02} {}", v & 0x1f, 1980 + (v >> 9));
            printf(desc, &Arg::Str(text.as_bytes()), &mut out);
        }
        Ty::DosTime | Ty::BeDosTime => {
            let v = (cx.value.q & 0xffff) as u32;
            let text = format!("{:02}:{:02}:{:02}", v >> 11, (v >> 5) & 0x3f, (v & 0x1f) * 2);
            printf(desc, &Arg::Str(text.as_bytes()), &mut out);
        }
        t if t.is_date() => {
            let v = sign_extend(m, cx.value.q);
            let secs = match t {
                Ty::WDate | Ty::BeWDate | Ty::LeWDate => ((v as i64) / 10_000_000).wrapping_sub(11_644_473_600),
                Ty::Date | Ty::BeDate | Ty::LeDate => i64::from(v as u32 as i32) * i64::from(!m.unsigned) + i64::from(v as u32) * i64::from(m.unsigned),
                _ => v as i64,
            };
            printf(desc, &Arg::Str(format_time(secs).as_bytes()), &mut out);
        }
        _ => {
            let v = sign_extend(m, cx.value.q);
            printf(desc, &Arg::Num(v, m.ty.size() == 8), &mut out);
        }
    }
    cx.out.extend(out);
}

pub enum Arg<'a> {
    /// A number as C passes it: promoted to int (or 64 bits when `wide`), whatever its signedness.
    Num(u64, bool),
    Str(&'a [u8]),
}

/// A printf over the one conversion libmagic descriptions carry.
pub fn printf(fmt: &[u8], arg: &Arg, out: &mut Vec<u8>) {
    let b = fmt;
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'%' {
            out.push(b[i]);
            i += 1;
            continue;
        }
        i += 1;
        if b.get(i) == Some(&b'%') {
            out.push(b'%');
            i += 1;
            continue;
        }
        let (mut left, mut zero, mut plus, mut alt) = (false, false, false, false);
        while let Some(&c) = b.get(i) {
            match c {
                b'-' => left = true,
                b'0' => zero = true,
                b'+' => plus = true,
                b'#' => alt = true,
                b' ' => {}
                _ => break,
            }
            i += 1;
        }
        let mut width = 0usize;
        while let Some(d) = b.get(i).filter(|c| c.is_ascii_digit()) {
            width = width * 10 + usize::from(d - b'0');
            i += 1;
        }
        let mut prec: Option<usize> = None;
        if b.get(i) == Some(&b'.') {
            i += 1;
            let mut p = 0;
            while let Some(d) = b.get(i).filter(|c| c.is_ascii_digit()) {
                p = p * 10 + usize::from(d - b'0');
                i += 1;
            }
            prec = Some(p);
        }
        while b.get(i).is_some_and(|c| matches!(c, b'l' | b'h' | b'z' | b'j' | b'q' | b'L')) {
            i += 1;
        }
        let Some(&conv) = b.get(i) else { break };
        i += 1;
        let mut body: Vec<u8> = match (conv, arg) {
            (b's', Arg::Str(s)) => {
                let mut s = s.to_vec();
                if let Some(p) = prec {
                    s.truncate(p);
                }
                s
            }
            (b's', Arg::Num(v, wide)) => num_i(*v, *wide).to_string().into_bytes(),
            (b'd' | b'i', Arg::Num(v, wide)) => {
                let n = num_i(*v, *wide);
                let mut s = n.to_string();
                if plus && n >= 0 {
                    s.insert(0, '+');
                }
                s.into_bytes()
            }
            (b'u', Arg::Num(v, wide)) => num_u(*v, *wide).to_string().into_bytes(),
            (b'x', Arg::Num(v, wide)) => format!("{}{:x}", if alt && num_u(*v, *wide) != 0 { "0x" } else { "" }, num_u(*v, *wide)).into_bytes(),
            (b'X', Arg::Num(v, wide)) => format!("{:X}", num_u(*v, *wide)).into_bytes(),
            (b'o', Arg::Num(v, wide)) => format!("{:o}", num_u(*v, *wide)).into_bytes(),
            (b'c', Arg::Num(v, _)) => vec![*v as u8],
            _ => Vec::new(),
        };
        if let (Some(p), true) = (prec, matches!(conv, b'd' | b'i' | b'u' | b'x' | b'X' | b'o')) {
            while body.len() < p {
                body.insert(0, b'0');
            }
        }
        if body.len() < width {
            let pad = width - body.len();
            if left {
                body.extend(std::iter::repeat_n(b' ', pad));
            } else if zero && conv != b's' {
                let neg = body.first().is_some_and(|c| *c == b'-' || *c == b'+');
                let at = usize::from(neg);
                for _ in 0..pad {
                    body.insert(at, b'0');
                }
            } else {
                for _ in 0..pad {
                    body.insert(0, b' ');
                }
            }
        }
        out.extend(body);
    }
}

fn num_i(v: u64, wide: bool) -> i64 {
    if wide { v as i64 } else { v as u32 as i32 as i64 }
}

fn num_u(v: u64, wide: bool) -> u64 {
    if wide { v } else { v as u32 as u64 }
}

/// asctime(gmtime(secs)) without the newline: `Fri May 21 05:20:00 1993`.
fn format_time(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // civil from days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    const WD: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MO: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{} {} {:2} {:02}:{:02}:{:02} {}", WD[days.rem_euclid(7) as usize], MO[(m - 1) as usize], d, rem / 3600, rem % 3600 / 60, rem % 60, y)
}

/// varexpand: `${x?then:else}` picks `then` when the file is executable.
fn varexpand(s: &[u8], mode: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i..].starts_with(b"${")
            && s.get(i + 2) == Some(&b'x')
            && s.get(i + 3) == Some(&b'?')
            && let Some(colon) = s[i + 4..].iter().position(|&c| c == b':')
            && let Some(close) = s[i + 4 + colon + 1..].iter().position(|&c| c == b'}')
        {
            let then = &s[i + 4..i + 4 + colon];
            let other = &s[i + 4 + colon + 1..i + 4 + colon + 1 + close];
            out.extend(if mode & 0o111 != 0 { then } else { other });
            i += 4 + colon + 1 + close + 1;
        } else {
            out.push(s[i]);
            i += 1;
        }
    }
    out
}
