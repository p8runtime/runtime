//! Character properties and case over PCRE2's own tables (ucd.rs), with the
//! lookups pcre2_internal.h's UCD_* macros make.

use crate::ucd::{BOOL_SETS, CASELESS_SETS, PROPERTIES, RECORDS, Record, SCRIPT_SETS, STAGE1, STAGE2};

// PCRE2's chartype numbers (pcre2_ucp.h).
pub(crate) const LL: u8 = 5;
pub(crate) const LT: u8 = 8;
pub(crate) const LU: u8 = 9;
pub(crate) const MN: u8 = 12;
pub(crate) const ND: u8 = 13;
pub(crate) const PC: u8 = 16;

// PCRE2's property kinds (PT_* in pcre2_internal.h).
pub(crate) const PT_ANY: u8 = 0;
pub(crate) const PT_LAMP: u8 = 1;
pub(crate) const PT_GC: u8 = 2;
pub(crate) const PT_PC: u8 = 3;
pub(crate) const PT_SC: u8 = 4;
pub(crate) const PT_SCX: u8 = 5;
pub(crate) const PT_ALNUM: u8 = 6;
pub(crate) const PT_SPACE: u8 = 7;
pub(crate) const PT_PXSPACE: u8 = 8;
pub(crate) const PT_WORD: u8 = 9;
pub(crate) const PT_UCNC: u8 = 11;
pub(crate) const PT_BIDICL: u8 = 12;
pub(crate) const PT_BOOL: u8 = 13;
// The POSIX classes under UCP (pcre2_internal.h's PT_PX*).
pub(crate) const PT_PXGRAPH: u8 = 14;
pub(crate) const PT_PXPRINT: u8 = 15;
pub(crate) const PT_PXPUNCT: u8 = 16;

// General categories (ucp_C … ucp_Z).
pub(crate) const GC_L: u8 = 1;
pub(crate) const GC_N: u8 = 3;
pub(crate) const GC_P: u8 = 4;
pub(crate) const GC_S: u8 = 5;
pub(crate) const GC_Z: u8 = 6;

/// Each chartype's general category (PRIV(ucp_gentype)).
const GENTYPE: [u8; 30] = [
    0, 0, 0, 0, 0, // Cc Cf Cn Co Cs
    1, 1, 1, 1, 1, // Ll Lm Lo Lt Lu
    2, 2, 2, // Mc Me Mn
    3, 3, 3, // Nd Nl No
    4, 4, 4, 4, 4, 4, 4, // Pc Pd Pe Pf Pi Po Ps
    5, 5, 5, 5, // Sc Sk Sm So
    6, 6, 6, // Zl Zp Zs
];

/// A code point's record; past U+10FFFF, unassigned.
fn record(c: u32) -> Record {
    if c > 0x10ffff {
        return (0, 2, 12, 0, 0, 0, 9, 0);
    }
    let block = STAGE1[(c / 128) as usize] as usize;
    RECORDS[STAGE2[block * 128 + (c % 128) as usize] as usize]
}

pub(crate) fn grapheme_break(c: u32) -> u8 {
    record(c).2
}

/// The other case of `c` (itself when it has none).
pub(crate) fn other_case(c: u32) -> u32 {
    (c as i64 + record(c).4 as i64) as u32
}

/// The characters caseless-equal to `c` beyond its other case, when it has
/// more than one (K, k and the Kelvin sign).
pub(crate) fn case_set(c: u32) -> &'static [u32] {
    let at = record(c).3 as usize;
    if at == 0 {
        return &[];
    }
    let end = CASELESS_SETS[at..].iter().position(|&x| x == u32::MAX).map_or(CASELESS_SETS.len(), |n| at + n);
    &CASELESS_SETS[at..end]
}

/// Whether `a` and `b` are the same letter ignoring case, by Unicode.
pub(crate) fn caseless_eq(a: u32, b: u32) -> bool {
    a == b || other_case(a) == b || case_set(a).contains(&b)
}

fn bit(map: &[u32], n: u16) -> bool {
    map.get(n as usize / 32).is_some_and(|w| w & (1 << (n % 32)) != 0)
}

/// Whether `c` has the property (kind, value), as pcre2_xclass.c tests it.
pub(crate) fn has_property(c: u32, kind: u8, value: u16) -> bool {
    let r = record(c);
    let gc = GENTYPE[r.1 as usize];
    match kind {
        PT_ANY => true,
        PT_LAMP => matches!(r.1, LU | LL | LT),
        PT_GC => gc as u16 == value,
        PT_PC => r.1 as u16 == value,
        PT_SC => r.0 as u16 == value,
        PT_SCX => r.0 as u16 == value || bit(&SCRIPT_SETS[r.5 as usize..], value),
        PT_ALNUM => gc == GC_L || gc == GC_N,
        PT_SPACE | PT_PXSPACE => gc == GC_Z || matches!(c, 9..=13),
        PT_WORD => gc == GC_L || gc == GC_N || r.1 == MN || r.1 == PC,
        PT_UCNC => c == '$' as u32 || c == '@' as u32 || c == '`' as u32 || (0xa0..=0xd7ff).contains(&c) || c >= 0xe000,
        PT_BIDICL => r.6 as u16 == value,
        PT_BOOL => bit(&BOOL_SETS[r.7 as usize..], value),
        PT_PXGRAPH => {
            // Not a space, control, unassigned or surrogate (but format
            // characters other than U+061C and U+180E count).
            !(gc == GC_Z || matches!(r.1, 0 | 2 | 4) || (r.1 == 1 && (c == 0x061c || (0x2066..=0x2069).contains(&c))))
        }
        PT_PXPRINT => {
            (gc != 0 || r.1 == 1 || r.1 == 3) && !(r.1 == 1 && (c == 0x061c || (0x2066..=0x2069).contains(&c)))
                || r.1 == 29 // Zs
        }
        PT_PXPUNCT => gc == GC_P || (c < 128 && gc == GC_S),
        _ => false,
    }
}

/// A property by its name in `\p{…}`: PCRE2 ignores case, spaces, hyphens
/// and underscores, and takes `sc:`/`scx:`/`bc:` prefixes.
pub(crate) fn property(name: &[u8]) -> Option<(u8, u16)> {
    let mut key: Vec<u8> = name
        .iter()
        .filter(|b| !matches!(b, b' ' | b'-' | b'_'))
        .map(|b| b.to_ascii_lowercase())
        .collect();
    let mut want = None;
    if let Some(colon) = key.iter().position(|&b| b == b':' || b == b'=') {
        let prefix = key[..colon].to_vec();
        key.drain(..=colon);
        want = Some(match prefix.as_slice() {
            b"sc" | b"script" => PT_SC,
            b"scx" | b"scriptextensions" => PT_SCX,
            b"bc" | b"bidiclass" => PT_BIDICL,
            _ => return None,
        });
        if want == Some(PT_BIDICL) {
            let bidi: Vec<u8> = [b"bidi".as_slice(), &key].concat();
            return PROPERTIES.iter().find(|p| p.0.as_bytes() == bidi.as_slice()).map(|p| (p.1, p.2));
        }
    }
    let found = PROPERTIES.iter().find(|p| p.0.as_bytes() == key.as_slice())?;
    match want {
        // A script named with sc: tests the Script property alone.
        Some(PT_SC) if found.1 == PT_SC || found.1 == PT_SCX => Some((PT_SC, found.2)),
        Some(PT_SCX) if found.1 == PT_SC || found.1 == PT_SCX => Some((PT_SCX, found.2)),
        Some(_) => None,
        None => Some((found.1, found.2)),
    }
}

/// Grapheme cluster break rules (PRIV(ucp_gbtable), Unicode 15): whether a
/// cluster continues from a character with break property `a` to one with `b`.
pub(crate) fn grapheme_joins(a: u8, b: u8) -> bool {
    const CR: u8 = 0;
    const LF: u8 = 1;
    const CONTROL: u8 = 2;
    const EXTEND: u8 = 3;
    const PREPEND: u8 = 4;
    const SPACING: u8 = 5;
    const L: u8 = 6;
    const V: u8 = 7;
    const T: u8 = 8;
    const LV: u8 = 9;
    const LVT: u8 = 10;
    const RI: u8 = 11;
    const ZWJ: u8 = 13;
    const PICT: u8 = 14;
    match (a, b) {
        (CR, LF) => true,
        (CR | LF | CONTROL, _) | (_, CR | LF | CONTROL) => false,
        (L, L | V | LV | LVT) => true,
        (LV | V, V | T) => true,
        (LVT | T, T) => true,
        (_, EXTEND | ZWJ | SPACING) => true,
        (PREPEND, _) => true,
        (ZWJ, PICT) => true,
        (RI, RI) => true,
        _ => false,
    }
}

pub(crate) const GB_RI: u8 = 11;
pub(crate) const GB_EXTEND: u8 = 3;
pub(crate) const GB_PICT: u8 = 14;
pub(crate) const GB_ZWJ: u8 = 13;
