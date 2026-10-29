//! The backtracking machine: runs a program against a subject.
//!
//! The backtrack stack holds both the choices still to try and the undo
//! records for captures, loop marks and call frames, so backtracking past a
//! point restores the state it found there. Atomic groups and lookarounds
//! run nested, and on success keep their undo records but drop their
//! choices.

use crate::compile::{Atom, Inst, Prog, RepeatKind};
use crate::parse::{Assert, Newline};
use crate::unicode;
use crate::{decode, Captures, MatchError, MatchOptions};

#[derive(Clone)]
struct Frame {
    group: u32,
    ret: u32,
    pos: usize,
    saved: Vec<Option<usize>>,
}

enum Bt {
    Alt { pc: u32, pos: usize, alt: bool },
    GiveBack { next: u32, min: usize, pos: usize },
    TakeMore { rep: u32, pos: usize, count: u32 },
    Slot { idx: u32, old: Option<usize>, frame: bool },
    Reg { idx: u32, old: usize },
    PopFrame,
    PushFrame(Frame),
    Mark { old: Option<u32>, new: u32, pos: usize },
    Verb(Barrier),
}

#[derive(Clone, Copy, Debug)]
enum Barrier {
    Commit,
    Prune,
    Skip(usize),
    Then,
}

enum Outcome {
    Match(usize),
    Accept(usize),
    NoMatch,
    Verb(Barrier),
}

struct Vm<'a> {
    p: &'a Prog,
    s: &'a [u8],
    slots: Vec<Option<usize>>,
    regs: Vec<usize>,
    stack: Vec<Bt>,
    frames: Vec<Frame>,
    mark: Option<u32>,
    steps: u64,
    match_limit: u64,
    depth: u64,
    depth_limit: u64,
    start_offset: usize,
    not_empty: bool,
    not_empty_atstart: bool,
    targets: Vec<usize>,
    nest: u32,
}

pub(crate) fn exec(p: &Prog, s: &[u8], start: usize, opts: &MatchOptions) -> Result<Option<Captures>, MatchError> {
    let mut vm = Vm {
        p,
        s,
        slots: vec![None; p.slots],
        regs: vec![0; p.regs],
        stack: Vec::new(),
        frames: Vec::new(),
        mark: None,
        steps: 0,
        match_limit: p.match_limit.map_or(opts.match_limit, |l| l.min(opts.match_limit)),
        depth: 0,
        depth_limit: p.depth_limit.map_or(opts.depth_limit, |l| l.min(opts.depth_limit)),
        start_offset: start,
        not_empty: opts.not_empty || p.not_empty,
        not_empty_atstart: opts.not_empty_atstart || p.not_empty_atstart,
        targets: Vec::new(),
        nest: 0,
    };
    let anchored = p.anchored || opts.anchored;
    let mut at = start;
    while at <= s.len() {
        if let Some(first) = &p.first {
            match s[at..].iter().position(|&b| first[b as usize]) {
                Some(n) => at += n,
                None => break,
            }
            if anchored && at != start {
                break;
            }
        }
        if p.startline && at > start && !vm.after_newline(at) {
            if at >= s.len() {
                break;
            }
            at += vm.char_len(at);
            continue;
        }
        vm.slots.iter_mut().for_each(|x| *x = None);
        vm.slots[0] = Some(at);
        vm.stack.clear();
        vm.frames.clear();
        vm.depth = 0;
        vm.step()?;
        let mut next = None;
        match vm.run(0, at)? {
            Outcome::Match(end) | Outcome::Accept(end) => {
                vm.slots[1] = Some(end);
                let mark = vm.mark.map(|m| p.marks[m as usize].clone());
                return Ok(Some(Captures { slots: vm.slots, mark }));
            }
            Outcome::NoMatch | Outcome::Verb(Barrier::Prune | Barrier::Then) => {}
            Outcome::Verb(Barrier::Commit) => break,
            Outcome::Verb(Barrier::Skip(to)) => {
                if to > at {
                    next = Some(to);
                }
            }
        }
        if anchored {
            break;
        }
        at = match next {
            Some(n) => n,
            None if at >= s.len() => break,
            None => at + vm.char_len(at),
        };
    }
    Ok(None)
}

impl Vm<'_> {
    fn step(&mut self) -> Result<(), MatchError> {
        self.steps += 1;
        if self.steps > self.match_limit {
            return Err(MatchError::MatchLimit);
        }
        Ok(())
    }

    fn push_choice(&mut self, bt: Bt) -> Result<(), MatchError> {
        self.step()?;
        self.depth += 1;
        if self.depth >= self.depth_limit {
            return Err(MatchError::DepthLimit);
        }
        self.stack.push(bt);
        Ok(())
    }

    fn char_len(&self, at: usize) -> usize {
        if self.p.utf { decode(self.s, at, true).1 } else { 1 }
    }

    fn char_at(&self, at: usize) -> Option<(u32, usize)> {
        if at >= self.s.len() {
            return None;
        }
        Some(decode(self.s, at, self.p.utf))
    }

    fn prev(&self, at: usize) -> usize {
        let mut i = at - 1;
        if self.p.utf {
            while i > 0 && self.s[i] & 0xc0 == 0x80 {
                i -= 1;
            }
        }
        i
    }

    fn prev_char(&self, at: usize) -> Option<u32> {
        if at == 0 {
            return None;
        }
        Some(decode(self.s, self.prev(at), self.p.utf).0)
    }

    /// The length of the newline at `at`, if there is one.
    fn newline_at(&self, at: usize) -> Option<usize> {
        let s = self.s;
        let b = *s.get(at)?;
        match self.p.newline {
            Newline::Lf => (b == b'\n').then_some(1),
            Newline::Cr => (b == b'\r').then_some(1),
            Newline::Nul => (b == 0).then_some(1),
            Newline::CrLf => (s[at..].starts_with(b"\r\n")).then_some(2),
            Newline::AnyCrLf => match b {
                b'\r' if s.get(at + 1) == Some(&b'\n') => Some(2),
                b'\r' | b'\n' => Some(1),
                _ => None,
            },
            Newline::Any => match b {
                b'\r' if s.get(at + 1) == Some(&b'\n') => Some(2),
                0x0a..=0x0d => Some(1),
                0x85 if !self.p.utf => Some(1),
                _ if self.p.utf => {
                    let (c, n) = decode(s, at, true);
                    matches!(c, 0x85 | 0x2028 | 0x2029).then_some(n)
                }
                _ => None,
            },
        }
    }

    /// Whether a newline ends right before `at`.
    fn after_newline(&self, at: usize) -> bool {
        if at == 0 {
            return false;
        }
        let s = self.s;
        match self.p.newline {
            Newline::CrLf => at >= 2 && &s[at - 2..at] == b"\r\n",
            Newline::Lf | Newline::Cr | Newline::Nul => self.newline_at(at - 1).is_some(),
            Newline::AnyCrLf | Newline::Any => {
                // \r\n is one newline: no line starts between its halves.
                if s[at - 1] == b'\r' && s.get(at) == Some(&b'\n') {
                    return false;
                }
                let p = self.prev(at);
                self.newline_at(p).is_some_and(|n| p + n == at) || (s[at - 1] == b'\n')
            }
        }
    }

    fn is_word(&self, c: Option<u32>) -> bool {
        match c {
            None => false,
            Some(c) if self.p.ucp => unicode::has_property(c, unicode::PT_WORD, 0),
            Some(c) => c < 128 && ((c as u8).is_ascii_alphanumeric() || c == b'_' as u32),
        }
    }

    fn caseless_eq(&self, a: u32, b: u32) -> bool {
        if self.p.utf || self.p.ucp {
            unicode::caseless_eq(a, b)
        } else {
            a == b || (a < 128 && b < 128 && (a as u8).eq_ignore_ascii_case(&(b as u8)))
        }
    }

    /// One item at `at`: the position after it.
    fn atom(&self, atom: Atom, at: usize) -> Option<usize> {
        match atom {
            Atom::Byte => (at < self.s.len()).then_some(at + 1),
            Atom::AnyNoNl => {
                if self.newline_at(at).is_some() {
                    // Under CRLF a lone \r is not a newline.
                    return None;
                }
                let (_, n) = self.char_at(at)?;
                Some(at + n)
            }
            _ => {
                let (c, n) = self.char_at(at)?;
                let ok = match atom {
                    Atom::Char(x) => c == x,
                    Atom::CharNc(x) => self.caseless_eq(c, x),
                    Atom::CharNcR(x) => (c < 128) == (x < 128) && self.caseless_eq(c, x),
                    Atom::Any => true,
                    Atom::Set(i) => self.p.sets[i as usize].matches(c, self.p.utf),
                    _ => unreachable!(),
                };
                ok.then_some(at + n)
            }
        }
    }

    fn set_slot(&mut self, idx: u32, value: Option<usize>) -> Result<(), MatchError> {
        let old = self.slots[idx as usize];
        let frame = idx.is_multiple_of(2) && idx > 0;
        if frame {
            self.depth += 1;
            if self.depth >= self.depth_limit {
                return Err(MatchError::DepthLimit);
            }
        }
        self.stack.push(Bt::Slot { idx, old, frame });
        self.slots[idx as usize] = value;
        Ok(())
    }

    /// Pops back to `base`, undoing as it goes.
    fn unwind(&mut self, base: usize) {
        while self.stack.len() > base {
            let bt = self.stack.pop().unwrap();
            self.undo(bt);
        }
    }

    /// Applies an undo record; for a choice, just forgets it.
    fn undo(&mut self, bt: Bt) {
        match bt {
            Bt::Alt { .. } | Bt::GiveBack { .. } | Bt::TakeMore { .. } => self.depth -= 1,
            Bt::Slot { idx, old, frame } => {
                self.slots[idx as usize] = old;
                if frame {
                    self.depth -= 1;
                }
            }
            Bt::Reg { idx, old } => self.regs[idx as usize] = old,
            Bt::PopFrame => {
                self.frames.pop();
            }
            Bt::PushFrame(f) => self.frames.push(f),
            Bt::Mark { old, .. } => self.mark = old,
            Bt::Verb(_) => {}
        }
    }

    /// After a nested run succeeded: drops its choices (and verbs), keeps
    /// its undo records.
    fn commit_nested(&mut self, base: usize) {
        let tail = self.stack.split_off(base);
        for bt in tail {
            match bt {
                Bt::Alt { .. } | Bt::GiveBack { .. } | Bt::TakeMore { .. } => self.depth -= 1,
                Bt::Verb(_) => {}
                other => self.stack.push(other),
            }
        }
    }

    fn nested(&mut self, pc: u32, at: usize) -> Result<Outcome, MatchError> {
        self.nest += 1;
        if self.nest > 1000 {
            self.nest -= 1;
            return Err(MatchError::DepthLimit);
        }
        let r = self.run(pc, at);
        self.nest -= 1;
        r
    }

    /// Runs a lookaround's body; on a match its undo records stay on the stack.
    fn assertion(&mut self, body: u32, at: usize, behind: bool, min: u32, max: u32) -> Result<bool, MatchError> {
        let base = self.stack.len();
        if !behind {
            return match self.nested(body, at)? {
                Outcome::Match(_) | Outcome::Accept(_) => {
                    self.commit_nested(base);
                    Ok(true)
                }
                _ => Ok(false),
            };
        }
        // Each length the body can have, shortest first.
        let mut start = at;
        let mut k = 0;
        while k < min {
            if start == 0 {
                return Ok(false);
            }
            start = self.prev(start);
            k += 1;
        }
        loop {
            self.targets.push(at);
            let r = self.nested(body, start);
            self.targets.pop();
            if let Outcome::Match(_) | Outcome::Accept(_) = r? {
                self.commit_nested(base);
                return Ok(true);
            }
            if k >= max || start == 0 {
                return Ok(false);
            }
            start = self.prev(start);
            k += 1;
        }
    }

    fn run(&mut self, mut pc: u32, mut pos: usize) -> Result<Outcome, MatchError> {
        let base = self.stack.len();
        let prog = self.p;
        let insts = &prog.insts;
        loop {
            let ok = match &insts[pc as usize] {
                Inst::Match => {
                    let start = self.slots[0].unwrap_or(pos);
                    if pos == start
                        && (self.not_empty || (self.not_empty_atstart && start == self.start_offset))
                    {
                        false
                    } else {
                        return Ok(Outcome::Match(pos));
                    }
                }
                Inst::Char(c) => match self.char_at(pos) {
                    Some((d, n)) if d == *c => {
                        pos += n;
                        pc += 1;
                        true
                    }
                    _ => false,
                },
                Inst::CharNc(c) => match self.char_at(pos) {
                    Some((d, n)) if self.caseless_eq(d, *c) => {
                        pos += n;
                        pc += 1;
                        true
                    }
                    _ => false,
                },
                Inst::CharNcR(c) => match self.char_at(pos) {
                    Some((d, n)) if (d < 128) == (*c < 128) && self.caseless_eq(d, *c) => {
                        pos += n;
                        pc += 1;
                        true
                    }
                    _ => false,
                },
                Inst::Any => match self.atom(Atom::Any, pos) {
                    Some(n) => {
                        pos = n;
                        pc += 1;
                        true
                    }
                    None => false,
                },
                Inst::AnyNoNl => match self.atom(Atom::AnyNoNl, pos) {
                    Some(n) => {
                        pos = n;
                        pc += 1;
                        true
                    }
                    None => false,
                },
                Inst::Byte => {
                    if pos < self.s.len() {
                        pos += 1;
                        pc += 1;
                        true
                    } else {
                        false
                    }
                }
                Inst::Set(i) => match self.char_at(pos) {
                    Some((c, n)) if self.p.sets[*i as usize].matches(c, self.p.utf) => {
                        pos += n;
                        pc += 1;
                        true
                    }
                    _ => false,
                },
                Inst::Assert(a) => {
                    let len = self.s.len();
                    let ok = match a {
                        Assert::Start => pos == 0,
                        Assert::StartLine => pos == 0 || (pos < len && self.after_newline(pos)),
                        Assert::End => pos == len,
                        Assert::EndOrNewline => pos == len || self.newline_at(pos).is_some_and(|n| pos + n == len),
                        Assert::EndLine => pos == len || self.newline_at(pos).is_some(),
                        Assert::StartOffset => pos == self.start_offset,
                        Assert::WordBoundary { .. } | Assert::NotWordBoundary { .. } => {
                            let before = self.is_word(self.prev_char(pos));
                            let after = self.is_word(self.char_at(pos).map(|c| c.0));
                            (before != after) == matches!(a, Assert::WordBoundary { .. })
                        }
                    };
                    if ok {
                        pc += 1;
                    }
                    ok
                }
                Inst::Split { first, second, alt } => {
                    self.push_choice(Bt::Alt { pc: *second, pos, alt: *alt })?;
                    pc = *first;
                    true
                }
                Inst::Jmp(t) => {
                    pc = *t;
                    true
                }
                Inst::Save(i) => {
                    self.set_slot(*i, Some(pos))?;
                    pc += 1;
                    true
                }
                Inst::Mark(r) => {
                    let old = self.regs[*r as usize];
                    self.stack.push(Bt::Reg { idx: *r, old });
                    self.regs[*r as usize] = pos;
                    pc += 1;
                    true
                }
                Inst::LoopCheck { reg, exit, back } => {
                    pc = if self.regs[*reg as usize] == pos { *exit } else { *back };
                    true
                }
                Inst::Repeat { atom, min, max, kind } => {
                    let (atom, min, max, kind) = (*atom, *min, *max, *kind);
                    let mut at = pos;
                    let mut count = 0;
                    let mut ok = true;
                    while count < min {
                        match self.atom(atom, at) {
                            Some(n) => at = n,
                            None => {
                                ok = false;
                                break;
                            }
                        }
                        count += 1;
                    }
                    if ok {
                        match kind {
                            RepeatKind::Greedy | RepeatKind::Possessive => {
                                let floor = at;
                                while count < max {
                                    match self.atom(atom, at) {
                                        Some(n) => at = n,
                                        None => break,
                                    }
                                    count += 1;
                                }
                                if kind == RepeatKind::Greedy && at > floor {
                                    self.push_choice(Bt::GiveBack { next: pc + 1, min: floor, pos: at })?;
                                }
                            }
                            RepeatKind::Lazy => {
                                if count < max {
                                    self.push_choice(Bt::TakeMore { rep: pc, pos: at, count })?;
                                }
                            }
                        }
                        pos = at;
                        pc += 1;
                    }
                    ok
                }
                Inst::Look { behind, neg, next, min, max } => {
                    let (behind, neg, next, min, max) = (*behind, *neg, *next, *min, *max);
                    let base = self.stack.len();
                    let matched = self.assertion(pc + 1, pos, behind, min, max)?;
                    if matched && neg {
                        self.unwind(base);
                    }
                    if matched != neg {
                        pc = next;
                        true
                    } else {
                        false
                    }
                }
                Inst::Atomic { next } => {
                    let next = *next;
                    let base = self.stack.len();
                    match self.nested(pc + 1, pos)? {
                        Outcome::Match(end) => {
                            self.commit_nested(base);
                            pos = end;
                            pc = next;
                            true
                        }
                        Outcome::Accept(end) => {
                            self.commit_nested(base);
                            return Ok(Outcome::Accept(end));
                        }
                        Outcome::NoMatch => false,
                        Outcome::Verb(v) => {
                            self.unwind(base);
                            return self.verb_out(v, base);
                        }
                    }
                }
                Inst::SubEnd => return Ok(Outcome::Match(pos)),
                Inst::SubEndAt => {
                    if Some(&pos) == self.targets.last() {
                        return Ok(Outcome::Match(pos));
                    }
                    false
                }
                Inst::BackRef { groups, nocase } => {
                    let set = groups.iter().find_map(|&g| {
                        match (self.slots[g as usize * 2], self.slots[g as usize * 2 + 1]) {
                            (Some(a), Some(b)) => Some((a, b)),
                            _ => None,
                        }
                    });
                    match set {
                        None => false,
                        Some((a, b)) => match self.backref(a, b, pos, *nocase) {
                            Some(end) => {
                                pos = end;
                                pc += 1;
                                true
                            }
                            None => false,
                        },
                    }
                }
                Inst::Call { group, target } => {
                    let (group, target) = (*group, *target);
                    if self.frames.iter().any(|f| f.group == group && f.pos == pos) {
                        // Recursing again without consuming anything would
                        // never end.
                        false
                    } else {
                        if self.frames.len() as u64 >= self.depth_limit.min(5000) {
                            return Err(MatchError::DepthLimit);
                        }
                        self.frames.push(Frame { group, ret: pc + 1, pos, saved: self.slots.clone() });
                        self.stack.push(Bt::PopFrame);
                        pc = target;
                        true
                    }
                }
                Inst::GroupEnd(g) => {
                    if self.frames.last().is_some_and(|f| f.group == *g) {
                        self.return_from_call(&mut pc)?;
                    } else {
                        pc += 1;
                    }
                    true
                }
                Inst::CondGroup { groups, no } => {
                    if groups.iter().any(|&g| self.slots[g as usize * 2 + 1].is_some()) {
                        pc += 1;
                    } else {
                        pc = *no;
                    }
                    true
                }
                Inst::CondRecursion { group, no } => {
                    let yes = match group {
                        None => !self.frames.is_empty(),
                        Some(g) => self.frames.last().is_some_and(|f| f.group == *g),
                    };
                    pc = if yes { pc + 1 } else { *no };
                    true
                }
                Inst::CondLook { behind, neg, min, max, yes, no } => {
                    let (behind, neg, min, max, yes, no) = (*behind, *neg, *min, *max, *yes, *no);
                    let base = self.stack.len();
                    let matched = self.assertion(pc + 1, pos, behind, min, max)?;
                    if matched && neg {
                        self.unwind(base);
                    }
                    pc = if matched != neg { yes } else { no };
                    true
                }
                Inst::Keep => {
                    let old = self.slots[0];
                    self.stack.push(Bt::Slot { idx: 0, old, frame: false });
                    self.slots[0] = Some(pos);
                    pc += 1;
                    true
                }
                Inst::Newline => match self.newline_seq(pos) {
                    Some(n) => {
                        pos += n;
                        pc += 1;
                        true
                    }
                    None => false,
                },
                Inst::Grapheme => match self.grapheme(pos) {
                    Some(end) => {
                        pos = end;
                        pc += 1;
                        true
                    }
                    None => false,
                },
                Inst::Fail => false,
                Inst::Accept => {
                    if !self.frames.is_empty() {
                        self.return_from_call(&mut pc)?;
                        true
                    } else {
                        let start = self.slots[0].unwrap_or(pos);
                        if pos == start && (self.not_empty || (self.not_empty_atstart && start == self.start_offset)) {
                            false
                        } else {
                            return Ok(Outcome::Accept(pos));
                        }
                    }
                }
                Inst::Commit => {
                    self.stack.push(Bt::Verb(Barrier::Commit));
                    pc += 1;
                    true
                }
                Inst::Prune(name) => {
                    if let Some(n) = name {
                        self.set_mark(*n, pos);
                    }
                    self.stack.push(Bt::Verb(Barrier::Prune));
                    pc += 1;
                    true
                }
                Inst::Skip(name) => {
                    match name {
                        None => self.stack.push(Bt::Verb(Barrier::Skip(pos))),
                        Some(n) => {
                            // Skip to where the named mark was set; with no
                            // such mark the skip is ignored.
                            let to = self.stack.iter().rev().find_map(|b| match b {
                                Bt::Mark { new, pos, .. } if new == n => Some(*pos),
                                _ => None,
                            });
                            if let Some(to) = to {
                                self.stack.push(Bt::Verb(Barrier::Skip(to)));
                            }
                        }
                    }
                    pc += 1;
                    true
                }
                Inst::Then(name) => {
                    if let Some(n) = name {
                        self.set_mark(*n, pos);
                    }
                    self.stack.push(Bt::Verb(Barrier::Then));
                    pc += 1;
                    true
                }
                Inst::SetMark(n) => {
                    self.set_mark(*n, pos);
                    pc += 1;
                    true
                }
            };
            if ok {
                continue;
            }
            // Backtrack.
            loop {
                if self.stack.len() == base {
                    return Ok(Outcome::NoMatch);
                }
                match self.stack.pop().unwrap() {
                    Bt::Alt { pc: p, pos: q, .. } => {
                        self.depth -= 1;
                        pc = p;
                        pos = q;
                        break;
                    }
                    Bt::GiveBack { next, min, pos: q } => {
                        self.depth -= 1;
                        let back = self.prev(q);
                        if back > min {
                            self.push_choice(Bt::GiveBack { next, min, pos: back })?;
                        }
                        pc = next;
                        pos = back;
                        break;
                    }
                    Bt::TakeMore { rep, pos: q, count } => {
                        self.depth -= 1;
                        let Inst::Repeat { atom, max, .. } = &insts[rep as usize] else { unreachable!() };
                        if let Some(n) = self.atom(*atom, q) {
                            if count + 1 < *max {
                                self.push_choice(Bt::TakeMore { rep, pos: n, count: count + 1 })?;
                            }
                            pc = rep + 1;
                            pos = n;
                            break;
                        }
                    }
                    Bt::Verb(Barrier::Then) => {
                        // Back to the innermost alternation's next branch.
                        let mut found = None;
                        while self.stack.len() > base {
                            match self.stack.pop().unwrap() {
                                Bt::Alt { pc: p, pos: q, alt: true } => {
                                    self.depth -= 1;
                                    found = Some((p, q));
                                    break;
                                }
                                other => self.undo(other),
                            }
                        }
                        match found {
                            Some((p, q)) => {
                                pc = p;
                                pos = q;
                                break;
                            }
                            None => return Ok(Outcome::Verb(Barrier::Then)),
                        }
                    }
                    Bt::Verb(v) => {
                        self.unwind(base);
                        return Ok(Outcome::Verb(v));
                    }
                    other => self.undo(other),
                }
            }
        }
    }

    fn verb_out(&mut self, v: Barrier, base: usize) -> Result<Outcome, MatchError> {
        self.unwind(base);
        Ok(Outcome::Verb(v))
    }

    fn set_mark(&mut self, n: u32, pos: usize) {
        self.stack.push(Bt::Mark { old: self.mark, new: n, pos });
        self.mark = Some(n);
    }

    /// Returns from the innermost call: captures go back to what they were
    /// before it, the position stays.
    fn return_from_call(&mut self, pc: &mut u32) -> Result<(), MatchError> {
        let f = self.frames.pop().unwrap();
        for i in 0..self.slots.len() {
            if i >= 2 && self.slots[i] != f.saved[i] {
                let old = self.slots[i];
                self.stack.push(Bt::Slot { idx: i as u32, old, frame: false });
                self.slots[i] = f.saved[i];
            }
        }
        *pc = f.ret;
        self.stack.push(Bt::PushFrame(f));
        Ok(())
    }

    fn backref(&self, a: usize, b: usize, at: usize, nocase: bool) -> Option<usize> {
        let sub = &self.s[a..b];
        if !nocase {
            return self.s[at..].starts_with(sub).then_some(at + sub.len());
        }
        let (mut i, mut j) = (0, at);
        while i < sub.len() {
            let (c, n) = decode(sub, i, self.p.utf);
            let (d, m) = self.char_at(j)?;
            if !self.caseless_eq(c, d) {
                return None;
            }
            i += n;
            j += m;
        }
        Some(j)
    }

    /// `\R` at `at`: the newline sequence's length.
    fn newline_seq(&self, at: usize) -> Option<usize> {
        let s = self.s;
        let b = *s.get(at)?;
        if b == b'\r' && s.get(at + 1) == Some(&b'\n') {
            return Some(2);
        }
        if b == b'\r' || b == b'\n' {
            return Some(1);
        }
        if self.p.bsr_anycrlf {
            return None;
        }
        let (c, n) = self.char_at(at)?;
        matches!(c, 0x0b | 0x0c | 0x85 | 0x2028 | 0x2029).then_some(n)
    }

    /// `\X` at `at`: the end of the extended grapheme cluster.
    fn grapheme(&self, at: usize) -> Option<usize> {
        let (c, n) = self.char_at(at)?;
        let mut end = at + n;
        let mut prev = unicode::grapheme_break(c);
        let mut ri_count = usize::from(prev == unicode::GB_RI);
        // Emoji ZWJ sequences: Extended_Pictographic Extend* ZWJ × Pictographic.
        let mut pict_seen = prev == unicode::GB_PICT;
        while let Some((d, m)) = self.char_at(end) {
            let next = unicode::grapheme_break(d);
            let joins = if prev == unicode::GB_RI && next == unicode::GB_RI {
                ri_count % 2 == 1
            } else if prev == unicode::GB_ZWJ && next == unicode::GB_PICT {
                pict_seen
            } else {
                unicode::grapheme_joins(prev, next) && !(prev == unicode::GB_ZWJ && next == unicode::GB_PICT)
            };
            if !joins {
                break;
            }
            if next == unicode::GB_RI {
                ri_count += 1;
            }
            if next == unicode::GB_PICT {
                pict_seen = true;
            } else if next != unicode::GB_EXTEND && next != unicode::GB_ZWJ {
                pict_seen = false;
            }
            prev = next;
            end += m;
        }
        Some(end)
    }
}
