//! PCRE2-compatible regular expressions in safe Rust. See README.md.
//!
//! A pattern is parsed (parse.rs) into a tree, compiled (compile.rs) into a
//! program, and run by a backtracking machine (exec.rs) with PCRE2's
//! semantics: Perl-style leftmost-first alternation, captures, back
//! references, lookaround, atomic groups, possessive quantifiers, recursion,
//! conditionals and the backtracking verbs, with PCRE2's own Unicode tables
//! (ucd.rs) and error messages.
#![forbid(unsafe_code)]

mod compile;
mod errors;
mod exec;
mod parse;
mod ucd;
mod unicode;

use std::fmt;

/// Compile options: PCRE2's option bits that PHP's modifiers set.
#[derive(Clone, Default, Debug)]
pub struct Options {
    pub caseless: bool,
    pub multiline: bool,
    pub dotall: bool,
    pub extended: bool,
    pub anchored: bool,
    pub dollar_endonly: bool,
    pub ungreedy: bool,
    pub utf: bool,
    pub ucp: bool,
    pub dupnames: bool,
    pub no_auto_capture: bool,
    /// PCRE2_EXTRA_CASELESS_RESTRICT: caseless matching never pairs an
    /// ASCII character with a non-ASCII one.
    pub caseless_restrict: bool,
}

/// Match-time options and limits (pcre2_match's options and match context).
#[derive(Clone, Debug)]
pub struct MatchOptions {
    pub anchored: bool,
    pub not_empty: bool,
    pub not_empty_atstart: bool,
    pub match_limit: u64,
    pub depth_limit: u64,
}

impl Default for MatchOptions {
    fn default() -> Self {
        MatchOptions { anchored: false, not_empty: false, not_empty_atstart: false, match_limit: 10_000_000, depth_limit: 10_000_000 }
    }
}

/// A compile error: PCRE2's error number, message and pattern offset.
#[derive(Clone, Debug, PartialEq)]
pub struct CompileError {
    pub code: u32,
    pub offset: usize,
    message: String,
}

impl CompileError {
    pub(crate) fn new(code: u32, offset: usize) -> CompileError {
        let message = errors::COMPILE.get(code as usize - 100).copied().unwrap_or("internal error").to_string();
        CompileError { code, offset, message }
    }

    pub(crate) fn utf(code: u32, offset: usize) -> CompileError {
        CompileError { code, offset, message: utf_error_message(code).to_string() }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at offset {}", self.message, self.offset)
    }
}

/// Why a match stopped without an answer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MatchError {
    /// pcre2_match's match limit (PHP's pcre.backtrack_limit).
    MatchLimit,
    /// pcre2_match's depth limit (PHP's pcre.recursion_limit).
    DepthLimit,
}

/// A successful match: offsets of each group (0 is the whole match).
#[derive(Clone, Debug, Default)]
pub struct Captures {
    slots: Vec<Option<usize>>,
    pub mark: Option<Vec<u8>>,
}

impl Captures {
    /// Group `i`'s (start, end), or None when it did not take part.
    pub fn get(&self, i: usize) -> Option<(usize, usize)> {
        match (self.slots.get(i * 2)?, self.slots.get(i * 2 + 1)?) {
            (Some(a), Some(b)) => Some((*a, *b)),
            _ => None,
        }
    }

    /// Groups 0..=n: one more than the highest group that took part, as
    /// pcre2_match's return value counts them.
    pub fn len(&self) -> usize {
        (0..self.slots.len() / 2).rev().find(|&i| self.get(i).is_some()).map_or(0, |i| i + 1)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub struct Regex {
    prog: compile::Prog,
    groups: usize,
    names: Vec<(Vec<u8>, usize)>,
}

impl Regex {
    pub fn new(pattern: &[u8], options: &Options) -> Result<Regex, CompileError> {
        let parsed = parse::parse(pattern, options)?;
        let groups = parsed.groups;
        let mut names = parsed.names.clone();
        names.sort_by(|a, b| a.0.cmp(&b.0));
        let prog = compile::compile(parsed, options.anchored, pattern.len())?;
        Ok(Regex { prog, groups, names })
    }

    /// The number of capturing groups.
    pub fn groups(&self) -> usize {
        self.groups
    }

    /// Named groups, sorted by name (pcre2's name table).
    pub fn names(&self) -> &[(Vec<u8>, usize)] {
        &self.names
    }

    /// Whether the pattern is in UTF mode (including by `(*UTF)`).
    pub fn is_utf(&self) -> bool {
        self.prog.utf
    }

    /// How many characters before the start offset a match may inspect
    /// (PCRE2_INFO_MAXLOOKBEHIND).
    pub fn max_lookbehind(&self) -> usize {
        self.prog.max_lookbehind as usize
    }

    /// Looks for a match at or after byte `start`.
    pub fn exec(&self, subject: &[u8], start: usize, options: &MatchOptions) -> Result<Option<Captures>, MatchError> {
        exec::exec(&self.prog, subject, start.min(subject.len()), options)
    }

    /// [`exec`](Regex::exec) into `caps`, reusing its room (a loop over a
    /// subject's matches allocates nothing per match): whether it matched.
    /// `caps` holds the match only when it did.
    pub fn exec_into(&self, subject: &[u8], start: usize, options: &MatchOptions, caps: &mut Captures) -> Result<bool, MatchError> {
        exec::exec_into(&self.prog, subject, start.min(subject.len()), options, caps)
    }
}

/// The Unicode version of the character tables.
pub fn unicode_version() -> &'static str {
    ucd::UNICODE_VERSION
}

/// Decodes the character at `at`: a code point in UTF mode, else a byte.
/// Bytes that don't form valid UTF-8 read as themselves.
pub(crate) fn decode(s: &[u8], at: usize, utf: bool) -> (u32, usize) {
    let b = s[at];
    if !utf || b < 0x80 {
        return (b as u32, 1);
    }
    let (len, init) = match b {
        0xc0..=0xdf => (2, (b & 0x1f) as u32),
        0xe0..=0xef => (3, (b & 0x0f) as u32),
        0xf0..=0xf7 => (4, (b & 0x07) as u32),
        _ => return (b as u32, 1),
    };
    if at + len > s.len() {
        return (b as u32, 1);
    }
    let mut c = init;
    for &x in &s[at + 1..at + len] {
        if x & 0xc0 != 0x80 {
            return (b as u32, 1);
        }
        c = c << 6 | (x & 0x3f) as u32;
    }
    (c, len)
}

/// Checks UTF-8 as pcre2_valid_utf does: Err((PCRE2 error number, offset)).
pub fn check_utf8(s: &[u8]) -> Result<(), (u32, usize)> {
    let mut i = 0;
    while i < s.len() {
        let b = s[i];
        if b < 0x80 {
            i += 1;
            continue;
        }
        if b < 0xc0 {
            return Err((22, i)); // isolated byte with 0x80 bit set
        }
        if b >= 0xfe {
            return Err((23, i));
        }
        let extra = match b {
            0xc0..=0xdf => 1,
            0xe0..=0xef => 2,
            0xf0..=0xf7 => 3,
            0xf8..=0xfb => 4,
            _ => 5,
        };
        // Missing bytes at the end: errors 3..=7 (1..=5 bytes missing).
        let have = s.len() - i - 1;
        if have < extra {
            return Err((3 + (extra - have - 1) as u32, i));
        }
        for k in 1..=extra {
            if s[i + k] & 0xc0 != 0x80 {
                return Err((7 + k as u32, i)); // byte k+1 top bits not 0x80
            }
        }
        if extra == 4 {
            return Err((13, i));
        }
        if extra == 5 {
            return Err((14, i));
        }
        let (c, _) = decode(s, i, true);
        match extra {
            1 if c < 0x80 => return Err((17, i)),
            2 if c < 0x800 => return Err((18, i)),
            3 if c < 0x10000 => return Err((19, i)),
            _ => {}
        }
        if c > 0x10ffff {
            return Err((15, i));
        }
        if (0xd800..=0xdfff).contains(&c) {
            return Err((16, i));
        }
        i += extra + 1;
    }
    Ok(())
}

fn utf_error_message(code: u32) -> &'static str {
    match code {
        3 => "UTF-8 error: 1 byte missing at end",
        4 => "UTF-8 error: 2 bytes missing at end",
        5 => "UTF-8 error: 3 bytes missing at end",
        6 => "UTF-8 error: 4 bytes missing at end",
        7 => "UTF-8 error: 5 bytes missing at end",
        8 => "UTF-8 error: byte 2 top bits not 0x80",
        9 => "UTF-8 error: byte 3 top bits not 0x80",
        10 => "UTF-8 error: byte 4 top bits not 0x80",
        11 => "UTF-8 error: byte 5 top bits not 0x80",
        12 => "UTF-8 error: byte 6 top bits not 0x80",
        13 => "UTF-8 error: 5-byte character is not allowed (RFC 3629)",
        14 => "UTF-8 error: 6-byte character is not allowed (RFC 3629)",
        15 => "UTF-8 error: code points greater than 0x10ffff are not defined",
        16 => "UTF-8 error: code points 0xd800-0xdfff are not defined",
        17 => "UTF-8 error: overlong 2-byte sequence",
        18 => "UTF-8 error: overlong 3-byte sequence",
        19 => "UTF-8 error: overlong 4-byte sequence",
        22 => "UTF-8 error: isolated byte with 0x80 bit set",
        _ => "UTF-8 error: illegal byte (0xfe or 0xff)",
    }
}

#[cfg(test)]
mod tests;
