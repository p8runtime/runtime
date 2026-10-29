//! Pattern syntax: PCRE2's grammar (pcre2pattern(3)) into a tree.
//!
//! Reference: $PHP_SRC_PATH/ext/pcre/pcre2lib/pcre2_compile.c (parse_regex).

use crate::unicode;
use crate::{CompileError, Options};

/// Option bits a pattern can change as it goes (`(?i)` and the rest).
#[derive(Clone, Copy, Default)]
pub(crate) struct Flags {
    pub caseless: bool,
    pub multiline: bool,
    pub dotall: bool,
    pub extended: bool,
    pub extended_more: bool,
    pub no_auto_capture: bool,
    pub ungreedy: bool,
    pub dupnames: bool,
    pub restrict: bool,
}

#[derive(Clone, Debug)]
pub(crate) enum Node {
    Empty,
    /// `restrict`: caseless, but ASCII and non-ASCII never match (/r).
    Char { c: u32, nocase: bool, restrict: bool },
    /// `.`; `nl` when it matches a newline too (dotall).
    Any { nl: bool },
    /// One code unit (`\C`).
    Byte,
    Set(Box<Set>),
    Assert(Assert),
    /// A capturing group (its number), or a non-capturing one.
    Group(Box<Node>, Option<usize>),
    Atomic(Box<Node>),
    Look { behind: bool, neg: bool, body: Box<Node>, at: usize },
    Concat(Vec<Node>),
    Alt(Vec<Node>),
    Repeat { node: Box<Node>, min: u32, max: Option<u32>, greedy: bool, possessive: bool },
    BackRef { to: Ref, nocase: bool },
    /// A subroutine call or recursion: `(?R)`, `(?1)`, `(?&name)`.
    Call(Ref),
    Cond { cond: Cond, yes: Box<Node>, no: Option<Box<Node>> },
    Keep,
    Verb(Verb),
    /// `\R`: any newline sequence.
    Newline,
    /// `\X`: an extended grapheme cluster.
    Grapheme,
}

#[derive(Clone, Debug)]
pub(crate) enum Ref {
    Num(usize),
    Name(Vec<u8>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Assert {
    /// `\A`, or `^` outside multiline mode.
    Start,
    /// `^` in multiline mode.
    StartLine,
    /// `\z`, or `$` with dollar_endonly.
    End,
    /// `\Z`, or `$`: the end or before a final newline.
    EndOrNewline,
    /// `$` in multiline mode.
    EndLine,
    /// `\G`.
    StartOffset,
    WordBoundary { ucp: bool },
    NotWordBoundary { ucp: bool },
}

#[derive(Clone, Debug)]
pub(crate) enum Cond {
    Group(Ref),
    Recursion(Option<Ref>),
    Define,
    Look { behind: bool, neg: bool, body: Box<Node>, at: usize },
    Always(bool),
}

#[derive(Clone, Debug)]
pub(crate) enum Verb {
    Accept,
    Fail,
    Commit,
    Prune(Option<Vec<u8>>),
    Skip(Option<Vec<u8>>),
    Then(Option<Vec<u8>>),
    Mark(Vec<u8>),
}

#[derive(Clone, Debug)]
pub(crate) struct Set {
    pub items: Vec<Item>,
    pub negated: bool,
    pub nocase: bool,
    pub restrict: bool,
}

#[derive(Clone, Debug)]
pub(crate) enum Item {
    Range(u32, u32),
    Digit(bool),
    Word(bool),
    Space(bool),
    HSpace(bool),
    VSpace(bool),
    /// A POSIX class over the ASCII tables (`[:alpha:]`), negated or not.
    Posix(Posix, bool),
    Prop { kind: u8, value: u16, neg: bool },
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Posix {
    Alpha,
    Digit,
    Alnum,
    Space,
    Blank,
    Cntrl,
    Graph,
    Print,
    Punct,
    Upper,
    Lower,
    Xdigit,
    Word,
    Ascii,
}

impl Posix {
    pub(crate) fn test(self, c: u32) -> bool {
        let Ok(b) = u8::try_from(c) else { return false };
        match self {
            Posix::Alpha => b.is_ascii_alphabetic(),
            Posix::Digit => b.is_ascii_digit(),
            Posix::Alnum => b.is_ascii_alphanumeric(),
            Posix::Space => matches!(b, 9..=13 | b' '),
            Posix::Blank => b == b' ' || b == b'\t',
            Posix::Cntrl => b < 32 || b == 127,
            Posix::Graph => (33..127).contains(&b),
            Posix::Print => (32..127).contains(&b),
            Posix::Punct => b.is_ascii_punctuation(),
            Posix::Upper => b.is_ascii_uppercase(),
            Posix::Lower => b.is_ascii_lowercase(),
            Posix::Xdigit => b.is_ascii_hexdigit(),
            Posix::Word => b.is_ascii_alphanumeric() || b == b'_',
            Posix::Ascii => b < 128,
        }
    }
}

pub(crate) struct Parsed {
    pub node: Node,
    pub groups: usize,
    /// Named groups in pattern order.
    pub names: Vec<(Vec<u8>, usize)>,
    pub utf: bool,
    pub ucp: bool,
    pub newline: Newline,
    pub bsr_anycrlf: bool,
    pub match_limit: Option<u64>,
    pub depth_limit: Option<u64>,
    pub not_empty: bool,
    pub not_empty_atstart: bool,
}

/// What counts as a newline for `^`, `$` and `.`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Newline {
    Lf,
    Cr,
    CrLf,
    AnyCrLf,
    Any,
    Nul,
}

struct Parser<'p> {
    pat: &'p [u8],
    pos: usize,
    utf: bool,
    ucp: bool,
    flags: Flags,
    groups: usize,
    names: Vec<(Vec<u8>, usize)>,
    depth: usize,
    /// Inside a lookaround (where `\K` is refused).
    in_look: usize,
    dollar_endonly: bool,
    /// Inside `\Q…\E`.
    quoting: bool,
    /// Every group reference with the offset PCRE2 reports it at, checked
    /// once all groups are known.
    refs: Vec<(Ref, usize)>,
    /// An error PCRE2 finds only after parsing (so after bad references).
    late: Option<CompileError>,
}

fn err(code: u32, offset: usize) -> CompileError {
    CompileError::new(code, offset)
}

pub(crate) fn parse(pat: &[u8], opts: &Options) -> Result<Parsed, CompileError> {
    let mut p = Parser {
        pat,
        pos: 0,
        utf: opts.utf,
        ucp: opts.ucp,
        flags: Flags {
            caseless: opts.caseless,
            multiline: opts.multiline,
            dotall: opts.dotall,
            extended: opts.extended,
            extended_more: false,
            no_auto_capture: opts.no_auto_capture,
            ungreedy: opts.ungreedy,
            dupnames: opts.dupnames,
            restrict: opts.caseless_restrict,
        },
        groups: 0,
        names: Vec::new(),
        depth: 0,
        in_look: 0,
        dollar_endonly: opts.dollar_endonly,
        quoting: false,
        refs: Vec::new(),
        late: None,
    };
    let mut newline = Newline::Lf;
    let mut bsr_anycrlf = false;
    let (mut match_limit, mut depth_limit) = (None, None);
    let (mut not_empty, mut not_empty_atstart) = (false, false);
    // Leading option settings: (*UTF), (*CRLF), (*LIMIT_MATCH=n) and so on.
    while p.pat[p.pos..].starts_with(b"(*") {
        let rest = &p.pat[p.pos + 2..];
        let Some(close) = rest.iter().position(|&b| b == b')') else { break };
        let word = &rest[..close];
        let limit = |prefix: &[u8]| -> Option<u64> {
            word.strip_prefix(prefix).and_then(|d| std::str::from_utf8(d).ok()).and_then(|d| d.parse().ok())
        };
        match word {
            b"UTF" | b"UTF8" => p.utf = true,
            b"UCP" => p.ucp = true,
            b"CR" => newline = Newline::Cr,
            b"LF" => newline = Newline::Lf,
            b"CRLF" => newline = Newline::CrLf,
            b"ANYCRLF" => newline = Newline::AnyCrLf,
            b"ANY" => newline = Newline::Any,
            b"NUL" => newline = Newline::Nul,
            b"BSR_ANYCRLF" => bsr_anycrlf = true,
            b"BSR_UNICODE" => bsr_anycrlf = false,
            b"NO_AUTO_POSSESS" | b"NO_START_OPT" | b"NO_DOTSTAR_ANCHOR" | b"NO_JIT" => {}
            b"NOTEMPTY" => not_empty = true,
            b"NOTEMPTY_ATSTART" => not_empty_atstart = true,
            _ => {
                if let Some(n) = limit(b"LIMIT_MATCH=") {
                    match_limit = Some(n);
                } else if let Some(n) = limit(b"LIMIT_DEPTH=").or_else(|| limit(b"LIMIT_RECURSION=")) {
                    depth_limit = Some(n);
                } else if limit(b"LIMIT_HEAP=").is_some() {
                } else {
                    break;
                }
            }
        }
        p.pos += close + 3;
    }
    if p.utf
        && let Err((code, at)) = crate::check_utf8(pat)
    {
        return Err(CompileError::utf(code, at));
    }
    let node = p.alternation()?;
    if p.pos < p.pat.len() {
        // Only a stray `)` stops the top-level alternation.
        return Err(err(122, p.pos));
    }
    for (r, at) in &p.refs {
        let exists = match r {
            Ref::Num(n) => *n <= p.groups,
            Ref::Name(name) => p.names.iter().any(|(m, _)| m == name),
        };
        if !exists {
            return Err(err(115, *at));
        }
    }
    if let Some(e) = p.late {
        return Err(e);
    }
    Ok(Parsed {
        node,
        groups: p.groups,
        names: p.names,
        utf: p.utf,
        ucp: p.ucp,
        newline,
        bsr_anycrlf,
        match_limit,
        depth_limit,
        not_empty,
        not_empty_atstart,
    })
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.pat.get(self.pos).copied()
    }

    fn peek_at(&self, n: usize) -> Option<u8> {
        self.pat.get(self.pos + n).copied()
    }

    fn eat(&mut self, b: u8) -> bool {
        if self.peek() == Some(b) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn eat_str(&mut self, s: &[u8]) -> bool {
        if self.pat[self.pos..].starts_with(s) {
            self.pos += s.len();
            true
        } else {
            false
        }
    }

    /// The next pattern character: a code point in UTF mode, else a byte.
    fn next_char(&mut self) -> u32 {
        let (c, n) = crate::decode(self.pat, self.pos, self.utf);
        self.pos += n;
        c
    }

    /// Skips white space and `#` comments in extended mode.
    fn skip_extended(&mut self) {
        if !self.flags.extended {
            return;
        }
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r') => self.pos += 1,
                Some(b'#') => {
                    while let Some(b) = self.peek() {
                        self.pos += 1;
                        if b == b'\n' {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
    }

    fn alternation(&mut self) -> Result<Node, CompileError> {
        let mut branches = vec![self.sequence()?];
        while self.eat(b'|') {
            branches.push(self.sequence()?);
        }
        Ok(if branches.len() == 1 { branches.pop().unwrap() } else { Node::Alt(branches) })
    }

    /// Alternatives of a branch-reset group `(?|…)`: each starts numbering
    /// at the same group.
    fn branch_reset(&mut self) -> Result<Node, CompileError> {
        let base = self.groups;
        let mut most = base;
        let mut branches = Vec::new();
        loop {
            self.groups = base;
            branches.push(self.sequence()?);
            most = most.max(self.groups);
            if !self.eat(b'|') {
                break;
            }
        }
        self.groups = most;
        Ok(if branches.len() == 1 { branches.pop().unwrap() } else { Node::Alt(branches) })
    }

    fn sequence(&mut self) -> Result<Node, CompileError> {
        let mut items: Vec<Node> = Vec::new();
        let outer = self.flags;
        loop {
            if self.quoting {
                if self.eat_str(b"\\E") {
                    self.quoting = false;
                    // A quantifier after \E repeats the last quoted character.
                    if let Some(Node::Char { .. }) = items.last() {
                        let last = items.pop().unwrap();
                        items.push(self.quantified(last)?);
                    }
                    continue;
                }
                let start = self.pos;
                if start >= self.pat.len() {
                    break;
                }
                let c = self.next_char();
                let atom = Node::Char { c, nocase: self.flags.caseless, restrict: self.flags.restrict };
                items.push(self.quantified(atom)?);
                continue;
            }
            self.skip_extended();
            let start = self.pos;
            let Some(b) = self.peek() else { break };
            let atom = match b {
                b'|' | b')' => break,
                b'(' => {
                    self.pos += 1;
                    match self.group(start)? {
                        Some(node) => node,
                        None => continue,
                    }
                }
                b'^' => {
                    self.pos += 1;
                    Node::Assert(if self.flags.multiline { Assert::StartLine } else { Assert::Start })
                }
                b'$' => {
                    self.pos += 1;
                    Node::Assert(if self.flags.multiline {
                        Assert::EndLine
                    } else if self.dollar_endonly() {
                        Assert::End
                    } else {
                        Assert::EndOrNewline
                    })
                }
                b'.' => {
                    self.pos += 1;
                    Node::Any { nl: self.flags.dotall }
                }
                b'[' => {
                    self.pos += 1;
                    self.class(start)?
                }
                b'\\' => match self.escape(start)? {
                    Some(node) => node,
                    None => continue,
                },
                b'*' | b'+' | b'?' => return Err(err(109, self.pos)),
                b'{' if self.quantifier_ahead() => return Err(err(109, self.pos)),
                _ => {
                    let c = self.next_char();
                    Node::Char { c, nocase: self.flags.caseless, restrict: self.flags.restrict }
                }
            };
            let atom = self.quantified(atom)?;
            items.push(atom);
        }
        // An option setting like `(?i)` lasts to the end of its group,
        // including later alternatives: the caller restores it.
        let _ = outer;
        Ok(match items.len() {
            0 => Node::Empty,
            1 => items.pop().unwrap(),
            _ => Node::Concat(items),
        })
    }

    fn dollar_endonly(&self) -> bool {
        self.dollar_endonly
    }

    /// Whether a `{` here starts a valid quantifier: `{n}`, `{n,}`, `{n,m}`
    /// or `{,m}` (spaces allowed around the numbers).
    fn quantifier_ahead(&self) -> bool {
        self.read_quantifier(self.pos).is_some()
    }

    fn read_quantifier(&self, at: usize) -> Option<(Option<u32>, Option<u32>, usize)> {
        let pat = self.pat;
        let mut i = at + 1;
        let skip = |i: &mut usize| {
            while pat.get(*i).is_some_and(|&b| b == b' ' || b == b'\t') {
                *i += 1;
            }
        };
        let num = |i: &mut usize| -> Option<Option<u32>> {
            let s = *i;
            let mut v: u64 = 0;
            while let Some(d) = pat.get(*i).filter(|b| b.is_ascii_digit()) {
                v = (v * 10 + (d - b'0') as u64).min(u64::from(u32::MAX));
                *i += 1;
            }
            Some(if *i == s { None } else { Some(v as u32) })
        };
        skip(&mut i);
        let min = num(&mut i)?;
        skip(&mut i);
        let max = if pat.get(i) == Some(&b',') {
            i += 1;
            skip(&mut i);
            let m = num(&mut i)?;
            skip(&mut i);
            if min.is_none() && m.is_none() {
                return None;
            }
            m
        } else {
            min?;
            Some(min.unwrap())
        };
        if pat.get(i) != Some(&b'}') {
            return None;
        }
        Some((min, max, i + 1))
    }

    fn quantified(&mut self, mut atom: Node) -> Result<Node, CompileError> {
        loop {
            self.skip_extended();
            let (min, max) = match self.peek() {
                Some(b'*') => {
                    self.pos += 1;
                    (0, None)
                }
                Some(b'+') => {
                    self.pos += 1;
                    (1, None)
                }
                Some(b'?') => {
                    self.pos += 1;
                    (0, Some(1))
                }
                Some(b'{') => match self.read_quantifier(self.pos) {
                    Some((min, max, end)) => {
                        self.pos = end;
                        let min = min.unwrap_or(0);
                        if min > 65535 || max.is_some_and(|m| m > 65535) {
                            return Err(err(105, end - 1));
                        }
                        if max.is_some_and(|m| m < min) {
                            return Err(err(104, end - 1));
                        }
                        (min, max)
                    }
                    None => return Ok(atom),
                },
                _ => return Ok(atom),
            };
            let mut greedy = !self.flags.ungreedy;
            let mut possessive = false;
            if self.eat(b'+') {
                possessive = true;
            } else if self.eat(b'?') {
                greedy = !greedy;
            }
            if let Node::Look { .. } = atom {
                if min == 0 {
                    atom = Node::Empty;
                }
                continue;
            }
            atom = Node::Repeat { node: Box::new(atom), min, max, greedy: greedy || possessive, possessive };
        }
    }

    /// After `(`: a group, an option setting (None), or a verb.
    fn group(&mut self, start: usize) -> Result<Option<Node>, CompileError> {
        self.depth += 1;
        if self.depth > 250 {
            return Err(err(119, self.pos));
        }
        let r = self.group_inner(start);
        self.depth -= 1;
        r
    }

    fn close(&mut self, start: usize) -> Result<(), CompileError> {
        if self.eat(b')') { Ok(()) } else { Err(err(114, if self.pos >= self.pat.len() { self.pos } else { start.max(self.pos) })) }
    }

    /// A group body with the option settings inside it scoped to it.
    fn body(&mut self, start: usize) -> Result<Node, CompileError> {
        let saved = self.flags;
        let node = self.alternation();
        self.flags = saved;
        let node = node?;
        self.close(start)?;
        Ok(node)
    }

    fn look(&mut self, behind: bool, neg: bool, start: usize) -> Result<Node, CompileError> {
        self.in_look += 1;
        let body = self.body(start);
        self.in_look -= 1;
        Ok(Node::Look { behind, neg, body: Box::new(body?), at: start })
    }

    fn capture(&mut self, start: usize, name: Option<Vec<u8>>) -> Result<Node, CompileError> {
        self.groups += 1;
        let n = self.groups;
        if n > 65535 {
            return Err(err(197, self.pos));
        }
        if let Some(name) = name {
            if let Some((_, existing)) = self.names.iter().find(|(m, _)| *m == name)
                && *existing != n
                && !self.flags.dupnames
            {
                return Err(err(143, self.pos));
            }
            if self.names.iter().any(|(m, g)| *g == n && *m != name) {
                return Err(err(165, self.pos));
            }
            if !self.names.iter().any(|(m, g)| *m == name && *g == n) {
                self.names.push((name, n));
            }
        }
        let body = self.body(start)?;
        Ok(Node::Group(Box::new(body), Some(n)))
    }

    fn note(&mut self, r: &Ref, at: usize) {
        self.refs.push((r.clone(), at));
    }

    /// A reference to a named group: its name up to `term`, noted at the
    /// name's offset.
    fn name_ref(&mut self, term: u8) -> Result<Ref, CompileError> {
        let at = self.pos;
        let r = Ref::Name(self.name(term)?);
        self.note(&r, at);
        Ok(r)
    }

    /// A group name up to `term`.
    fn name(&mut self, term: u8) -> Result<Vec<u8>, CompileError> {
        let s = self.pos;
        if self.peek().is_some_and(|b| b.is_ascii_digit()) {
            return Err(err(144, self.pos));
        }
        while let Some(b) = self.peek() {
            if is_word_byte(b) || (self.utf && b >= 0x80) {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == s {
            return Err(err(162, self.pos));
        }
        if self.pos - s > 128 {
            return Err(err(148, self.pos));
        }
        let name = self.pat[s..self.pos].to_vec();
        if !self.eat(term) {
            return Err(err(142, self.pos));
        }
        Ok(name)
    }

    /// A signed group number after `(?`, `(?(` or `\g`: relative ones
    /// resolved against the groups so far.
    fn number(&mut self) -> Result<Option<usize>, CompileError> {
        let sign = match self.peek() {
            Some(b'+') => 1,
            Some(b'-') => -1,
            _ => 0,
        };
        let s = self.pos;
        if sign != 0 {
            self.pos += 1;
        }
        let ds = self.pos;
        let mut v: u64 = 0;
        while let Some(d) = self.peek().filter(|b| b.is_ascii_digit()) {
            v = (v * 10 + (d - b'0') as u64).min(1 << 20);
            self.pos += 1;
        }
        if self.pos == ds {
            if sign != 0 {
                return Err(err(129, self.pos));
            }
            self.pos = s;
            return Ok(None);
        }
        if v > 65535 {
            return Err(err(161, self.pos));
        }
        let v = v as usize;
        Ok(Some(match sign {
            1 => {
                if v == 0 {
                    return Err(err(126, self.pos));
                }
                self.groups + v
            }
            -1 => {
                if v == 0 {
                    return Err(err(126, self.pos));
                }
                if v > self.groups {
                    return Err(err(115, self.pos));
                }
                self.groups + 1 - v
            }
            _ => v,
        }))
    }

    fn verb_name(&mut self) -> Result<Option<Vec<u8>>, CompileError> {
        if !self.eat(b':') {
            return Ok(None);
        }
        let s = self.pos;
        while let Some(b) = self.peek() {
            if b == b')' {
                break;
            }
            self.pos += 1;
        }
        Ok(Some(self.pat[s..self.pos].to_vec()))
    }

    fn group_inner(&mut self, start: usize) -> Result<Option<Node>, CompileError> {
        if self.eat(b'*') {
            // (*VERB), (*VERB:NAME) or an alpha assertion (*pla:…).
            let s = self.pos;
            while self.peek().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_') {
                self.pos += 1;
            }
            let word = self.pat[s..self.pos].to_vec();
            let alpha = |w: &[u8]| -> Option<(bool, bool, bool)> {
                Some(match w {
                    b"pla" | b"positive_lookahead" | b"napla" | b"non_atomic_positive_lookahead" => (false, false, false),
                    b"nla" | b"negative_lookahead" => (false, true, false),
                    b"plb" | b"positive_lookbehind" | b"naplb" | b"non_atomic_positive_lookbehind" => (true, false, false),
                    b"nlb" | b"negative_lookbehind" => (true, true, false),
                    b"atomic" => (false, false, true),
                    b"sr" | b"script_run" | b"asr" | b"atomic_script_run" => (false, false, false),
                    _ => return None,
                })
            };
            if self.peek() == Some(b':') && word.iter().all(|b| b.is_ascii_lowercase() || *b == b'_') && !word.is_empty() {
                let Some((behind, neg, atomic)) = alpha(&word) else { return Err(err(195, self.pos)) };
                self.pos += 1;
                if atomic {
                    let body = self.body(start)?;
                    return Ok(Some(Node::Atomic(Box::new(body))));
                }
                if matches!(word.as_slice(), b"sr" | b"script_run" | b"asr" | b"atomic_script_run") {
                    let body = self.body(start)?;
                    return Ok(Some(Node::Group(Box::new(body), None)));
                }
                return self.look(behind, neg, start).map(Some);
            }
            let verb = match word.as_slice() {
                b"ACCEPT" => {
                    self.verb_name()?;
                    Verb::Accept
                }
                b"FAIL" | b"F" => {
                    self.verb_name()?;
                    Verb::Fail
                }
                b"COMMIT" => {
                    self.verb_name()?;
                    Verb::Commit
                }
                b"PRUNE" => Verb::Prune(self.verb_name()?),
                b"SKIP" => Verb::Skip(self.verb_name()?),
                b"THEN" => Verb::Then(self.verb_name()?),
                b"MARK" | b"" => match self.verb_name()? {
                    Some(name) if !name.is_empty() => Verb::Mark(name),
                    _ => return Err(err(166, self.pos)),
                },
                _ => return Err(err(160, self.pos)),
            };
            if !self.eat(b')') {
                return Err(err(160, self.pos));
            }
            return Ok(Some(Node::Verb(verb)));
        }
        if !self.eat(b'?') {
            if self.flags.no_auto_capture {
                let body = self.body(start)?;
                return Ok(Some(Node::Group(Box::new(body), None)));
            }
            return self.capture(start, None).map(Some);
        }
        let Some(b) = self.peek() else { return Err(err(111, self.pos)) };
        match b {
            b'#' => {
                while let Some(b) = self.peek() {
                    self.pos += 1;
                    if b == b')' {
                        return Ok(None);
                    }
                }
                Err(err(118, self.pos))
            }
            b':' => {
                self.pos += 1;
                let body = self.body(start)?;
                Ok(Some(Node::Group(Box::new(body), None)))
            }
            b'|' => {
                self.pos += 1;
                let saved = self.flags;
                let node = self.branch_reset();
                self.flags = saved;
                let node = node?;
                self.close(start)?;
                Ok(Some(Node::Group(Box::new(node), None)))
            }
            b'>' => {
                self.pos += 1;
                let body = self.body(start)?;
                Ok(Some(Node::Atomic(Box::new(body))))
            }
            b'=' => {
                self.pos += 1;
                self.look(false, false, start).map(Some)
            }
            b'!' => {
                self.pos += 1;
                self.look(false, true, start).map(Some)
            }
            b'*' => {
                // (?*…): a non-atomic positive lookahead.
                self.pos += 1;
                self.look(false, false, start).map(Some)
            }
            b'<' if matches!(self.peek_at(1), Some(b'=' | b'!' | b'*')) => {
                let neg = self.peek_at(1) == Some(b'!');
                self.pos += 2;
                self.look(true, neg, start).map(Some)
            }
            b'<' => {
                self.pos += 1;
                let name = self.name(b'>')?;
                self.capture(start, Some(name)).map(Some)
            }
            b'\'' => {
                self.pos += 1;
                let name = self.name(b'\'')?;
                self.capture(start, Some(name)).map(Some)
            }
            b'P' => {
                self.pos += 1;
                match self.peek() {
                    Some(b'<') => {
                        self.pos += 1;
                        let name = self.name(b'>')?;
                        self.capture(start, Some(name)).map(Some)
                    }
                    Some(b'=') => {
                        self.pos += 1;
                        let to = self.name_ref(b')')?;
                        Ok(Some(Node::BackRef { to, nocase: self.flags.caseless }))
                    }
                    Some(b'>') => {
                        self.pos += 1;
                        let to = self.name_ref(b')')?;
                        Ok(Some(Node::Call(to)))
                    }
                    _ => Err(err(141, self.pos)),
                }
            }
            b'&' => {
                self.pos += 1;
                let to = self.name_ref(b')')?;
                Ok(Some(Node::Call(to)))
            }
            b'R' => {
                self.pos += 1;
                if !self.eat(b')') {
                    return Err(err(158, self.pos));
                }
                Ok(Some(Node::Call(Ref::Num(0))))
            }
            b'0'..=b'9' | b'+' | b'-' if b != b'-' || self.peek_at(1).is_some_and(|d| d.is_ascii_digit()) => {
                let n = self.number()?.ok_or_else(|| err(129, self.pos))?;
                let at = self.pos;
                if !self.eat(b')') {
                    return Err(err(115, self.pos));
                }
                self.note(&Ref::Num(n), at);
                Ok(Some(Node::Call(Ref::Num(n))))
            }
            b'(' => self.conditional(start).map(Some),
            b'C' => {
                // A callout: p8 has no callout function, so it matches empty.
                self.pos += 1;
                match self.peek() {
                    Some(b'0'..=b'9') => {
                        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
                            self.pos += 1;
                        }
                    }
                    Some(d @ (b'`' | b'\'' | b'"' | b'^' | b'%' | b'#' | b'$' | b'{')) => {
                        let end = if d == b'{' { b'}' } else { d };
                        let open = self.pos;
                        self.pos += 1;
                        loop {
                            let Some(b) = self.peek() else { return Err(err(181, open)) };
                            self.pos += 1;
                            if b == end {
                                if self.peek() == Some(end) {
                                    self.pos += 1;
                                    continue;
                                }
                                break;
                            }
                        }
                    }
                    _ => {}
                }
                if !self.eat(b')') {
                    return Err(err(139, self.pos));
                }
                Ok(None)
            }
            _ => self.options(start),
        }
    }

    /// `(?imnsxUJ-imnsx)` or `(?imnsx:…)`, `(?^…)`.
    fn options(&mut self, start: usize) -> Result<Option<Node>, CompileError> {
        let mut flags = self.flags;
        let mut on = true;
        if self.eat(b'^') {
            flags.caseless = false;
            flags.multiline = false;
            flags.dotall = false;
            flags.extended = false;
            flags.extended_more = false;
            flags.no_auto_capture = false;
            flags.ungreedy = false;
            flags.restrict = false;
        }
        loop {
            let Some(b) = self.peek() else { return Err(err(111, self.pos)) };
            self.pos += 1;
            match b {
                b'-' => {
                    if !on {
                        return Err(err(194, self.pos - 1));
                    }
                    on = false;
                }
                b'i' => flags.caseless = on,
                b'm' => flags.multiline = on,
                b's' => flags.dotall = on,
                b'n' => flags.no_auto_capture = on,
                b'U' => flags.ungreedy = on,
                b'J' => flags.dupnames = on,
                b'r' => flags.restrict = on,
                b'x' => {
                    if on && self.peek() == Some(b'x') {
                        self.pos += 1;
                        flags.extended_more = true;
                    } else if !on {
                        flags.extended_more = false;
                    }
                    flags.extended = on;
                }
                b')' => {
                    self.flags = flags;
                    return Ok(None);
                }
                b':' => {
                    let saved = self.flags;
                    self.flags = flags;
                    let node = self.alternation();
                    self.flags = saved;
                    let node = node?;
                    self.close(start)?;
                    return Ok(Some(Node::Group(Box::new(node), None)));
                }
                _ => return Err(err(111, self.pos - 1)),
            }
        }
    }

    /// After `(?(`.
    fn conditional(&mut self, start: usize) -> Result<Node, CompileError> {
        self.pos += 1;
        let cond_at = self.pos;
        let cond = if self.peek() == Some(b'?') || self.peek() == Some(b'*') {
            // An assertion condition.
            let alpha = self.peek() == Some(b'*');
            self.pos += 1;
            let (behind, neg) = if alpha {
                let s = self.pos;
                while self.peek().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_') {
                    self.pos += 1;
                }
                let r = match &self.pat[s..self.pos] {
                    b"pla" | b"positive_lookahead" => (false, false),
                    b"nla" | b"negative_lookahead" => (false, true),
                    b"plb" | b"positive_lookbehind" => (true, false),
                    b"nlb" | b"negative_lookbehind" => (true, true),
                    _ => return Err(err(128, cond_at)),
                };
                if !self.eat(b':') {
                    return Err(err(128, cond_at));
                }
                r
            } else if self.eat(b'=') {
                (false, false)
            } else if self.eat(b'!') {
                (false, true)
            } else if self.eat_str(b"<=") {
                (true, false)
            } else if self.eat_str(b"<!") {
                (true, true)
            } else if self.peek() == Some(b'C') {
                // A callout condition: (?(?C…)(?=…)…). p8 has no callouts,
                // so the assertion that must follow is the condition.
                while self.peek().is_some_and(|b| b != b')') {
                    self.pos += 1;
                }
                if !self.eat(b')') {
                    return Err(err(139, self.pos));
                }
                if !self.eat_str(b"(?") {
                    return Err(err(128, self.pos));
                }
                if self.eat(b'=') {
                    (false, false)
                } else if self.eat(b'!') {
                    (false, true)
                } else if self.eat_str(b"<=") {
                    (true, false)
                } else if self.eat_str(b"<!") {
                    (true, true)
                } else {
                    return Err(err(128, self.pos - 2));
                }
            } else {
                return Err(err(128, self.pos));
            };
            self.in_look += 1;
            let body = self.body(cond_at);
            self.in_look -= 1;
            Cond::Look { behind, neg, body: Box::new(body?), at: start }
        } else {
            let paren = cond_at - 1;
            let c = if self.eat(b'<') {
                Cond::Group(self.name_ref(b'>')?)
            } else if self.eat(b'\'') {
                Cond::Group(self.name_ref(b'\'')?)
            } else if self.eat_str(b"R&") {
                Cond::Recursion(Some(self.name_ref(b')')?))
            } else if self.peek() == Some(b'R') && self.peek_at(1).is_some_and(|b| b == b')' || b.is_ascii_digit()) {
                self.pos += 1;
                match self.number()? {
                    Some(n) => Cond::Recursion(Some(Ref::Num(n))),
                    None => Cond::Recursion(None),
                }
            } else if self.eat_str(b"DEFINE)") {
                self.pos -= 1;
                Cond::Define
            } else if self.eat_str(b"VERSION") {
                let ge = self.eat(b'>');
                if !self.eat(b'=') {
                    return Err(err(179, self.pos));
                }
                let s = self.pos;
                while self.peek().is_some_and(|b| b.is_ascii_digit() || b == b'.') {
                    self.pos += 1;
                }
                let v: f64 = std::str::from_utf8(&self.pat[s..self.pos]).ok().and_then(|v| v.parse().ok()).ok_or_else(|| err(179, self.pos))?;
                let ours = 10.44;
                Cond::Always(if ge { ours >= v } else { (ours - v).abs() < 1e-9 })
            } else if let Some(n) = self.number()? {
                self.note(&Ref::Num(n), paren);
                Cond::Group(Ref::Num(n))
            } else if self.peek().is_some_and(is_word_byte) {
                let s = self.pos;
                while self.peek().is_some_and(is_word_byte) {
                    self.pos += 1;
                }
                let r = Ref::Name(self.pat[s..self.pos].to_vec());
                self.note(&r, s);
                Cond::Group(r)
            } else {
                return Err(err(128, self.pos));
            };
            if !self.eat(b')') {
                return Err(err(124, self.pos));
            }
            c
        };
        let saved = self.flags;
        let mut branches = vec![self.sequence()];
        while branches.last().is_some_and(|b| b.is_ok()) && self.eat(b'|') {
            branches.push(self.sequence());
        }
        self.flags = saved;
        let mut branches = branches.into_iter().collect::<Result<Vec<_>, _>>()?;
        if branches.len() > 2 && self.late.is_none() {
            self.late = Some(err(127, self.pos));
        }
        if matches!(cond, Cond::Define) && branches.len() > 1 && self.late.is_none() {
            self.late = Some(err(154, self.pos));
        }
        self.close(start)?;
        let no = if branches.len() == 2 { branches.pop().map(Box::new) } else { None };
        let yes = Box::new(branches.pop().unwrap());
        Ok(Node::Cond { cond, yes, no })
    }

    /// After `\` outside a class: a node, or None for `\Q`, `\E`.
    fn escape(&mut self, start: usize) -> Result<Option<Node>, CompileError> {
        self.pos += 1;
        let Some(b) = self.peek() else { return Err(err(101, self.pos)) };
        let ucp = self.ucp;
        let nocase = self.flags.caseless;
        let restrict = self.flags.restrict;
        let set = |item: Item, _: &Self| Node::Set(Box::new(Set { items: vec![item], negated: false, nocase: false, restrict: false }));
        let node = match b {
            b'Q' => {
                // Literal characters up to \E, each its own atom.
                self.pos += 1;
                self.quoting = true;
                return Ok(None);
            }
            b'E' => {
                self.pos += 1;
                return Ok(None);
            }
            b'd' | b'D' | b'w' | b'W' | b's' | b'S' | b'h' | b'H' | b'v' | b'V' => {
                self.pos += 1;
                set(self.class_escape(b), self)
            }
            b'p' | b'P' => {
                self.pos += 1;
                let item = self.property(b == b'P')?;
                set(item, self)
            }
            b'b' | b'B' => {
                self.pos += 1;
                Node::Assert(if b == b'b' { Assert::WordBoundary { ucp } } else { Assert::NotWordBoundary { ucp } })
            }
            b'A' => {
                self.pos += 1;
                Node::Assert(Assert::Start)
            }
            b'z' => {
                self.pos += 1;
                Node::Assert(Assert::End)
            }
            b'Z' => {
                self.pos += 1;
                Node::Assert(Assert::EndOrNewline)
            }
            b'G' => {
                self.pos += 1;
                Node::Assert(Assert::StartOffset)
            }
            b'K' => {
                self.pos += 1;
                Node::Keep
            }
            b'R' => {
                self.pos += 1;
                Node::Newline
            }
            b'X' => {
                self.pos += 1;
                Node::Grapheme
            }
            b'C' => {
                self.pos += 1;
                Node::Byte
            }
            b'N' if self.peek_at(1) != Some(b'{') || self.read_quantifier(self.pos + 1).is_some() => {
                self.pos += 1;
                Node::Any { nl: false }
            }
            b'N' if self.pat[self.pos..].starts_with(b"N{U+") => {
                if !self.utf {
                    return Err(err(193, self.pos + 1));
                }
                self.pos += 4;
                let c = self.hex_digits(b'}')?;
                Node::Char { c, nocase, restrict }
            }
            b'g' => {
                self.pos += 1;
                self.g_escape(start)?
            }
            b'k' => {
                self.pos += 1;
                let term = match self.peek() {
                    Some(b'<') => b'>',
                    Some(b'\'') => b'\'',
                    Some(b'{') => b'}',
                    _ => return Err(err(169, self.pos)),
                };
                self.pos += 1;
                let to = self.name_ref(term)?;
                Node::BackRef { to, nocase }
            }
            b'1'..=b'9' => {
                // A back reference unless it reads as octal (\12 with fewer
                // than 12 groups so far, and octal digits).
                let s = self.pos;
                let mut v: usize = 0;
                while let Some(d) = self.peek().filter(|b| b.is_ascii_digit()) {
                    v = (v * 10 + (d - b'0') as usize).min(1 << 20);
                    self.pos += 1;
                }
                if v < 10 || b >= b'8' || v <= self.groups {
                    if v > 65535 {
                        return Err(err(161, self.pos));
                    }
                    self.note(&Ref::Num(v), s);
                    Node::BackRef { to: Ref::Num(v), nocase }
                } else {
                    self.pos = s;
                    let c = self.char_escape(false)?;
                    Node::Char { c, nocase, restrict }
                }
            }
            _ => {
                let c = self.char_escape(false)?;
                Node::Char { c, nocase, restrict }
            }
        };
        Ok(Some(node))
    }

    fn g_escape(&mut self, _start: usize) -> Result<Node, CompileError> {
        let nocase = self.flags.caseless;
        let at = self.pos;
        // A bad relative number reads as an error where the reference starts.
        let number = |p: &mut Self| p.number().map_err(|e| if e.code == 115 { err(115, at) } else { e });
        match self.peek() {
            Some(b'{') => {
                self.pos += 1;
                if let Some(n) = number(self)? {
                    if !self.eat(b'}') {
                        return Err(err(157, self.pos));
                    }
                    self.note(&Ref::Num(n), at);
                    Ok(Node::BackRef { to: Ref::Num(n), nocase })
                } else {
                    let to = self.name_ref(b'}')?;
                    Ok(Node::BackRef { to, nocase })
                }
            }
            Some(q @ (b'<' | b'\'')) => {
                // \g<n> / \g'name': a subroutine call (Oniguruma syntax).
                let term = if q == b'<' { b'>' } else { b'\'' };
                self.pos += 1;
                if let Some(n) = number(self)? {
                    if !self.eat(term) {
                        return Err(err(157, self.pos));
                    }
                    self.note(&Ref::Num(n), at);
                    Ok(Node::Call(Ref::Num(n)))
                } else {
                    let to = self.name_ref(term)?;
                    Ok(Node::Call(to))
                }
            }
            _ => match number(self)? {
                Some(n) => {
                    self.note(&Ref::Num(n), at);
                    Ok(Node::BackRef { to: Ref::Num(n), nocase })
                }
                None => Err(err(157, self.pos)),
            },
        }
    }

    /// `\d`, `\w` and the rest as a set item, Unicode-aware under UCP.
    fn class_escape(&self, b: u8) -> Item {
        let neg = b.is_ascii_uppercase();
        match (b.to_ascii_lowercase(), self.ucp) {
            (b'd', true) => Item::Prop { kind: unicode::PT_PC, value: unicode::ND as u16, neg },
            (b'w', true) => Item::Prop { kind: unicode::PT_WORD, value: 0, neg },
            (b's', true) => Item::Prop { kind: unicode::PT_SPACE, value: 0, neg },
            (b'd', false) => Item::Digit(neg),
            (b'w', false) => Item::Word(neg),
            (b's', false) => Item::Space(neg),
            (b'h', _) => Item::HSpace(neg),
            _ => Item::VSpace(neg),
        }
    }

    /// After `\p` or `\P`.
    fn property(&mut self, mut neg: bool) -> Result<Item, CompileError> {
        let name: Vec<u8> = if self.eat(b'{') {
            if self.eat(b'^') {
                neg = !neg;
            }
            let s = self.pos;
            while self.peek().is_some_and(|b| b != b'}') {
                self.pos += 1;
            }
            if !self.eat(b'}') {
                return Err(err(146, self.pos));
            }
            self.pat[s..self.pos - 1].to_vec()
        } else {
            match self.peek() {
                Some(b) if b != b'{' => {
                    self.pos += 1;
                    vec![b]
                }
                _ => return Err(err(146, self.pos)),
            }
        };
        let (kind, value) = unicode::property(&name).ok_or_else(|| err(147, self.pos))?;
        Ok(Item::Prop { kind, value, neg })
    }

    fn hex_digits(&mut self, term: u8) -> Result<u32, CompileError> {
        let s = self.pos;
        let mut v: u64 = 0;
        while let Some(d) = self.peek().and_then(|b| (b as char).to_digit(16)) {
            v = (v << 4 | d as u64).min(1 << 40);
            self.pos += 1;
        }
        if self.pos == s {
            return Err(err(178, self.pos));
        }
        if !self.eat(term) {
            return Err(err(167, self.pos));
        }
        self.code_point(v)
    }

    fn code_point(&self, v: u64) -> Result<u32, CompileError> {
        if (self.utf && v > 0x10ffff) || (!self.utf && v > 0xff) {
            return Err(err(134, self.pos.saturating_sub(1)));
        }
        if self.utf && (0xd800..=0xdfff).contains(&v) {
            return Err(err(173, self.pos.saturating_sub(1)));
        }
        Ok(v as u32)
    }

    /// A character escape after `\` (the pointer at the letter): `\n`,
    /// `\x41`, `\101`, `\cA`, `\o{…}`, or a literal punctuation character.
    fn char_escape(&mut self, in_class: bool) -> Result<u32, CompileError> {
        let Some(b) = self.peek() else { return Err(err(101, self.pos)) };
        self.pos += 1;
        Ok(match b {
            b'a' => 7,
            b'e' => 27,
            b'f' => 12,
            b'n' => 10,
            b'r' => 13,
            b't' => 9,
            b'b' if in_class => 8,
            b'0'..=b'7' => {
                // Up to three octal digits in all.
                let mut v = (b - b'0') as u64;
                for _ in 0..2 {
                    match self.peek() {
                        Some(d @ b'0'..=b'7') => {
                            v = v * 8 + (d - b'0') as u64;
                            self.pos += 1;
                        }
                        _ => break,
                    }
                }
                if !self.utf && v > 0xff {
                    return Err(err(151, self.pos));
                }
                v as u32
            }
            b'o' => {
                if !self.eat(b'{') {
                    return Err(err(155, self.pos));
                }
                let s = self.pos;
                let mut v: u64 = 0;
                while let Some(d @ b'0'..=b'7') = self.peek() {
                    v = (v * 8 + (d - b'0') as u64).min(1 << 40);
                    self.pos += 1;
                }
                if self.pos == s {
                    return Err(err(178, self.pos));
                }
                if !self.eat(b'}') {
                    return Err(err(164, self.pos));
                }
                self.code_point(v)?
            }
            b'x' => {
                if self.eat(b'{') {
                    self.hex_digits(b'}')?
                } else {
                    let mut v = 0u32;
                    for _ in 0..2 {
                        match self.peek().and_then(|b| (b as char).to_digit(16)) {
                            Some(d) => {
                                v = v << 4 | d;
                                self.pos += 1;
                            }
                            None => break,
                        }
                    }
                    v
                }
            }
            b'c' => {
                let Some(c) = self.peek() else { return Err(err(102, self.pos)) };
                if !(32..127).contains(&c) {
                    return Err(err(168, self.pos));
                }
                self.pos += 1;
                (c.to_ascii_uppercase() ^ 0x40) as u32
            }
            b'F' | b'L' | b'l' | b'U' | b'u' => return Err(err(137, self.pos)),
            b'N' => return Err(err(if in_class { 171 } else { 137 }, self.pos)),
            b'8' | b'9' if in_class => return Err(err(107, self.pos)),
            // (PCRE2 reports an unknown escape at its letter.)
            _ if b.is_ascii_alphanumeric() => {
                return Err(if in_class { err(107, self.pos) } else { err(103, self.pos - 1) });
            }
            _ => {
                self.pos -= 1;
                self.next_char()
            }
        })
    }

    /// After `[`.
    fn class(&mut self, start: usize) -> Result<Node, CompileError> {
        let mut negated = false;
        if self.eat(b'^') {
            negated = true;
        }
        // POSIX `[[:alpha:]]` outside a class is an error; inside it's fine.
        let mut items: Vec<Item> = Vec::new();
        let mut first = true;
        loop {
            if self.flags.extended_more {
                while matches!(self.peek(), Some(b' ' | b'\t')) {
                    self.pos += 1;
                }
            }
            let Some(b) = self.peek() else { return Err(err(106, self.pos)) };
            if b == b']' && !first && !self.quoting {
                self.pos += 1;
                break;
            }
            first = false;
            // A POSIX class.
            if b == b'[' && !self.quoting && matches!(self.peek_at(1), Some(b':' | b'.' | b'=')) {
                let kind = self.peek_at(1).unwrap();
                let s = self.pos + 2;
                let neg = self.pat.get(s) == Some(&b'^');
                let ns = if neg { s + 1 } else { s };
                let e = (ns..self.pat.len()).find(|&i| !self.pat[i].is_ascii_alphabetic()).unwrap_or(self.pat.len());
                if self.pat.get(e) == Some(&kind) && self.pat.get(e + 1) == Some(&b']') {
                    if kind != b':' {
                        return Err(err(113, self.pos));
                    }
                    let name = &self.pat[ns..e];
                    let posix = match name {
                        b"alpha" => Posix::Alpha,
                        b"digit" => Posix::Digit,
                        b"alnum" => Posix::Alnum,
                        b"space" => Posix::Space,
                        b"blank" => Posix::Blank,
                        b"cntrl" => Posix::Cntrl,
                        b"graph" => Posix::Graph,
                        b"print" => Posix::Print,
                        b"punct" => Posix::Punct,
                        b"upper" => Posix::Upper,
                        b"lower" => Posix::Lower,
                        b"xdigit" => Posix::Xdigit,
                        b"word" => Posix::Word,
                        b"ascii" => Posix::Ascii,
                        _ => return Err(err(130, self.pos + 2)),
                    };
                    self.pos = e + 2;
                    items.push(self.posix_item(posix, neg));
                    continue;
                }
            }
            let at = self.pos;
            let lo = match self.class_atom()? {
                ClassAtom::Char(c) => c,
                ClassAtom::Item(item) => {
                    // `[\d-z]`: a class can't start a range.
                    if !self.quoting && self.peek() == Some(b'-') && self.peek_at(1).is_some_and(|b| b != b']') {
                        return Err(err(150, self.pos));
                    }
                    items.push(item);
                    continue;
                }
            };
            // A range?
            if !self.quoting && self.peek() == Some(b'-') && self.peek_at(1).is_some_and(|b| b != b']') {
                let save = self.pos;
                self.pos += 1;
                if self.peek() == Some(b'[') && matches!(self.peek_at(1), Some(b':' | b'.' | b'=')) {
                    return Err(err(150, self.pos));
                }
                match self.class_atom()? {
                    ClassAtom::Char(hi) => {
                        if hi < lo {
                            return Err(err(108, self.pos - 1));
                        }
                        items.push(Item::Range(lo, hi));
                    }
                    ClassAtom::Item(item) => {
                        // `[a-\d]`: PCRE2 refuses it.
                        let _ = (item, save, at);
                        return Err(err(150, self.pos - 1));
                    }
                }
                continue;
            }
            items.push(Item::Range(lo, lo));
        }
        let _ = start;
        Ok(Node::Set(Box::new(Set { items, negated, nocase: self.flags.caseless, restrict: self.flags.restrict })))
    }

    fn posix_item(&self, posix: Posix, neg: bool) -> Item {
        if self.ucp {
            let prop = |kind: u8, value: u16| Item::Prop { kind, value, neg };
            match posix {
                Posix::Alpha => return prop(unicode::PT_GC, unicode::GC_L as u16),
                Posix::Lower => return prop(unicode::PT_PC, unicode::LL as u16),
                Posix::Upper => return prop(unicode::PT_PC, unicode::LU as u16),
                Posix::Alnum => return prop(unicode::PT_ALNUM, 0),
                Posix::Space => return prop(unicode::PT_PXSPACE, 0),
                Posix::Word => return prop(unicode::PT_WORD, 0),
                Posix::Digit => return prop(unicode::PT_PC, unicode::ND as u16),
                Posix::Graph => return prop(unicode::PT_PXGRAPH, 0),
                Posix::Print => return prop(unicode::PT_PXPRINT, 0),
                Posix::Punct => return prop(unicode::PT_PXPUNCT, 0),
                _ => {}
            }
        }
        Item::Posix(posix, neg)
    }

    fn class_atom(&mut self) -> Result<ClassAtom, CompileError> {
        loop {
            if self.quoting {
                if self.eat_str(b"\\E") {
                    self.quoting = false;
                    continue;
                }
                return Ok(ClassAtom::Char(self.next_char()));
            }
            if self.eat_str(b"\\Q") {
                self.quoting = true;
                continue;
            }
            if self.eat_str(b"\\E") {
                continue;
            }
            break;
        }
        match self.peek() {
            None => return Err(err(106, self.pos)),
            // An empty \Q\E before the `]` ends the class.
            Some(b']') if self.pos > 0 && self.pat[self.pos - 1] == b'E' => return Ok(ClassAtom::Item(Item::Range(1, 0))),
            Some(b'\\') => {}
            Some(_) => return Ok(ClassAtom::Char(self.next_char())),
        }
        self.pos += 1;
        let Some(b) = self.peek() else { return Err(err(101, self.pos)) };
        match b {
            b'd' | b'D' | b'w' | b'W' | b's' | b'S' | b'h' | b'H' | b'v' | b'V' => {
                self.pos += 1;
                Ok(ClassAtom::Item(self.class_escape(b)))
            }
            b'p' | b'P' => {
                self.pos += 1;
                Ok(ClassAtom::Item(self.property(b == b'P')?))
            }
            b'b' => {
                self.pos += 1;
                Ok(ClassAtom::Char(8))
            }
            b'B' | b'R' | b'X' => Err(err(107, self.pos + 1)),
            _ => self.char_escape(true).map(ClassAtom::Char),
        }
    }
}

enum ClassAtom {
    Char(u32),
    Item(Item),
}

