//! A lazy DFA for the regular subset of patterns.
//!
//! A pattern without back references, lookaround, atomic groups, verbs,
//! recursion or conditionals is a regular language, and a match can be
//! found in one pass over the subject instead of a backtracking run from
//! every start position. The DFA is built from the compiled program
//! (compile.rs) so that it tests characters with exactly the code the
//! machine (exec.rs) uses, and its states are made on demand as the
//! subject is read: a state is an ordered list of program positions (the
//! machine's alternatives in the order it would try them) plus what it
//! knows about the previous character, and a transition is computed the
//! first time a state meets a character and remembered.
//!
//! PCRE's leftmost-first semantics come from the order: when the closure
//! of a state reaches `Match`, every lower-priority alternative is dropped
//! (the backtracker would never reach them), while higher-priority ones go
//! on and may end the match later. The end found this way is the end the
//! backtracker finds. The leftmost start is found by running the DFA
//! anchored at the candidate starts; captures come from the backtracker,
//! run once, anchored at that start.
//!
//! What stays on the backtracker: the non-regular features above, `\C`
//! in UTF mode, CRLF newline conventions (`.` and `$` would need to see
//! two characters at once), `(*LIMIT_…)`, NOTEMPTY, and any search with
//! lowered backtracking limits (their errors are part of the semantics).
//! A DFA that outgrows its cache hands the search back to the machine.

use std::collections::HashMap;

use crate::compile::{Atom, Inst, Prog, RepeatKind};
use crate::exec::{atom_matches, is_word, newline_char};
use crate::parse::{Assert, Newline};

/// One node of the DFA's view of the program.
#[derive(Clone, Copy, Debug)]
enum Node {
    /// Consume a character matching the atom, then go on at `.1`.
    Atom(Atom, u32),
    /// Try `.0`, then `.1`.
    Split(u32, u32),
    Jmp(u32),
    /// Go on at `next` when the assertion holds (fails, when `neg`).
    Assert { a: Assert, neg: bool, next: u32 },
    Match,
}

/// The largest program the DFA takes (a bounded repeat expands).
const MAX_NODES: usize = 2000;
/// The most states a pattern's DFA may hold before it is cleared.
const MAX_STATES: usize = 512;
/// How many times a pattern's DFA may be cleared before it gives up.
const MAX_RESETS: u32 = 4;

const UNKNOWN: u32 = u32::MAX;
const MATCH_BIT: u32 = 1 << 31;
const DEAD: u32 = 0;

// State flags: what the state knows about the character before the
// current position (the closure's assertions need it).
const PREV_WORD: u8 = 1;
const PREV_NL: u8 = 2;
const AT_START: u8 = 4;
/// The current position is the search's start offset (`\G`, and the
/// NOTEMPTY_ATSTART option): only an initial state.
const AT_OFFSET: u8 = 8;
/// A new alternative starting here is tried after the others
/// (an unanchored search before any match is found).
const RESTART: u8 = 16;
/// An empty match here does not count (NOTEMPTY_ATSTART at the start
/// offset): only an initial state.
const NO_EMPTY: u8 = 32;

struct State {
    kernel: Box<[u32]>,
    flags: u8,
    /// Transitions by byte (by code point only below 0x80 in UTF mode):
    /// the next state's id, with MATCH_BIT when a match ends before the
    /// character.
    trans: Box<[u32; 256]>,
    /// Whether a match ends at the end of the subject from here.
    eof: Option<bool>,
}

pub(crate) struct Dfa {
    nodes: Vec<Node>,
    start: u32,
    /// The program has `$` outside multiline mode: the last character
    /// steps differently from the others.
    needs_last: bool,
    utf: bool,
    states: Vec<State>,
    index: HashMap<(Box<[u32]>, u8), u32>,
    /// Transitions on a non-ASCII character in UTF mode, by (state, code point).
    wide: HashMap<(u32, u32), u32>,
    /// Transitions on the subject's last character, by (state, byte).
    last: HashMap<(u32, u32), u32>,
    /// Empty states that restart at each position, by their flags.
    starts: [u32; 8],
    resets: u32,
    disabled: bool,
    // Scratch for the closure.
    visited: Vec<u32>,
    stamp: u32,
    stack: Vec<u32>,
    next: Vec<u32>,
}

/// The DFA gave up on this search (its cache overflowed): the machine
/// finishes it.
pub(crate) struct Overflow;

impl Dfa {
    /// The DFA for a program, when the program is regular.
    pub(crate) fn new(p: &Prog) -> Option<Dfa> {
        if p.utf && p.insts.iter().any(|i| matches!(i, Inst::Byte | Inst::Repeat { atom: Atom::Byte, .. })) {
            return None;
        }
        if p.newline == Newline::CrLf || p.not_empty || p.match_limit.is_some() || p.depth_limit.is_some() {
            return None;
        }
        let simple_newline = matches!(p.newline, Newline::Lf | Newline::Cr | Newline::Nul);
        let mut nodes = Vec::with_capacity(p.insts.len());
        // Targets that name a program position (rather than a node) are
        // tagged with INST and mapped once every node exists.
        const INST: u32 = 1 << 30;
        let mut at = vec![0u32; p.insts.len() + 1];
        let mut needs_last = false;
        // The end of a lookaround whose body was taken as assertions.
        let mut look_end = usize::MAX;
        let assertion_ok = |a: &Assert, needs_last: &mut bool| -> bool {
            match a {
                Assert::StartLine | Assert::EndLine => simple_newline,
                Assert::EndOrNewline => {
                    *needs_last = true;
                    simple_newline
                }
                _ => true,
            }
        };
        for (pc, inst) in p.insts.iter().enumerate() {
            at[pc] = nodes.len() as u32;
            let next = INST | (pc as u32 + 1);
            match inst {
                Inst::Match => nodes.push(Node::Match),
                // A lookaround made only of assertions is an assertion
                // (`(?!^)`): zero-width, so looking behind is the same.
                Inst::Look { neg, next: after, .. } => {
                    let body = &p.insts[pc + 1..*after as usize];
                    let [asserts @ .., Inst::SubEnd | Inst::SubEndAt] = body else { return None };
                    if asserts.is_empty() || (*neg && asserts.len() > 1) || !asserts.iter().all(|i| matches!(i, Inst::Assert(a) if assertion_ok(a, &mut needs_last))) {
                        return None;
                    }
                    for (k, i) in asserts.iter().enumerate() {
                        let Inst::Assert(a) = i else { unreachable!() };
                        let n = nodes.len() as u32;
                        let to = if k + 1 == asserts.len() { INST | after } else { n + 1 };
                        nodes.push(Node::Assert { a: *a, neg: *neg, next: to });
                    }
                    look_end = *after as usize - 1;
                }
                Inst::SubEnd | Inst::SubEndAt if pc <= look_end => nodes.push(Node::Jmp(next)),
                Inst::Char(c) => nodes.push(Node::Atom(Atom::Char(*c), next)),
                Inst::CharNc(c) => nodes.push(Node::Atom(Atom::CharNc(*c), next)),
                Inst::CharNcR(c) => nodes.push(Node::Atom(Atom::CharNcR(*c), next)),
                Inst::Any => nodes.push(Node::Atom(Atom::Any, next)),
                Inst::AnyNoNl => nodes.push(Node::Atom(Atom::AnyNoNl, next)),
                Inst::Byte => nodes.push(Node::Atom(Atom::Byte, next)),
                Inst::Set(i) => nodes.push(Node::Atom(Atom::Set(*i), next)),
                Inst::Assert(a) => {
                    if !assertion_ok(a, &mut needs_last) {
                        return None;
                    }
                    nodes.push(Node::Assert { a: *a, neg: false, next });
                }
                Inst::Split { first, second, .. } => nodes.push(Node::Split(INST | first, INST | second)),
                Inst::Jmp(t) => nodes.push(Node::Jmp(INST | t)),
                Inst::Save(_) | Inst::Mark(_) | Inst::GroupEnd(_) => nodes.push(Node::Jmp(next)),
                // An empty iteration leaves the loop: in a closure, going
                // round again reaches only positions already taken.
                Inst::LoopCheck { back, .. } => nodes.push(Node::Jmp(INST | back)),
                Inst::Repeat { atom, min, max, kind } => {
                    if *kind == RepeatKind::Possessive {
                        return None;
                    }
                    let bounded = if *max == u32::MAX { 0 } else { *max - *min };
                    if nodes.len() + *min as usize + bounded as usize * 2 + 2 > MAX_NODES {
                        return None;
                    }
                    for _ in 0..*min {
                        let n = nodes.len() as u32;
                        nodes.push(Node::Atom(*atom, n + 1));
                    }
                    if *max == u32::MAX {
                        let split = nodes.len() as u32;
                        let (body, exit) = (split + 1, next);
                        nodes.push(if *kind == RepeatKind::Greedy { Node::Split(body, exit) } else { Node::Split(exit, body) });
                        nodes.push(Node::Atom(*atom, split));
                    } else {
                        for _ in 0..bounded {
                            let split = nodes.len() as u32;
                            nodes.push(if *kind == RepeatKind::Greedy { Node::Split(split + 1, next) } else { Node::Split(next, split + 1) });
                            nodes.push(Node::Atom(*atom, split + 2));
                        }
                        nodes.push(Node::Jmp(next));
                    }
                }
                Inst::Atomic { .. }
                | Inst::SubEnd
                | Inst::SubEndAt
                | Inst::BackRef { .. }
                | Inst::Call { .. }
                | Inst::CondGroup { .. }
                | Inst::CondRecursion { .. }
                | Inst::CondLook { .. }
                | Inst::Keep
                | Inst::Newline
                | Inst::Grapheme
                | Inst::Fail
                | Inst::Accept
                | Inst::Commit
                | Inst::Prune(_)
                | Inst::Skip(_)
                | Inst::Then(_)
                | Inst::SetMark(_) => return None,
            }
            if nodes.len() > MAX_NODES {
                return None;
            }
        }
        at[p.insts.len()] = nodes.len() as u32;
        let map = |t: u32| if t & INST != 0 { at[(t & !INST) as usize] } else { t };
        for n in &mut nodes {
            match n {
                Node::Atom(_, t) | Node::Jmp(t) | Node::Assert { next: t, .. } => *t = map(*t),
                Node::Split(a, b) => {
                    *a = map(*a);
                    *b = map(*b);
                }
                Node::Match => {}
            }
        }
        let count = nodes.len();
        let mut dfa = Dfa {
            nodes,
            start: 0,
            needs_last,
            utf: p.utf,
            states: Vec::new(),
            index: HashMap::new(),
            wide: HashMap::new(),
            last: HashMap::new(),
            starts: [UNKNOWN; 8],
            resets: 0,
            disabled: false,
            visited: vec![0; count],
            stamp: 0,
            stack: Vec::new(),
            next: Vec::new(),
        };
        dfa.clear();
        Some(dfa)
    }

    fn clear(&mut self) {
        self.states.clear();
        self.index.clear();
        self.wide.clear();
        self.last.clear();
        self.starts = [UNKNOWN; 8];
        // State 0 is the dead state: nothing alive, no restart.
        self.state(Box::new([]), 0);
    }

    /// The state with this kernel and flags, made if it is new.
    fn state(&mut self, kernel: Box<[u32]>, flags: u8) -> u32 {
        if let Some(&id) = self.index.get(&(kernel.clone(), flags)) {
            return id;
        }
        let id = self.states.len() as u32;
        self.states.push(State { kernel: kernel.clone(), flags, trans: Box::new([UNKNOWN; 256]), eof: None });
        self.index.insert((kernel, flags), id);
        id
    }

    /// An empty state that restarts at each position, for the character
    /// before the position.
    fn restart_state(&mut self, flags: u8) -> u32 {
        let k = (flags & 7) as usize;
        if self.starts[k] == UNKNOWN {
            self.starts[k] = self.state(Box::new([]), (flags & 7) | RESTART);
        }
        self.starts[k]
    }

    /// What the state after a character knows about it.
    fn flags_after(p: &Prog, c: u32) -> u8 {
        (if is_word(p, Some(c)) { PREV_WORD } else { 0 }) | (if newline_char(p, c) { PREV_NL } else { 0 })
    }

    /// What a state at `pos` knows about the character before it.
    fn flags_at(p: &Prog, s: &[u8], pos: usize) -> u8 {
        if pos == 0 {
            return AT_START;
        }
        let mut i = pos - 1;
        if p.utf {
            while i > 0 && s[i] & 0xc0 == 0x80 {
                i -= 1;
            }
        }
        Self::flags_after(p, crate::decode(s, i, p.utf).0)
    }

    /// The transition from `sid` on `c` (None at the end of the subject):
    /// the next state with MATCH_BIT when a match ends before `c`. With
    /// `last`, `c` is the subject's last character. Err when the cache is
    /// full: it has been cleared.
    fn step(&mut self, p: &Prog, sid: u32, c: Option<u32>, last: bool) -> Result<u32, Overflow> {
        let flags = self.states[sid as usize].flags;
        self.stamp = self.stamp.wrapping_add(1);
        if self.stamp == 0 {
            self.visited.iter_mut().for_each(|v| *v = 0);
            self.stamp = 1;
        }
        self.next.clear();
        let mut matched = false;
        let kernel_len = self.states[sid as usize].kernel.len();
        let count = kernel_len + usize::from(flags & RESTART != 0);
        'threads: for i in 0..count {
            let root = if i < kernel_len { self.states[sid as usize].kernel[i] } else { self.start };
            self.stack.clear();
            self.stack.push(root);
            while let Some(n) = self.stack.pop() {
                if self.visited[n as usize] == self.stamp {
                    continue;
                }
                self.visited[n as usize] = self.stamp;
                match self.nodes[n as usize] {
                    Node::Atom(atom, next) => {
                        if let Some(c) = c
                            && atom_matches(p, atom, c)
                        {
                            self.next.push(next);
                        }
                    }
                    Node::Split(a, b) => {
                        self.stack.push(b);
                        self.stack.push(a);
                    }
                    Node::Jmp(t) => self.stack.push(t),
                    Node::Assert { a, neg, next } => {
                        let holds = match a {
                            Assert::Start => flags & AT_START != 0,
                            // Not after a newline that ends the subject.
                            Assert::StartLine => flags & AT_START != 0 || (c.is_some() && flags & PREV_NL != 0),
                            Assert::End => c.is_none(),
                            Assert::EndOrNewline => c.is_none() || (last && newline_char(p, c.unwrap_or(0))),
                            Assert::EndLine => c.is_none_or(|c| newline_char(p, c)),
                            Assert::StartOffset => flags & AT_OFFSET != 0,
                            Assert::WordBoundary { .. } | Assert::NotWordBoundary { .. } => {
                                let before = flags & PREV_WORD != 0;
                                let after = is_word(p, c);
                                (before != after) == matches!(a, Assert::WordBoundary { .. })
                            }
                        };
                        if holds != neg {
                            self.stack.push(next);
                        }
                    }
                    Node::Match => {
                        if flags & NO_EMPTY != 0 {
                            continue;
                        }
                        matched = true;
                        break 'threads;
                    }
                }
            }
        }
        let Some(c) = c else {
            return Ok(if matched { MATCH_BIT } else { 0 });
        };
        let mut next_flags = Self::flags_after(p, c);
        if flags & RESTART != 0 && !matched {
            next_flags |= RESTART;
        }
        let id = if self.next.is_empty() && next_flags & RESTART == 0 {
            DEAD
        } else {
            if self.states.len() >= MAX_STATES {
                self.resets += 1;
                self.clear();
                if self.resets >= MAX_RESETS {
                    self.disabled = true;
                }
                return Err(Overflow);
            }
            let kernel: Box<[u32]> = self.next.as_slice().into();
            self.state(kernel, next_flags)
        };
        Ok(id | if matched { MATCH_BIT } else { 0 })
    }

    /// Where the leftmost-first match from `start` ends, if there is one,
    /// starting only at `start` when `anchored`. `offset` is the search's
    /// start offset (`\G`); `no_empty` rejects an empty match there.
    fn find_end(&mut self, p: &Prog, s: &[u8], start: usize, offset: usize, no_empty: bool, anchored: bool) -> Result<Option<usize>, Overflow> {
        let mut flags = Self::flags_at(p, s, start);
        if start == offset {
            flags |= AT_OFFSET | if no_empty { NO_EMPTY } else { 0 };
        }
        let mut sid = if anchored { self.state(Box::new([self.start]), flags) } else { self.state(Box::new([]), flags | RESTART) };
        let mut found = None;
        let mut pos = start;
        let len = s.len();
        while pos < len {
            let st = &self.states[sid as usize];
            if st.kernel.is_empty() && st.flags & RESTART != 0 && !matches!(p.first, crate::compile::First::Any) {
                // Nothing alive: skip to where a match can start.
                match crate::exec::next_start(p, s, pos) {
                    Some(n) if n == pos => {}
                    Some(n) => {
                        pos = n;
                        sid = self.restart_state(Self::flags_at(p, s, pos));
                        continue;
                    }
                    None => return Ok(found),
                }
            }
            let b = s[pos];
            let (c, n) = if self.utf && b >= 0x80 { crate::decode(s, pos, true) } else { (b as u32, 1) };
            let last = self.needs_last && pos + n >= len;
            let tr = if last {
                match self.last.get(&(sid, c)) {
                    Some(&t) => t,
                    None => {
                        let t = self.step(p, sid, Some(c), true)?;
                        self.last.insert((sid, c), t);
                        t
                    }
                }
            } else if n == 1 {
                let t = self.states[sid as usize].trans[b as usize];
                if t == UNKNOWN {
                    let t = self.step(p, sid, Some(c), false)?;
                    self.states[sid as usize].trans[b as usize] = t;
                    t
                } else {
                    t
                }
            } else {
                match self.wide.get(&(sid, c)) {
                    Some(&t) => t,
                    None => {
                        let t = self.step(p, sid, Some(c), false)?;
                        self.wide.insert((sid, c), t);
                        t
                    }
                }
            };
            if tr & MATCH_BIT != 0 {
                found = Some(pos);
            }
            sid = tr & !MATCH_BIT;
            if sid == DEAD {
                return Ok(found);
            }
            pos += n;
        }
        let eof = match self.states[sid as usize].eof {
            Some(m) => m,
            None => {
                let m = self.step(p, sid, None, false)? & MATCH_BIT != 0;
                self.states[sid as usize].eof = Some(m);
                m
            }
        };
        Ok(if eof { Some(len) } else { found })
    }

    /// The leftmost-first match at or after `start`: its bounds. With
    /// `no_empty`, an empty match at `start` does not count
    /// (NOTEMPTY_ATSTART).
    pub(crate) fn find(&mut self, p: &Prog, s: &[u8], start: usize, no_empty: bool, anchored: bool) -> Result<Option<(usize, usize)>, Overflow> {
        if self.disabled {
            return Err(Overflow);
        }
        if anchored {
            return Ok(self.find_end(p, s, start, start, no_empty, true)?.map(|end| (start, end)));
        }
        let Some(end) = self.find_end(p, s, start, start, no_empty, false)? else { return Ok(None) };
        // The leftmost start that matches: the backtracker's. A start
        // whose match ends at `end` is the one the unanchored search found.
        let mut at = start;
        loop {
            if !matches!(p.first, crate::compile::First::Any) {
                match crate::exec::next_start(p, s, at) {
                    Some(n) => at = n,
                    None => break,
                }
            }
            if at > end {
                break;
            }
            if let Some(e) = self.find_end(p, s, at, start, no_empty, true)? {
                return Ok(Some((at, e)));
            }
            if at >= s.len() {
                break;
            }
            at += if p.utf { crate::decode(s, at, true).1 } else { 1 };
        }
        // Unreachable when the DFA agrees with itself; the machine decides.
        Err(Overflow)
    }
}
