//! The tree into a program for the backtracking machine (exec.rs).

use crate::parse::{Assert, Cond, Item, Newline, Node, Parsed, Ref, Set, Verb};
use crate::unicode;
use crate::CompileError;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Atom {
    Char(u32),
    CharNc(u32),
    /// Caseless, ASCII and non-ASCII apart.
    CharNcR(u32),
    Any,
    AnyNoNl,
    Byte,
    Set(u32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RepeatKind {
    Greedy,
    Lazy,
    Possessive,
}

#[derive(Clone, Debug)]
pub(crate) enum Inst {
    Match,
    Char(u32),
    CharNc(u32),
    CharNcR(u32),
    Any,
    AnyNoNl,
    Byte,
    Set(u32),
    Assert(Assert),
    /// Try the first target, then the second. `alt` marks an alternation
    /// (where `(*THEN)` backtracks to).
    Split { first: u32, second: u32, alt: bool },
    Jmp(u32),
    Save(u32),
    /// regs[n] = the position, so a loop can tell an empty iteration.
    Mark(u32),
    /// Loop back unless the iteration since `Mark(reg)` matched nothing.
    LoopCheck { reg: u32, exit: u32, back: u32 },
    /// A single-character item repeated: max u32::MAX for unbounded.
    Repeat { atom: Atom, min: u32, max: u32, kind: RepeatKind },
    /// A lookaround: its body follows and ends in SubEnd; `next` after it.
    Look { behind: bool, neg: bool, next: u32, min: u32, max: u32 },
    /// An atomic group: its body follows and ends in SubEnd.
    Atomic { next: u32 },
    SubEnd,
    /// The end of a lookbehind body: it must end where the assertion is.
    SubEndAt,
    BackRef { groups: Box<[u32]>, nocase: bool },
    Call { group: u32, target: u32 },
    GroupEnd(u32),
    CondGroup { groups: Box<[u32]>, no: u32 },
    CondRecursion { group: Option<u32>, no: u32 },
    /// An assertion condition: its body follows, ending in SubEnd/SubEndAt;
    /// the yes branch at `yes`, the no branch at `no`.
    CondLook { behind: bool, neg: bool, min: u32, max: u32, yes: u32, no: u32 },
    Keep,
    Newline,
    Grapheme,
    Fail,
    Accept,
    Commit,
    Prune(Option<u32>),
    Skip(Option<u32>),
    Then(Option<u32>),
    SetMark(u32),
}

/// A character class with its answers for the first 256 characters.
pub(crate) struct ClassSet {
    pub set: Set,
    pub low: [u64; 4],
}

impl ClassSet {
    fn new(set: Set, ucp_case: bool) -> ClassSet {
        let mut low = [0u64; 4];
        let mut cs = ClassSet { set, low };
        for c in 0..256u32 {
            if cs.test_items(c, ucp_case) != cs.set.negated {
                low[c as usize / 64] |= 1 << (c % 64);
            }
        }
        cs.low = low;
        cs
    }

    pub(crate) fn matches(&self, c: u32, utf: bool) -> bool {
        if c < 256 {
            return self.low[c as usize / 64] & (1 << (c % 64)) != 0;
        }
        self.test_items(c, utf) != self.set.negated
    }

    fn test_items(&self, c: u32, unicode_case: bool) -> bool {
        self.set.items.iter().any(|item| {
            if item_matches(item, c) {
                return true;
            }
            if !self.set.nocase {
                return false;
            }
            let Item::Range(..) = item else { return false };
            let restrict = self.set.restrict;
            let ok = |v: u32| v != c && (!restrict || (v < 128) == (c < 128)) && item_matches(item, v);
            if unicode_case {
                ok(unicode::other_case(c)) || unicode::case_set(c).iter().any(|&v| ok(v))
            } else if c < 128 && (c as u8).is_ascii_alphabetic() {
                item_matches(item, c ^ 0x20)
            } else {
                false
            }
        })
    }
}

fn item_matches(item: &Item, c: u32) -> bool {
    match *item {
        Item::Range(lo, hi) => lo <= c && c <= hi,
        Item::Digit(neg) => (c < 128 && (c as u8).is_ascii_digit()) != neg,
        Item::Word(neg) => (c < 128 && ((c as u8).is_ascii_alphanumeric() || c == b'_' as u32)) != neg,
        Item::Space(neg) => matches!(c, 9..=13 | 32) != neg,
        Item::HSpace(neg) => is_hspace(c) != neg,
        Item::VSpace(neg) => is_vspace(c) != neg,
        Item::Posix(p, neg) => p.test(c) != neg,
        Item::Prop { kind, value, neg } => unicode::has_property(c, kind, value) != neg,
    }
}

pub(crate) fn is_hspace(c: u32) -> bool {
    matches!(c, 0x09 | 0x20 | 0xa0 | 0x1680 | 0x180e | 0x2000..=0x200a | 0x202f | 0x205f | 0x3000)
}

pub(crate) fn is_vspace(c: u32) -> bool {
    matches!(c, 0x0a..=0x0d | 0x85 | 0x2028 | 0x2029)
}

pub(crate) struct Prog {
    pub insts: Vec<Inst>,
    pub sets: Vec<ClassSet>,
    pub marks: Vec<Vec<u8>>,
    pub slots: usize,
    pub regs: usize,
    pub utf: bool,
    pub ucp: bool,
    pub newline: Newline,
    pub bsr_anycrlf: bool,
    /// Only a match at the start offset can succeed.
    pub anchored: bool,
    /// A match can start only at the subject's start or after a newline
    /// (the pattern starts with `.*` and `.` stops at newlines).
    pub startline: bool,
    /// The bytes a match can start with, when they're known.
    pub first: First,
    pub match_limit: Option<u64>,
    pub depth_limit: Option<u64>,
    pub not_empty: bool,
    pub not_empty_atstart: bool,
    /// How far back (in characters) a match may look before its start.
    pub max_lookbehind: u32,
    /// The backtrack stack's room, kept between matches (exec.rs).
    pub stack: std::cell::Cell<Vec<crate::exec::Bt>>,
}

/// The bytes a match can start with: few enough to look for with memchr,
/// or a table of them.
pub(crate) enum First {
    Any,
    One(u8),
    Two(u8, u8),
    Three(u8, u8, u8),
    Set(Box<[bool; 256]>),
    /// The pattern starts with these bytes (two or more): memmem.
    Text(Box<memchr::memmem::Finder<'static>>),
}

impl First {
    fn of(set: Option<Box<[bool; 256]>>, prefix: Vec<u8>) -> First {
        let Some(set) = set else { return First::Any };
        if prefix.len() >= 2 {
            return First::Text(Box::new(memchr::memmem::Finder::new(&prefix).into_owned()));
        }
        let mut bytes = (0..=255u8).filter(|&b| set[b as usize]);
        match (bytes.next(), bytes.next(), bytes.next(), bytes.next()) {
            (Some(a), None, _, _) => First::One(a),
            (Some(a), Some(b), None, _) => First::Two(a, b),
            (Some(a), Some(b), Some(c), None) => First::Three(a, b, c),
            _ => First::Set(set),
        }
    }
}

struct Compiler<'a> {
    insts: Vec<Inst>,
    sets: Vec<ClassSet>,
    marks: Vec<Vec<u8>>,
    regs: u32,
    groups: usize,
    names: &'a [(Vec<u8>, usize)],
    /// Each group's first instruction, for calls (patched at the end).
    group_start: Vec<Option<u32>>,
    calls: Vec<(usize, u32)>,
    /// Group bodies, for calls to a group inside a lookbehind's width.
    utf: bool,
    ucp: bool,
    max_lookbehind: u32,
}

const MAX_INSTS: usize = 2_000_000;

fn err(code: u32, offset: usize) -> CompileError {
    CompileError::new(code, offset)
}

pub(crate) fn compile(parsed: Parsed, anchored_opt: bool, pattern_len: usize) -> Result<Prog, CompileError> {
    let mut c = Compiler {
        insts: Vec::new(),
        sets: Vec::new(),
        marks: Vec::new(),
        regs: 0,
        groups: parsed.groups,
        names: &parsed.names,
        group_start: vec![None; parsed.groups + 1],
        calls: Vec::new(),
        utf: parsed.utf,
        ucp: parsed.ucp,
        max_lookbehind: 0,
    };
    // Every reference must name a group that exists.
    check_refs(&parsed.node, &c, pattern_len)?;
    c.group_start[0] = Some(0);
    c.node(&parsed.node, pattern_len)?;
    c.emit(Inst::GroupEnd(0));
    c.emit(Inst::Match);
    for (at, group) in std::mem::take(&mut c.calls) {
        let target = c.group_start[group as usize].ok_or_else(|| err(115, pattern_len))?;
        if let Inst::Call { target: t, .. } = &mut c.insts[at] {
            *t = target;
        }
    }
    let dotstar = if names_groups(&parsed.node) { None } else { dot_star(&parsed.node) };
    let anchored = anchored_opt || starts_anchored(&parsed.node) || dotstar == Some(true);
    let startline = !anchored && dotstar == Some(false);
    let first = if anchored {
        First::Any
    } else {
        let mut prefix = Vec::new();
        literal_prefix(&parsed.node, parsed.utf, &mut prefix);
        First::of(first_bytes(&parsed.node, &c), prefix)
    };
    Ok(Prog {
        insts: c.insts,
        sets: c.sets,
        marks: c.marks,
        slots: (parsed.groups + 1) * 2,
        regs: c.regs as usize,
        utf: parsed.utf,
        ucp: parsed.ucp,
        newline: parsed.newline,
        bsr_anycrlf: parsed.bsr_anycrlf,
        anchored,
        startline,
        first,
        match_limit: parsed.match_limit,
        depth_limit: parsed.depth_limit,
        not_empty: parsed.not_empty,
        not_empty_atstart: parsed.not_empty_atstart,
        max_lookbehind: c.max_lookbehind,
        stack: Default::default(),
    })
}

fn check_refs(node: &Node, c: &Compiler, at: usize) -> Result<(), CompileError> {
    let check = |r: &Ref| -> Result<(), CompileError> {
        match r {
            Ref::Num(n) if *n > c.groups => Err(err(115, at)),
            Ref::Name(name) if !c.names.iter().any(|(m, _)| m == name) => Err(err(115, at)),
            _ => Ok(()),
        }
    };
    match node {
        Node::BackRef { to, .. } | Node::Call(to) => check(to),
        Node::Group(b, _) | Node::Atomic(b) | Node::Look { body: b, .. } | Node::Repeat { node: b, .. } => check_refs(b, c, at),
        Node::Concat(v) | Node::Alt(v) => v.iter().try_for_each(|n| check_refs(n, c, at)),
        Node::Cond { cond, yes, no } => {
            match cond {
                Cond::Group(Ref::Num(n)) if *n > c.groups => return Err(err(115, at)),
                // A name that is not a group reads as a (failing) condition
                // only when it is "R…" or "DEFINE"; otherwise it's an error.
                Cond::Group(Ref::Name(name)) if !c.names.iter().any(|(m, _)| m == name) => return Err(err(115, at)),
                Cond::Recursion(Some(r)) => check(r)?,
                Cond::Look { body, .. } => check_refs(body, c, at)?,
                _ => {}
            }
            check_refs(yes, c, at)?;
            if let Some(no) = no {
                check_refs(no, c, at)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// PCRE's dot-star optimisation: a pattern that starts with `.*` (greedy
/// or not) can match only where that `.*` could not have started further
/// left: Some(true) anchored (`.` matches newlines), Some(false) at the
/// start or after a newline. None when it doesn't start so.
fn dot_star(node: &Node) -> Option<bool> {
    match node {
        Node::Repeat { node, min: 0, max: None, .. } => match **node {
            Node::Any { nl } => Some(nl),
            _ => None,
        },
        Node::Concat(v) => v.first().and_then(dot_star),
        Node::Group(b, _) => dot_star(b),
        Node::Alt(v) => {
            let all: Option<Vec<bool>> = v.iter().map(dot_star).collect();
            all.and_then(|kinds| kinds.iter().all(|&k| k == kinds[0]).then(|| kinds[0]))
        }
        _ => None,
    }
}

/// Whether anything names a group (back reference, subroutine call,
/// condition) or can move the start (`(*PRUNE)`, `(*SKIP)`), which the
/// dot-star optimisation must not meet (PCRE leaves it off then too).
fn names_groups(node: &Node) -> bool {
    match node {
        Node::BackRef { .. } | Node::Call(_) | Node::Cond { .. } => true,
        Node::Verb(Verb::Prune(_) | Verb::Skip(_)) => true,
        Node::Group(b, _) | Node::Atomic(b) | Node::Look { body: b, .. } | Node::Repeat { node: b, .. } => names_groups(b),
        Node::Concat(v) | Node::Alt(v) => v.iter().any(names_groups),
        _ => false,
    }
}

fn starts_anchored(node: &Node) -> bool {
    match node {
        Node::Assert(Assert::Start) => true,
        Node::Concat(v) => v.first().is_some_and(starts_anchored),
        Node::Alt(v) => v.iter().all(starts_anchored),
        Node::Group(b, _) | Node::Atomic(b) => starts_anchored(b),
        _ => false,
    }
}

/// Adds the bytes every match starts with: the pattern's leading
/// case-sensitive characters (through groups); false once something else
/// comes, so nothing after it counts.
fn literal_prefix(node: &Node, utf: bool, out: &mut Vec<u8>) -> bool {
    match node {
        Node::Char { c, nocase: false, .. } => {
            if utf {
                let Some(ch) = char::from_u32(*c) else { return false };
                let mut buf = [0u8; 4];
                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            } else if *c < 256 {
                out.push(*c as u8);
            } else {
                return false;
            }
            true
        }
        Node::Group(b, _) => literal_prefix(b, utf, out),
        Node::Concat(v) => v.iter().all(|n| literal_prefix(n, utf, out)),
        _ => false,
    }
}

/// The set of bytes a match must start with, or None when any could.
fn first_bytes(node: &Node, c: &Compiler) -> Option<Box<[bool; 256]>> {
    let mut set = Box::new([false; 256]);
    match firsts(node, c, &mut set) {
        Some(false) => Some(set),
        _ => None,
    }
}

/// Adds the bytes `node` can start with; Some(true) when it can also match
/// the empty string (so what follows counts too), None when unknown.
fn firsts(node: &Node, c: &Compiler, set: &mut [bool; 256]) -> Option<bool> {
    let add_char = |set: &mut [bool; 256], ch: u32| {
        if c.utf {
            let mut buf = [0u8; 4];
            if let Some(s) = char::from_u32(ch) {
                set[s.encode_utf8(&mut buf).as_bytes()[0] as usize] = true;
            }
        } else if ch < 256 {
            set[ch as usize] = true;
        }
    };
    match node {
        Node::Empty | Node::Assert(_) | Node::Keep | Node::Look { .. } => Some(true),
        Node::Verb(Verb::Mark(_)) => Some(true),
        Node::Char { c: ch, nocase, .. } => {
            add_char(set, *ch);
            if *nocase {
                if c.utf {
                    add_char(set, unicode::other_case(*ch));
                    for &v in unicode::case_set(*ch) {
                        add_char(set, v);
                    }
                } else if *ch < 128 {
                    add_char(set, ch ^ 0x20);
                }
            }
            Some(false)
        }
        Node::Set(s) => {
            let cs = ClassSet::new((**s).clone(), c.utf);
            for b in 0..128u32 {
                if cs.matches(b, c.utf) {
                    set[b as usize] = true;
                }
            }
            for b in 128..256u32 {
                if c.utf {
                    // Any lead byte: the class may hold characters above 127.
                    if b >= 0xc0 {
                        set[b as usize] = true;
                    }
                } else if cs.matches(b, false) {
                    set[b as usize] = true;
                }
            }
            Some(false)
        }
        Node::Group(b, _) | Node::Atomic(b) => firsts(b, c, set),
        Node::Concat(v) => {
            for n in v {
                if !firsts(n, c, set)? {
                    return Some(false);
                }
            }
            Some(true)
        }
        Node::Alt(v) => {
            let mut empty = false;
            for n in v {
                empty |= firsts(n, c, set)?;
            }
            Some(empty)
        }
        Node::Repeat { node, min, .. } => {
            let empty = firsts(node, c, set)?;
            Some(empty || *min == 0)
        }
        _ => None,
    }
}

/// Whether a node can match the empty string.
fn nullable(node: &Node) -> bool {
    match node {
        Node::Char { .. } | Node::Any { .. } | Node::Byte | Node::Set(_) | Node::Newline | Node::Grapheme => false,
        Node::Concat(v) => v.iter().all(nullable),
        Node::Alt(v) => v.iter().any(nullable),
        Node::Group(b, _) | Node::Atomic(b) => nullable(b),
        Node::Repeat { node, min, .. } => *min == 0 || nullable(node),
        Node::Verb(Verb::Fail) => false,
        _ => true,
    }
}

impl Compiler<'_> {
    fn emit(&mut self, inst: Inst) -> usize {
        self.insts.push(inst);
        self.insts.len() - 1
    }

    fn pc(&self) -> u32 {
        self.insts.len() as u32
    }

    fn mark_name(&mut self, name: &[u8]) -> u32 {
        if let Some(i) = self.marks.iter().position(|m| m == name) {
            return i as u32;
        }
        self.marks.push(name.to_vec());
        (self.marks.len() - 1) as u32
    }

    fn set(&mut self, set: &Set) -> u32 {
        self.sets.push(ClassSet::new(set.clone(), self.utf));
        (self.sets.len() - 1) as u32
    }

    fn groups_of(&self, r: &Ref) -> Box<[u32]> {
        match r {
            Ref::Num(n) => Box::new([*n as u32]),
            Ref::Name(name) => self.names.iter().filter(|(m, _)| m == name).map(|(_, g)| *g as u32).collect(),
        }
    }

    fn atom(&mut self, node: &Node) -> Option<Atom> {
        Some(match node {
            Node::Char { c, nocase: false, .. } => Atom::Char(*c),
            Node::Char { c, nocase: true, restrict } => {
                if self.has_case(*c) {
                    if *restrict { Atom::CharNcR(*c) } else { Atom::CharNc(*c) }
                } else {
                    Atom::Char(*c)
                }
            }
            Node::Any { nl: true } => Atom::Any,
            Node::Any { nl: false } => Atom::AnyNoNl,
            Node::Byte => Atom::Byte,
            Node::Set(s) => Atom::Set(self.set(s)),
            _ => return None,
        })
    }

    fn has_case(&self, c: u32) -> bool {
        if self.utf || self.ucp {
            unicode::other_case(c) != c || !unicode::case_set(c).is_empty()
        } else {
            c < 128 && (c as u8).is_ascii_alphabetic()
        }
    }

    fn node(&mut self, node: &Node, at: usize) -> Result<(), CompileError> {
        if self.insts.len() > MAX_INSTS {
            return Err(err(120, at));
        }
        match node {
            Node::Empty => {}
            Node::Char { .. } | Node::Any { .. } | Node::Byte | Node::Set(_) => {
                let inst = match self.atom(node).unwrap() {
                    Atom::Char(c) => Inst::Char(c),
                    Atom::CharNc(c) => Inst::CharNc(c),
                    Atom::CharNcR(c) => Inst::CharNcR(c),
                    Atom::Any => Inst::Any,
                    Atom::AnyNoNl => Inst::AnyNoNl,
                    Atom::Byte => Inst::Byte,
                    Atom::Set(s) => Inst::Set(s),
                };
                self.emit(inst);
            }
            Node::Assert(a) => {
                if matches!(a, Assert::WordBoundary { .. } | Assert::NotWordBoundary { .. }) {
                    self.max_lookbehind = self.max_lookbehind.max(1);
                }
                self.emit(Inst::Assert(*a));
            }
            Node::Group(body, None) => self.node(body, at)?,
            Node::Group(body, Some(n)) => {
                let start = self.pc();
                if self.group_start[*n].is_none() {
                    self.group_start[*n] = Some(start);
                }
                self.emit(Inst::Save(*n as u32 * 2));
                self.node(body, at)?;
                self.emit(Inst::Save(*n as u32 * 2 + 1));
                self.emit(Inst::GroupEnd(*n as u32));
            }
            Node::Atomic(body) => {
                let head = self.emit(Inst::Atomic { next: 0 });
                self.node(body, at)?;
                self.emit(Inst::SubEnd);
                let next = self.pc();
                self.insts[head] = Inst::Atomic { next };
            }
            Node::Look { behind, neg, body, at: look_at } => {
                let (min, max) = if *behind { self.lookbehind_width(body, *look_at)? } else { (0, 0) };
                let head = self.emit(Inst::Look { behind: *behind, neg: *neg, next: 0, min, max });
                self.node(body, at)?;
                self.emit(if *behind { Inst::SubEndAt } else { Inst::SubEnd });
                let next = self.pc();
                self.insts[head] = Inst::Look { behind: *behind, neg: *neg, next, min, max };
            }
            Node::Concat(v) => {
                for n in v {
                    self.node(n, at)?;
                }
            }
            Node::Alt(v) => {
                let mut jumps = Vec::new();
                for (i, n) in v.iter().enumerate() {
                    if i + 1 < v.len() {
                        let split = self.emit(Inst::Split { first: 0, second: 0, alt: true });
                        self.node(n, at)?;
                        jumps.push(self.emit(Inst::Jmp(0)));
                        let next = self.pc();
                        self.insts[split] = Inst::Split { first: split as u32 + 1, second: next, alt: true };
                    } else {
                        self.node(n, at)?;
                    }
                }
                let end = self.pc();
                for j in jumps {
                    self.insts[j] = Inst::Jmp(end);
                }
            }
            Node::Repeat { node: inner, min, max, greedy, possessive } => {
                self.repeat(inner, *min, *max, *greedy, *possessive, at)?;
            }
            Node::BackRef { to, nocase } => {
                let groups = self.groups_of(to);
                self.emit(Inst::BackRef { groups, nocase: *nocase });
            }
            Node::Call(to) => {
                let group = self.groups_of(to)[0];
                let at = self.emit(Inst::Call { group, target: 0 });
                self.calls.push((at, group));
            }
            Node::Cond { cond, yes, no } => self.conditional(cond, yes, no.as_deref(), at)?,
            Node::Keep => {
                self.emit(Inst::Keep);
            }
            Node::Newline => {
                self.emit(Inst::Newline);
            }
            Node::Grapheme => {
                self.emit(Inst::Grapheme);
            }
            Node::Verb(v) => {
                let inst = match v {
                    Verb::Accept => Inst::Accept,
                    Verb::Fail => Inst::Fail,
                    Verb::Commit => Inst::Commit,
                    Verb::Prune(n) => Inst::Prune(n.as_deref().map(|n| self.mark_name(n))),
                    Verb::Skip(n) => Inst::Skip(n.as_deref().map(|n| self.mark_name(n))),
                    Verb::Then(n) => Inst::Then(n.as_deref().map(|n| self.mark_name(n))),
                    Verb::Mark(n) => Inst::SetMark(self.mark_name(n)),
                };
                self.emit(inst);
            }
        }
        Ok(())
    }

    fn conditional(&mut self, cond: &Cond, yes: &Node, no: Option<&Node>, at: usize) -> Result<(), CompileError> {
        let head = match cond {
            Cond::Group(r) => self.emit(Inst::CondGroup { groups: self.groups_of(r), no: 0 }),
            Cond::Recursion(r) => {
                let group = r.as_ref().map(|r| self.groups_of(r)[0]);
                self.emit(Inst::CondRecursion { group, no: 0 })
            }
            Cond::Define => {
                // Never taken: the body is there to be called.
                let j = self.emit(Inst::Jmp(0));
                self.node(yes, at)?;
                let end = self.pc();
                self.insts[j] = Inst::Jmp(end);
                return Ok(());
            }
            Cond::Always(true) => {
                self.node(yes, at)?;
                return Ok(());
            }
            Cond::Always(false) => {
                if let Some(no) = no {
                    self.node(no, at)?;
                }
                return Ok(());
            }
            Cond::Look { behind, neg, body, at: look_at } => {
                let (min, max) = if *behind { self.lookbehind_width(body, *look_at)? } else { (0, 0) };
                let head = self.emit(Inst::CondLook { behind: *behind, neg: *neg, min, max, yes: 0, no: 0 });
                self.node(body, at)?;
                self.emit(if *behind { Inst::SubEndAt } else { Inst::SubEnd });
                let yes_pc = self.pc();
                self.node(yes, at)?;
                let j = self.emit(Inst::Jmp(0));
                let no_pc = self.pc();
                if let Some(no) = no {
                    self.node(no, at)?;
                }
                let end = self.pc();
                self.insts[j] = Inst::Jmp(end);
                self.insts[head] = Inst::CondLook { behind: *behind, neg: *neg, min, max, yes: yes_pc, no: no_pc };
                return Ok(());
            }
        };
        self.node(yes, at)?;
        let j = self.emit(Inst::Jmp(0));
        let no_pc = self.pc();
        if let Some(no) = no {
            self.node(no, at)?;
        }
        let end = self.pc();
        self.insts[j] = Inst::Jmp(end);
        match &mut self.insts[head] {
            Inst::CondGroup { no, .. } | Inst::CondRecursion { no, .. } => *no = no_pc,
            _ => unreachable!(),
        }
        Ok(())
    }

    fn repeat(&mut self, inner: &Node, min: u32, max: Option<u32>, greedy: bool, possessive: bool, at: usize) -> Result<(), CompileError> {
        if max == Some(0) {
            return Ok(());
        }
        let kind = if possessive {
            RepeatKind::Possessive
        } else if greedy {
            RepeatKind::Greedy
        } else {
            RepeatKind::Lazy
        };
        if let Some(atom) = self.atom(inner) {
            self.emit(Inst::Repeat { atom, min, max: max.unwrap_or(u32::MAX), kind });
            return Ok(());
        }
        if possessive {
            let head = self.emit(Inst::Atomic { next: 0 });
            self.repeat(inner, min, max, true, false, at)?;
            self.emit(Inst::SubEnd);
            let next = self.pc();
            self.insts[head] = Inst::Atomic { next };
            return Ok(());
        }
        // An assertion or verb repeats at most once.
        if matches!(inner, Node::Assert(_) | Node::Keep | Node::Verb(_) | Node::Look { .. }) {
            if min > 0 {
                self.node(inner, at)?;
            }
            return Ok(());
        }
        for _ in 0..min {
            self.node(inner, at)?;
            if self.insts.len() > MAX_INSTS {
                return Err(err(120, at));
            }
        }
        let empty = nullable(inner);
        match max {
            None => {
                let reg = if empty {
                    self.regs += 1;
                    Some(self.regs - 1)
                } else {
                    None
                };
                let split = self.emit(Inst::Split { first: 0, second: 0, alt: false });
                if let Some(r) = reg {
                    self.emit(Inst::Mark(r));
                }
                self.node(inner, at)?;
                let check = self.emit(Inst::Jmp(split as u32));
                let exit = self.pc();
                if let Some(r) = reg {
                    self.insts[check] = Inst::LoopCheck { reg: r, exit, back: split as u32 };
                }
                let body = split as u32 + 1;
                self.insts[split] = if greedy {
                    Inst::Split { first: body, second: exit, alt: false }
                } else {
                    Inst::Split { first: exit, second: body, alt: false }
                };
            }
            Some(max) => {
                let mut splits = Vec::new();
                for _ in min..max {
                    splits.push(self.emit(Inst::Split { first: 0, second: 0, alt: false }));
                    self.node(inner, at)?;
                    if self.insts.len() > MAX_INSTS {
                        return Err(err(120, at));
                    }
                }
                let exit = self.pc();
                for s in splits {
                    let body = s as u32 + 1;
                    self.insts[s] = if greedy {
                        Inst::Split { first: body, second: exit, alt: false }
                    } else {
                        Inst::Split { first: exit, second: body, alt: false }
                    };
                }
            }
        }
        Ok(())
    }

    /// A lookbehind's length range in characters: each branch must have a
    /// bounded length, at most 255.
    fn lookbehind_width(&mut self, body: &Node, at: usize) -> Result<(u32, u32), CompileError> {
        let (min, max) = self.width(body, 0).ok_or_else(|| err(125, at))?;
        if max > 255 {
            return Err(err(187, at));
        }
        self.max_lookbehind = self.max_lookbehind.max(max);
        Ok((min, max))
    }

    fn width(&self, node: &Node, depth: u32) -> Option<(u32, u32)> {
        if depth > 50 {
            return None;
        }
        Some(match node {
            Node::Empty | Node::Assert(_) | Node::Look { .. } | Node::Keep | Node::Verb(_) => (0, 0),
            Node::Char { .. } | Node::Any { .. } | Node::Byte | Node::Set(_) => (1, 1),
            Node::Newline => (1, 2),
            Node::Group(b, _) | Node::Atomic(b) => self.width(b, depth)?,
            Node::Concat(v) => {
                let mut t = (0u32, 0u32);
                for n in v {
                    let w = self.width(n, depth)?;
                    t = (t.0.saturating_add(w.0), t.1.saturating_add(w.1));
                }
                t
            }
            Node::Alt(v) => {
                let mut t = (u32::MAX, 0u32);
                for n in v {
                    let w = self.width(n, depth)?;
                    t = (t.0.min(w.0), t.1.max(w.1));
                }
                t
            }
            Node::Repeat { node, min, max, .. } => {
                let w = self.width(node, depth)?;
                let max = (*max)?;
                (w.0.saturating_mul(*min), w.1.saturating_mul(max))
            }
            Node::Cond { yes, no, .. } => {
                let a = self.width(yes, depth)?;
                let b = match no {
                    Some(n) => self.width(n, depth)?,
                    None => (0, 0),
                };
                (a.0.min(b.0), a.1.max(b.1))
            }
            _ => return None,
        })
    }
}
