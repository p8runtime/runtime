//! Content models: a particle tree compiled to an NFA that the validator
//! steps through child element by child element, as xmlschemas.c does with
//! libxml2's regexp automata. Counted occurrences are unrolled.

use super::schema::{ElemId, Particle, Schema, Term, Wildcard};
use std::collections::BTreeSet;

const UNROLL_CAP: u32 = 128;

#[derive(Clone)]
pub(crate) enum Label {
    Elem(ElemId),
    Any(Wildcard),
}

#[derive(Default)]
struct State {
    eps: Vec<usize>,
    /// (label, document-order position, target, loop-back copy)
    trans: Vec<(Label, usize, usize, bool)>,
}

pub(crate) struct Nfa {
    states: Vec<State>,
    start: usize,
    accept: usize,
    pos: usize,
}

/// What a child element matched.
pub(crate) enum Matched {
    Decl(ElemId),
    Wild(Wildcard),
}

/// One thing a content model could take next, for the error message.
pub(crate) enum Expect {
    Elem(ElemId),
    Wild(Wildcard),
}

impl Nfa {
    pub(crate) fn new(schema: &Schema, p: &Particle) -> Nfa {
        let mut n = Nfa { states: vec![State::default()], start: 0, accept: 0, pos: 0 };
        let end = n.particle(schema, p, 0, false);
        n.accept = end;
        n
    }

    fn state(&mut self) -> usize {
        self.states.push(State::default());
        self.states.len() - 1
    }

    fn eps(&mut self, from: usize, to: usize) {
        self.states[from].eps.push(to);
    }

    /// Compiles `p` after `from`. Particles libxml2 implements with
    /// counters (see `counted`) tag their repeat copies: its error messages
    /// list those apart from what follows the loop.
    fn particle(&mut self, schema: &Schema, p: &Particle, from: usize, ctx: bool) -> usize {
        let counted = counted(p);
        let min = p.min.min(UNROLL_CAP);
        let mut cur = from;
        for _ in 0..min {
            cur = self.once(schema, &p.term, cur, ctx);
        }
        match p.max {
            None => {
                if min == 0 {
                    let end = self.state();
                    self.eps(cur, end);
                    let f = self.once(schema, &p.term, cur, ctx);
                    self.eps(f, end);
                    let g = self.once(schema, &p.term, f, ctx || counted);
                    self.eps(g, f);
                    end
                } else {
                    let g = self.once(schema, &p.term, cur, ctx || counted);
                    self.eps(g, cur);
                    cur
                }
            }
            Some(max) => {
                let extra = max.saturating_sub(p.min).min(UNROLL_CAP);
                let end = self.state();
                self.eps(cur, end);
                for k in 0..extra {
                    let tag = ctx || (counted && !(min == 0 && k == 0));
                    cur = self.once(schema, &p.term, cur, tag);
                    self.eps(cur, end);
                }
                end
            }
        }
    }

    fn once(&mut self, schema: &Schema, term: &Term, from: usize, tag: bool) -> usize {
        match term {
            Term::Elem(e) => {
                let to = self.state();
                let mut cands = Vec::new();
                if !schema.elems[*e].abstract_ {
                    cands.push(*e);
                }
                if let Some(ms) = schema.members.get(e) {
                    cands.extend(ms.iter().copied().filter(|m| !schema.elems[*m].abstract_));
                }
                for c in cands {
                    self.pos += 1;
                    let pos = self.pos;
                    self.states[from].trans.push((Label::Elem(c), pos, to, tag));
                }
                to
            }
            Term::Any(w) => {
                let to = self.state();
                self.pos += 1;
                let pos = self.pos;
                self.states[from].trans.push((Label::Any(w.clone()), pos, to, tag));
                to
            }
            Term::Seq(parts) | Term::All(parts) => {
                let mut cur = from;
                for p in parts {
                    cur = self.particle(schema, p, cur, tag);
                }
                cur
            }
            Term::Choice(parts) => {
                let to = self.state();
                if parts.is_empty() {
                    self.eps(from, to);
                }
                for p in parts {
                    let f = self.state();
                    self.eps(from, f);
                    let e = self.particle(schema, p, f, tag);
                    self.eps(e, to);
                }
                to
            }
        }
    }

    fn closure(&self, seeds: impl IntoIterator<Item = usize>) -> BTreeSet<usize> {
        let mut seen = BTreeSet::new();
        let mut stack: Vec<usize> = seeds.into_iter().collect();
        while let Some(s) = stack.pop() {
            if seen.insert(s) {
                stack.extend(self.states[s].eps.iter().copied());
            }
        }
        seen
    }

    pub(crate) fn begin(&self) -> BTreeSet<usize> {
        self.closure([self.start])
    }

    pub(crate) fn accepts(&self, set: &BTreeSet<usize>) -> bool {
        set.contains(&self.accept)
    }

    /// Steps over an element named (`ns`, `local`).
    pub(crate) fn step(&self, schema: &Schema, set: &BTreeSet<usize>, ns: &str, local: &str) -> Option<(Matched, BTreeSet<usize>)> {
        let mut found: Option<(Matched, Vec<usize>)> = None;
        let mut wild: Option<(Matched, Vec<usize>)> = None;
        for &s in set {
            for (label, _, to, _) in &self.states[s].trans {
                match label {
                    Label::Elem(e) => {
                        let d = &schema.elems[*e];
                        if d.name == local && d.ns == ns {
                            match &mut found {
                                Some((_, targets)) => targets.push(*to),
                                None => found = Some((Matched::Decl(*e), vec![*to])),
                            }
                        }
                    }
                    Label::Any(w) => {
                        if w.allows(ns) {
                            match &mut wild {
                                Some((_, targets)) => targets.push(*to),
                                None => wild = Some((Matched::Wild(w.clone()), vec![*to])),
                            }
                        }
                    }
                }
            }
        }
        let (m, mut targets) = match (found, wild) {
            (Some(f), Some(w)) => {
                let mut f = f;
                f.1.extend(w.1);
                f
            }
            (Some(f), None) => f,
            (None, Some(w)) => w,
            (None, None) => return None,
        };
        targets.sort_unstable();
        targets.dedup();
        Some((m, self.closure(targets)))
    }

    /// The names the model could take next from `set`, document order, as
    /// libxml2 words them: `missing` for the end of the content, else for
    /// an element that does not fit.
    pub(crate) fn expected(&self, set: &BTreeSet<usize>, missing: bool) -> Vec<Expect> {
        let mut all: Vec<(usize, &Label, bool)> = Vec::new();
        for &s in set {
            for (label, pos, _, tag) in &self.states[s].trans {
                all.push((*pos, label, *tag));
            }
        }
        all.sort_by_key(|(p, _, _)| *p);
        let looped = all.iter().any(|(_, _, t)| *t);
        let keep: Box<dyn Fn(bool) -> bool> = if missing {
            if looped { Box::new(|t| t) } else { Box::new(|_| true) }
        } else if all.iter().any(|(_, _, t)| !*t) {
            Box::new(|t| !t)
        } else if self.accepts(set) {
            Box::new(|_| false)
        } else {
            Box::new(|_| true)
        };
        all.into_iter()
            .filter(|(_, _, t)| keep(*t))
            .map(|(_, l, _)| match l {
                Label::Elem(e) => Expect::Elem(*e),
                Label::Any(w) => Expect::Wild(w.clone()),
            })
            .collect()
    }
}

/// Whether libxml2 builds `p` with counters rather than epsilon loops.
fn counted(p: &Particle) -> bool {
    match &p.term {
        Term::Choice(_) => p.max != Some(1),
        _ => match p.max {
            Some(m) => m > 1,
            None => p.min >= 2,
        },
    }
}

/// The `all` group state: which of its elements were seen.
pub(crate) struct AllModel<'a> {
    pub parts: &'a [Particle],
    pub seen: Vec<bool>,
}

impl<'a> AllModel<'a> {
    pub(crate) fn new(parts: &'a [Particle]) -> AllModel<'a> {
        AllModel { parts, seen: vec![false; parts.len()] }
    }

    /// The candidates of part `i`: the element and its substitutes.
    fn candidates(schema: &Schema, p: &Particle) -> Vec<ElemId> {
        let Term::Elem(e) = &p.term else { return Vec::new() };
        let mut v = Vec::new();
        if !schema.elems[*e].abstract_ {
            v.push(*e);
        }
        if let Some(ms) = schema.members.get(e) {
            v.extend(ms.iter().copied().filter(|m| !schema.elems[*m].abstract_));
        }
        v
    }

    pub(crate) fn step(&mut self, schema: &Schema, ns: &str, local: &str) -> Option<ElemId> {
        for (i, p) in self.parts.iter().enumerate() {
            if self.seen[i] {
                continue;
            }
            for c in Self::candidates(schema, p) {
                let d = &schema.elems[c];
                if d.name == local && d.ns == ns {
                    self.seen[i] = true;
                    return Some(c);
                }
            }
        }
        None
    }

    pub(crate) fn complete(&self) -> bool {
        self.parts.iter().zip(&self.seen).all(|(p, s)| *s || p.min == 0)
    }

    /// The elements not seen yet (optional ones too).
    pub(crate) fn expected(&self, schema: &Schema) -> Vec<Expect> {
        let mut out = Vec::new();
        for (i, p) in self.parts.iter().enumerate() {
            if !self.seen[i] {
                out.extend(Self::candidates(schema, p).into_iter().map(Expect::Elem));
            }
        }
        out
    }
}
