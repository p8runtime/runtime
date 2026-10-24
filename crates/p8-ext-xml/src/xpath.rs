//! XPath 1.0 over the DOM tree, for DOMXPath and SimpleXMLElement::xpath:
//! the expression language (W3C XPath 1.0) with libxml2's behaviour where
//! the spec leaves room (number formatting, error messages).
//!
//! Reference: https://www.w3.org/TR/1999/REC-xpath-19991116/, libxml2 xpath.c

use crate::dom::tree::{DOC, Doc, Id, Kind};
use std::collections::HashMap;

/// An XPath value.
#[derive(Clone, Debug, PartialEq)]
pub enum XValue {
    /// In document order, without duplicates.
    Nodes(Vec<Id>),
    Bool(bool),
    Num(f64),
    Str(Vec<u8>),
}

/// libxml2's XPath error codes (xmlXPathError), as messages.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum XError {
    UnfinishedLiteral,
    InvalidPredicate,
    InvalidExpression,
    UnregisteredFunction,
    InvalidType,
    InvalidArity,
    UndefinedVariable,
    UndefinedPrefix,
}

impl XError {
    pub fn message(self) -> &'static str {
        match self {
            XError::UnfinishedLiteral => "Unfinished literal",
            XError::InvalidPredicate => "Invalid predicate",
            XError::InvalidExpression => "Invalid expression",
            XError::UnregisteredFunction => "Unregistered function",
            XError::InvalidType => "Invalid type",
            XError::InvalidArity => "Invalid number of arguments",
            XError::UndefinedVariable => "Undefined variable",
            XError::UndefinedPrefix => "Undefined namespace prefix",
        }
    }
}

// ---- tokens ----

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Slash,
    DoubleSlash,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Dot,
    DotDot,
    At,
    Comma,
    ColonColon,
    Pipe,
    Plus,
    Minus,
    Eq,
    Neq,
    Lt,
    Le,
    Gt,
    Ge,
    /// `*` as multiplication.
    Mul,
    /// `and`, `or`, `div`, `mod` as operators.
    Op(&'static str),
    /// A name test: `*`, `p:*`, `name`, `p:name` (local "*" for wildcards).
    Name(Option<Vec<u8>>, Vec<u8>),
    Literal(Vec<u8>),
    Number(f64),
    Var(Option<Vec<u8>>, Vec<u8>),
}

fn is_name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || (c as u32 > 0x7f && crate::parser::is_name_start_char(c as u32))
}

fn is_name_char(c: char) -> bool {
    is_name_start(c) || c.is_ascii_digit() || matches!(c, '-' | '.') || (c as u32 > 0x7f && crate::parser::is_name_char(c as u32))
}

fn tokenize(expr: &str) -> Result<Vec<Tok>, XError> {
    let chars: Vec<char> = expr.chars().collect();
    let mut out: Vec<Tok> = Vec::new();
    let mut i = 0;
    // XPath 3.7: after a token that isn't @ :: ( [ , or an operator, `*`
    // multiplies and a name is an operator name.
    let operator_context = |out: &Vec<Tok>| match out.last() {
        None => false,
        Some(t) => !matches!(
            t,
            Tok::At | Tok::ColonColon | Tok::LParen | Tok::LBracket | Tok::Comma | Tok::Op(_) | Tok::Mul | Tok::Slash | Tok::DoubleSlash | Tok::Pipe | Tok::Plus | Tok::Minus | Tok::Eq | Tok::Neq | Tok::Lt | Tok::Le | Tok::Gt | Tok::Ge
        ),
    };
    let name_at = |i: usize| -> Option<(String, usize)> {
        if i < chars.len() && is_name_start(chars[i]) {
            let mut j = i + 1;
            while j < chars.len() && is_name_char(chars[j]) {
                j += 1;
            }
            Some((chars[i..j].iter().collect(), j))
        } else {
            None
        }
    };
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            ' ' | '\t' | '\n' | '\r' => i += 1,
            '/' if next == Some('/') => {
                out.push(Tok::DoubleSlash);
                i += 2;
            }
            '/' => {
                out.push(Tok::Slash);
                i += 1;
            }
            '(' => {
                out.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                out.push(Tok::RParen);
                i += 1;
            }
            '[' => {
                out.push(Tok::LBracket);
                i += 1;
            }
            ']' => {
                out.push(Tok::RBracket);
                i += 1;
            }
            '@' => {
                out.push(Tok::At);
                i += 1;
            }
            ',' => {
                out.push(Tok::Comma);
                i += 1;
            }
            '|' => {
                out.push(Tok::Pipe);
                i += 1;
            }
            '+' => {
                out.push(Tok::Plus);
                i += 1;
            }
            '-' => {
                out.push(Tok::Minus);
                i += 1;
            }
            '=' => {
                out.push(Tok::Eq);
                i += 1;
            }
            '!' if next == Some('=') => {
                out.push(Tok::Neq);
                i += 2;
            }
            '<' if next == Some('=') => {
                out.push(Tok::Le);
                i += 2;
            }
            '<' => {
                out.push(Tok::Lt);
                i += 1;
            }
            '>' if next == Some('=') => {
                out.push(Tok::Ge);
                i += 2;
            }
            '>' => {
                out.push(Tok::Gt);
                i += 1;
            }
            ':' if next == Some(':') => {
                out.push(Tok::ColonColon);
                i += 2;
            }
            '.' if next == Some('.') => {
                out.push(Tok::DotDot);
                i += 2;
            }
            '.' if !next.is_some_and(|n| n.is_ascii_digit()) => {
                out.push(Tok::Dot);
                i += 1;
            }
            '"' | '\'' => {
                let end = chars[i + 1..].iter().position(|&x| x == c).ok_or(XError::UnfinishedLiteral)?;
                out.push(Tok::Literal(chars[i + 1..i + 1 + end].iter().collect::<String>().into_bytes()));
                i += end + 2;
            }
            '0'..='9' | '.' => {
                let mut j = i;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                }
                if j < chars.len() && chars[j] == '.' {
                    j += 1;
                    while j < chars.len() && chars[j].is_ascii_digit() {
                        j += 1;
                    }
                }
                // An exponent, as libxml2's xmlXPathCompNumber reads one.
                if j < chars.len() && matches!(chars[j], 'e' | 'E') {
                    let mut k = j + 1;
                    if k < chars.len() && matches!(chars[k], '+' | '-') {
                        k += 1;
                    }
                    if k < chars.len() && chars[k].is_ascii_digit() {
                        while k < chars.len() && chars[k].is_ascii_digit() {
                            k += 1;
                        }
                        j = k;
                    }
                }
                let s: String = chars[i..j].iter().collect();
                out.push(Tok::Number(s.parse().map_err(|_| XError::InvalidExpression)?));
                i = j;
            }
            '$' => {
                let (n, j) = name_at(i + 1).ok_or(XError::InvalidExpression)?;
                if chars.get(j) == Some(&':') && let Some((l, k)) = name_at(j + 1) {
                    out.push(Tok::Var(Some(n.into_bytes()), l.into_bytes()));
                    i = k;
                } else {
                    out.push(Tok::Var(None, n.into_bytes()));
                    i = j;
                }
            }
            '*' => {
                if operator_context(&out) {
                    out.push(Tok::Mul);
                } else {
                    out.push(Tok::Name(None, b"*".to_vec()));
                }
                i += 1;
            }
            _ => {
                let (n, j) = name_at(i).ok_or(XError::InvalidExpression)?;
                if operator_context(&out) {
                    let op = match n.as_str() {
                        "and" => "and",
                        "or" => "or",
                        "div" => "div",
                        "mod" => "mod",
                        _ => return Err(XError::InvalidExpression),
                    };
                    out.push(Tok::Op(op));
                    i = j;
                    continue;
                }
                // p:name, p:* (a colon not followed by another colon).
                if chars.get(j) == Some(&':') && chars.get(j + 1) != Some(&':') {
                    if chars.get(j + 1) == Some(&'*') {
                        out.push(Tok::Name(Some(n.into_bytes()), b"*".to_vec()));
                        i = j + 2;
                        continue;
                    }
                    if let Some((l, k)) = name_at(j + 1) {
                        out.push(Tok::Name(Some(n.into_bytes()), l.into_bytes()));
                        i = k;
                        continue;
                    }
                    return Err(XError::InvalidExpression);
                }
                out.push(Tok::Name(None, n.into_bytes()));
                i = j;
            }
        }
    }
    Ok(out)
}

// ---- syntax ----

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis {
    Ancestor,
    AncestorOrSelf,
    Attribute,
    Child,
    Descendant,
    DescendantOrSelf,
    Following,
    FollowingSibling,
    Namespace,
    Parent,
    Preceding,
    PrecedingSibling,
    Itself,
}

impl Axis {
    fn from(name: &[u8]) -> Option<Axis> {
        Some(match name {
            b"ancestor" => Axis::Ancestor,
            b"ancestor-or-self" => Axis::AncestorOrSelf,
            b"attribute" => Axis::Attribute,
            b"child" => Axis::Child,
            b"descendant" => Axis::Descendant,
            b"descendant-or-self" => Axis::DescendantOrSelf,
            b"following" => Axis::Following,
            b"following-sibling" => Axis::FollowingSibling,
            b"namespace" => Axis::Namespace,
            b"parent" => Axis::Parent,
            b"preceding" => Axis::Preceding,
            b"preceding-sibling" => Axis::PrecedingSibling,
            b"self" => Axis::Itself,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug)]
enum Test {
    /// A name test: prefix (resolved at evaluation) and local name ("*": any).
    Name(Option<Vec<u8>>, Vec<u8>),
    Node,
    Text,
    Comment,
    Pi(Option<Vec<u8>>),
}

#[derive(Clone, Debug)]
struct Step {
    axis: Axis,
    test: Test,
    preds: Vec<Expr>,
}

#[derive(Clone, Debug)]
enum Expr {
    Or(Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Cmp(Tok, Box<Expr>, Box<Expr>),
    Arith(Tok, Box<Expr>, Box<Expr>),
    Neg(Box<Expr>),
    Union(Box<Expr>, Box<Expr>),
    /// A location path: from the root (true) or the context node, then steps.
    Path(bool, Vec<Step>),
    /// A filter expression with predicates, then steps.
    Filter(Box<Expr>, Vec<Expr>, Vec<Step>),
    Literal(Vec<u8>),
    Number(f64),
    /// `$name`: PHP binds no variables, so evaluating one is an error.
    Var,
    Call(Option<Vec<u8>>, Vec<u8>, Vec<Expr>),
}

struct Parser {
    toks: Vec<Tok>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.at)
    }

    fn peek_at(&self, n: usize) -> Option<&Tok> {
        self.toks.get(self.at + n)
    }

    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == Some(t) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, t: &Tok) -> Result<(), XError> {
        if self.eat(t) { Ok(()) } else { Err(XError::InvalidExpression) }
    }

    fn expr(&mut self) -> Result<Expr, XError> {
        let mut e = self.and()?;
        while self.eat(&Tok::Op("or")) {
            e = Expr::Or(Box::new(e), Box::new(self.and()?));
        }
        Ok(e)
    }

    fn and(&mut self) -> Result<Expr, XError> {
        let mut e = self.equality()?;
        while self.eat(&Tok::Op("and")) {
            e = Expr::And(Box::new(e), Box::new(self.equality()?));
        }
        Ok(e)
    }

    fn equality(&mut self) -> Result<Expr, XError> {
        let mut e = self.relational()?;
        while let Some(t @ (Tok::Eq | Tok::Neq)) = self.peek().cloned() {
            self.at += 1;
            e = Expr::Cmp(t, Box::new(e), Box::new(self.relational()?));
        }
        Ok(e)
    }

    fn relational(&mut self) -> Result<Expr, XError> {
        let mut e = self.additive()?;
        while let Some(t @ (Tok::Lt | Tok::Le | Tok::Gt | Tok::Ge)) = self.peek().cloned() {
            self.at += 1;
            e = Expr::Cmp(t, Box::new(e), Box::new(self.additive()?));
        }
        Ok(e)
    }

    fn additive(&mut self) -> Result<Expr, XError> {
        let mut e = self.multiplicative()?;
        while let Some(t @ (Tok::Plus | Tok::Minus)) = self.peek().cloned() {
            self.at += 1;
            e = Expr::Arith(t, Box::new(e), Box::new(self.multiplicative()?));
        }
        Ok(e)
    }

    fn multiplicative(&mut self) -> Result<Expr, XError> {
        let mut e = self.unary()?;
        while let Some(t @ (Tok::Mul | Tok::Op("div") | Tok::Op("mod"))) = self.peek().cloned() {
            self.at += 1;
            e = Expr::Arith(t, Box::new(e), Box::new(self.unary()?));
        }
        Ok(e)
    }

    fn unary(&mut self) -> Result<Expr, XError> {
        if self.eat(&Tok::Minus) {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        self.union()
    }

    fn union(&mut self) -> Result<Expr, XError> {
        let mut e = self.path()?;
        while self.eat(&Tok::Pipe) {
            e = Expr::Union(Box::new(e), Box::new(self.path()?));
        }
        Ok(e)
    }

    fn starts_filter(&self) -> bool {
        match self.peek() {
            Some(Tok::Literal(_) | Tok::Number(_) | Tok::Var(..) | Tok::LParen) => true,
            Some(Tok::Name(_, n)) => self.peek_at(1) == Some(&Tok::LParen) && !matches!(n.as_slice(), b"comment" | b"text" | b"node" | b"processing-instruction") && n.as_slice() != b"*",
            _ => false,
        }
    }

    fn path(&mut self) -> Result<Expr, XError> {
        if self.starts_filter() {
            let primary = self.primary()?;
            let mut preds = Vec::new();
            while self.peek() == Some(&Tok::LBracket) {
                preds.push(self.predicate()?);
            }
            let mut steps = Vec::new();
            loop {
                if self.eat(&Tok::Slash) {
                    self.relative(&mut steps)?;
                } else if self.eat(&Tok::DoubleSlash) {
                    steps.push(Step { axis: Axis::DescendantOrSelf, test: Test::Node, preds: Vec::new() });
                    self.relative(&mut steps)?;
                } else {
                    break;
                }
            }
            if preds.is_empty() && steps.is_empty() {
                return Ok(primary);
            }
            return Ok(Expr::Filter(Box::new(primary), preds, steps));
        }
        let mut steps = Vec::new();
        if self.eat(&Tok::Slash) {
            // `/` alone, or followed by a relative path.
            if self.starts_step() {
                self.relative(&mut steps)?;
            }
            return Ok(Expr::Path(true, steps));
        }
        if self.eat(&Tok::DoubleSlash) {
            steps.push(Step { axis: Axis::DescendantOrSelf, test: Test::Node, preds: Vec::new() });
            self.relative(&mut steps)?;
            return Ok(Expr::Path(true, steps));
        }
        if !self.starts_step() {
            return Err(XError::InvalidExpression);
        }
        self.relative(&mut steps)?;
        Ok(Expr::Path(false, steps))
    }

    fn starts_step(&self) -> bool {
        matches!(self.peek(), Some(Tok::Dot | Tok::DotDot | Tok::At | Tok::Name(..)))
    }

    fn relative(&mut self, steps: &mut Vec<Step>) -> Result<(), XError> {
        steps.push(self.step()?);
        loop {
            if self.eat(&Tok::Slash) {
                steps.push(self.step()?);
            } else if self.eat(&Tok::DoubleSlash) {
                steps.push(Step { axis: Axis::DescendantOrSelf, test: Test::Node, preds: Vec::new() });
                steps.push(self.step()?);
            } else {
                return Ok(());
            }
        }
    }

    fn step(&mut self) -> Result<Step, XError> {
        if self.eat(&Tok::Dot) {
            return Ok(Step { axis: Axis::Itself, test: Test::Node, preds: Vec::new() });
        }
        if self.eat(&Tok::DotDot) {
            return Ok(Step { axis: Axis::Parent, test: Test::Node, preds: Vec::new() });
        }
        let mut axis = Axis::Child;
        if self.eat(&Tok::At) {
            axis = Axis::Attribute;
        } else if let (Some(Tok::Name(None, n)), Some(Tok::ColonColon)) = (self.peek().cloned(), self.peek_at(1)) {
            axis = Axis::from(&n).ok_or(XError::InvalidExpression)?;
            self.at += 2;
        }
        let test = match self.peek().cloned() {
            Some(Tok::Name(None, n)) if self.peek_at(1) == Some(&Tok::LParen) && matches!(n.as_slice(), b"comment" | b"text" | b"node" | b"processing-instruction") => {
                self.at += 2;
                let t = match n.as_slice() {
                    b"comment" => Test::Comment,
                    b"text" => Test::Text,
                    b"node" => Test::Node,
                    _ => match self.peek().cloned() {
                        Some(Tok::Literal(l)) => {
                            self.at += 1;
                            Test::Pi(Some(l))
                        }
                        _ => Test::Pi(None),
                    },
                };
                self.expect(&Tok::RParen)?;
                t
            }
            Some(Tok::Name(p, n)) => {
                self.at += 1;
                Test::Name(p, n)
            }
            _ => return Err(XError::InvalidExpression),
        };
        let mut preds = Vec::new();
        while self.peek() == Some(&Tok::LBracket) {
            preds.push(self.predicate()?);
        }
        Ok(Step { axis, test, preds })
    }

    fn predicate(&mut self) -> Result<Expr, XError> {
        self.expect(&Tok::LBracket)?;
        let e = self.expr().map_err(|_| XError::InvalidPredicate)?;
        self.expect(&Tok::RBracket).map_err(|_| XError::InvalidPredicate)?;
        Ok(e)
    }

    fn primary(&mut self) -> Result<Expr, XError> {
        match self.peek().cloned() {
            Some(Tok::Literal(l)) => {
                self.at += 1;
                Ok(Expr::Literal(l))
            }
            Some(Tok::Number(n)) => {
                self.at += 1;
                Ok(Expr::Number(n))
            }
            Some(Tok::Var(..)) => {
                self.at += 1;
                Ok(Expr::Var)
            }
            Some(Tok::LParen) => {
                self.at += 1;
                let e = self.expr()?;
                self.expect(&Tok::RParen)?;
                Ok(e)
            }
            Some(Tok::Name(p, n)) => {
                self.at += 2;
                let mut args = Vec::new();
                if !self.eat(&Tok::RParen) {
                    loop {
                        args.push(self.expr()?);
                        if self.eat(&Tok::RParen) {
                            break;
                        }
                        self.expect(&Tok::Comma)?;
                    }
                }
                Ok(Expr::Call(p, n, args))
            }
            _ => Err(XError::InvalidExpression),
        }
    }
}

/// A parsed expression.
pub struct Compiled(Expr);

pub fn compile(expr: &str) -> Result<Compiled, XError> {
    let toks = tokenize(expr)?;
    if toks.is_empty() {
        return Err(XError::InvalidExpression);
    }
    let mut p = Parser { toks, at: 0 };
    let e = p.expr()?;
    if p.at != p.toks.len() {
        return Err(XError::InvalidExpression);
    }
    Ok(Compiled(e))
}

// ---- evaluation ----

/// A function the caller provides (PHP's registered functions): its
/// namespace URI and local name, and the evaluated arguments.
pub type ExtFn<'a> = dyn FnMut(&[u8], &[u8], Vec<XValue>) -> Option<Result<XValue, XError>> + 'a;

pub struct Eval<'a, 'f> {
    doc: &'a Doc,
    /// Prefix bindings: registered ones, then the context node's in-scope ones.
    ns: &'a [(Vec<u8>, Vec<u8>)],
    ext: Option<&'f mut ExtFn<'f>>,
    order: Option<HashMap<Id, usize>>,
}

struct Ctx {
    node: Id,
    pos: usize,
    size: usize,
}

impl<'a, 'f> Eval<'a, 'f> {
    pub fn new(doc: &'a Doc, ns: &'a [(Vec<u8>, Vec<u8>)], ext: Option<&'f mut ExtFn<'f>>) -> Eval<'a, 'f> {
        Eval { doc, ns, ext, order: None }
    }

    pub fn run(&mut self, c: &Compiled, context: Id) -> Result<XValue, XError> {
        self.eval(&c.0, &Ctx { node: context, pos: 1, size: 1 })
    }

    fn resolve(&self, prefix: &[u8]) -> Result<Vec<u8>, XError> {
        if prefix == b"xml" {
            return Ok(crate::parser::XML_NS.to_vec());
        }
        self.ns.iter().find(|(p, _)| p == prefix).map(|(_, u)| u.clone()).ok_or(XError::UndefinedPrefix)
    }

    fn eval(&mut self, e: &Expr, cx: &Ctx) -> Result<XValue, XError> {
        Ok(match e {
            Expr::Or(a, b) => XValue::Bool(self.boolean(a, cx)? || self.boolean(b, cx)?),
            Expr::And(a, b) => XValue::Bool(self.boolean(a, cx)? && self.boolean(b, cx)?),
            Expr::Cmp(op, a, b) => {
                let (x, y) = (self.eval(a, cx)?, self.eval(b, cx)?);
                XValue::Bool(self.compare(op, &x, &y))
            }
            Expr::Arith(op, a, b) => {
                let (xv, yv) = (self.eval(a, cx)?, self.eval(b, cx)?);
                let (x, y) = (self.number_of(&xv), self.number_of(&yv));
                XValue::Num(match op {
                    Tok::Plus => x + y,
                    Tok::Minus => x - y,
                    Tok::Mul => x * y,
                    Tok::Op("div") => x / y,
                    _ => x % y,
                })
            }
            Expr::Neg(a) => {
                let v = self.eval(a, cx)?;
                XValue::Num(-self.number_of(&v))
            }
            Expr::Union(a, b) => {
                let (XValue::Nodes(mut x), XValue::Nodes(y)) = (self.eval(a, cx)?, self.eval(b, cx)?) else { return Err(XError::InvalidType) };
                x.extend(y);
                XValue::Nodes(self.sorted(x))
            }
            Expr::Path(absolute, steps) => {
                let start = if *absolute { self.root_of(cx.node) } else { cx.node };
                XValue::Nodes(self.steps(vec![start], steps)?)
            }
            Expr::Filter(primary, preds, steps) => {
                let XValue::Nodes(mut nodes) = self.eval(primary, cx)? else { return Err(XError::InvalidType) };
                for p in preds {
                    nodes = self.filter(nodes, p)?;
                }
                XValue::Nodes(self.steps(nodes, steps)?)
            }
            Expr::Literal(l) => XValue::Str(l.clone()),
            Expr::Number(n) => XValue::Num(*n),
            Expr::Var => return Err(XError::UndefinedVariable),
            Expr::Call(p, n, args) => self.call(p.as_deref(), n, args, cx)?,
        })
    }

    fn boolean(&mut self, e: &Expr, cx: &Ctx) -> Result<bool, XError> {
        let v = self.eval(e, cx)?;
        Ok(to_bool(&v))
    }

    /// The root a node hangs from: the document, or a detached subtree's top.
    fn root_of(&self, mut id: Id) -> Id {
        while let Some(p) = self.doc.parent(id) {
            id = p;
        }
        id
    }

    /// Document order positions, from the root of the tree.
    fn order(&mut self) -> &HashMap<Id, usize> {
        if self.order.is_none() {
            let mut map = HashMap::new();
            let doc = self.doc;
            fn walk(d: &Doc, id: Id, map: &mut HashMap<Id, usize>) {
                let n = map.len();
                map.insert(id, n);
                for &a in &d.node(id).attrs {
                    let n = map.len();
                    map.insert(a, n);
                }
                for &c in &d.node(id).children {
                    walk(d, c, map);
                }
            }
            walk(doc, DOC, &mut map);
            // Detached nodes after the document, in id order.
            self.order = Some(map);
        }
        self.order.as_ref().expect("made")
    }

    fn sorted(&mut self, mut nodes: Vec<Id>) -> Vec<Id> {
        let order = self.order();
        nodes.sort_by_key(|id| (order.get(id).copied().unwrap_or(usize::MAX), *id));
        nodes.dedup();
        nodes
    }

    fn steps(&mut self, mut nodes: Vec<Id>, steps: &[Step]) -> Result<Vec<Id>, XError> {
        for step in steps {
            let mut out = Vec::new();
            for &n in &nodes {
                let mut selected: Vec<Id> = Vec::new();
                for c in self.axis(step.axis, n) {
                    if self.matches(step, c)? {
                        selected.push(c);
                    }
                }
                for p in &step.preds {
                    selected = self.filter(selected, p)?;
                }
                out.extend(selected);
            }
            nodes = self.sorted(out);
        }
        Ok(nodes)
    }

    /// Keeps the nodes the predicate holds for, with proximity positions
    /// in the order given (axis order).
    fn filter(&mut self, nodes: Vec<Id>, pred: &Expr) -> Result<Vec<Id>, XError> {
        let size = nodes.len();
        let mut out = Vec::new();
        for (i, &n) in nodes.iter().enumerate() {
            let v = self.eval(pred, &Ctx { node: n, pos: i + 1, size })?;
            let keep = match v {
                XValue::Num(x) => x == (i + 1) as f64,
                v => to_bool(&v),
            };
            if keep {
                out.push(n);
            }
        }
        Ok(out)
    }

    /// The nodes on `axis` from `n`, in axis order.
    fn axis(&self, axis: Axis, n: Id) -> Vec<Id> {
        let d = self.doc;
        let is_attr = d.node(n).kind == Kind::Attribute;
        let mut out = Vec::new();
        match axis {
            Axis::Itself => out.push(n),
            Axis::Child => {
                if !is_attr {
                    out.extend(d.node(n).children.iter().copied());
                }
            }
            Axis::Attribute => {
                if d.node(n).kind == Kind::Element {
                    out.extend(d.node(n).attrs.iter().copied());
                }
            }
            Axis::Parent => out.extend(d.parent(n)),
            Axis::Ancestor | Axis::AncestorOrSelf => {
                if axis == Axis::AncestorOrSelf {
                    out.push(n);
                }
                let mut at = d.parent(n);
                while let Some(p) = at {
                    out.push(p);
                    at = d.parent(p);
                }
            }
            Axis::Descendant | Axis::DescendantOrSelf => {
                if axis == Axis::DescendantOrSelf {
                    out.push(n);
                }
                if !is_attr {
                    fn walk(d: &Doc, id: Id, out: &mut Vec<Id>) {
                        for &c in &d.node(id).children {
                            out.push(c);
                            walk(d, c, out);
                        }
                    }
                    walk(d, n, &mut out);
                }
            }
            Axis::FollowingSibling | Axis::PrecedingSibling => {
                if !is_attr && let Some(p) = d.parent(n) {
                    let kids = &d.node(p).children;
                    let i = kids.iter().position(|&c| c == n).unwrap_or(0);
                    if axis == Axis::FollowingSibling {
                        out.extend(kids[i + 1..].iter().copied());
                    } else {
                        out.extend(kids[..i].iter().rev().copied());
                    }
                }
            }
            Axis::Following => {
                // After n in document order, not its descendants.
                let mut at = n;
                loop {
                    let start = if d.node(at).kind == Kind::Attribute { d.parent(at) } else { Some(at) };
                    let Some(s) = start else { break };
                    if s != at {
                        // From an attribute: the element's descendants follow it.
                        fn walk(d: &Doc, id: Id, out: &mut Vec<Id>) {
                            for &c in &d.node(id).children {
                                out.push(c);
                                walk(d, c, out);
                            }
                        }
                        walk(d, s, &mut out);
                    }
                    let Some(p) = d.parent(s) else { break };
                    let kids = &d.node(p).children;
                    let i = kids.iter().position(|&c| c == s).unwrap_or(0);
                    for &sib in &kids[i + 1..] {
                        out.push(sib);
                        fn walk(d: &Doc, id: Id, out: &mut Vec<Id>) {
                            for &c in &d.node(id).children {
                                out.push(c);
                                walk(d, c, out);
                            }
                        }
                        walk(d, sib, &mut out);
                    }
                    at = p;
                    if d.node(at).kind == Kind::Document {
                        break;
                    }
                }
            }
            Axis::Preceding => {
                // Before n in document order, not its ancestors; reverse order.
                let ancestors: Vec<Id> = {
                    let mut v = Vec::new();
                    let mut at = d.parent(n);
                    while let Some(p) = at {
                        v.push(p);
                        at = d.parent(p);
                    }
                    v
                };
                let root = self.root_of(n);
                let mut all = Vec::new();
                fn walk(d: &Doc, id: Id, out: &mut Vec<Id>) {
                    out.push(id);
                    for &c in &d.node(id).children {
                        walk(d, c, out);
                    }
                }
                walk(d, root, &mut all);
                let me = if is_attr { d.parent(n).unwrap_or(n) } else { n };
                let stop = all.iter().position(|&x| x == me).unwrap_or(0);
                out.extend(all[..stop].iter().rev().copied().filter(|x| !ancestors.contains(x) && *x != me));
            }
            Axis::Namespace => {}
        }
        out
    }

    fn matches(&self, step: &Step, id: Id) -> Result<bool, XError> {
        let n = self.doc.node(id);
        Ok(match &step.test {
            Test::Node => true,
            Test::Text => matches!(n.kind, Kind::Text | Kind::CData),
            Test::Comment => n.kind == Kind::Comment,
            Test::Pi(target) => n.kind == Kind::Pi && target.as_ref().is_none_or(|t| *t == n.name),
            Test::Name(prefix, local) => {
                let principal = if step.axis == Axis::Attribute { Kind::Attribute } else { Kind::Element };
                if n.kind != principal {
                    return Ok(false);
                }
                let uri = match prefix {
                    Some(p) => Some(self.resolve(p)?),
                    None => None,
                };
                if local == b"*" {
                    return Ok(prefix.is_none() || n.ns == uri);
                }
                n.name == *local && n.ns == uri
            }
        })
    }

    fn string_value(&self, id: Id) -> Vec<u8> {
        self.doc.text_content(id)
    }

    fn string_of(&self, v: &XValue) -> Vec<u8> {
        match v {
            XValue::Nodes(n) => n.first().map(|&id| self.string_value(id)).unwrap_or_default(),
            XValue::Bool(b) => if *b { b"true".to_vec() } else { b"false".to_vec() },
            XValue::Num(x) => format_number(*x).into_bytes(),
            XValue::Str(s) => s.clone(),
        }
    }

    fn number_of(&self, v: &XValue) -> f64 {
        match v {
            XValue::Num(x) => *x,
            XValue::Bool(b) => f64::from(u8::from(*b)),
            v => parse_number(&self.string_of(v)),
        }
    }

    fn compare(&self, op: &Tok, x: &XValue, y: &XValue) -> bool {
        let eq = matches!(op, Tok::Eq | Tok::Neq);
        match (x, y) {
            (XValue::Nodes(a), XValue::Nodes(b)) => {
                let bs: Vec<Vec<u8>> = b.iter().map(|&n| self.string_value(n)).collect();
                a.iter().any(|&n| {
                    let s = self.string_value(n);
                    bs.iter().any(|t| if eq { cmp_eq(op, &s, t) } else { cmp_num(op, parse_number(&s), parse_number(t)) })
                })
            }
            (XValue::Nodes(a), other) | (other, XValue::Nodes(a)) => {
                let flipped = matches!(x, XValue::Nodes(_)).then_some(()).is_none();
                let op = if flipped { flip(op) } else { op.clone() };
                match other {
                    XValue::Bool(b) => {
                        let nb = !a.is_empty();
                        if eq { (nb == *b) == (op == Tok::Eq) } else { cmp_num(&op, f64::from(u8::from(nb)), f64::from(u8::from(*b))) }
                    }
                    XValue::Num(n) => a.iter().any(|&id| cmp_num(&op, parse_number(&self.string_value(id)), *n)),
                    XValue::Str(s) => a.iter().any(|&id| {
                        let v = self.string_value(id);
                        if eq { cmp_eq(&op, &v, s) } else { cmp_num(&op, parse_number(&v), parse_number(s)) }
                    }),
                    XValue::Nodes(_) => unreachable!("handled above"),
                }
            }
            _ if eq => {
                let same = if matches!(x, XValue::Bool(_)) || matches!(y, XValue::Bool(_)) {
                    to_bool(x) == to_bool(y)
                } else if matches!(x, XValue::Num(_)) || matches!(y, XValue::Num(_)) {
                    self.number_of(x) == self.number_of(y)
                } else {
                    self.string_of(x) == self.string_of(y)
                };
                same == (*op == Tok::Eq)
            }
            _ => cmp_num(op, self.number_of(x), self.number_of(y)),
        }
    }

    fn call(&mut self, prefix: Option<&[u8]>, name: &[u8], args: &[Expr], cx: &Ctx) -> Result<XValue, XError> {
        if let Some(p) = prefix {
            let uri = self.resolve(p)?;
            let mut values = Vec::new();
            for a in args {
                values.push(self.eval(a, cx)?);
            }
            return match self.ext.as_mut().and_then(|f| f(&uri, name, values)) {
                Some(r) => r,
                None => Err(XError::UnregisteredFunction),
            };
        }
        let mut v = Vec::new();
        for a in args {
            v.push(self.eval(a, cx)?);
        }
        let n = v.len();
        let arity = |lo: usize, hi: usize| if n < lo || n > hi { Err(XError::InvalidArity) } else { Ok(()) };
        let ctx_str = |me: &Self| me.string_value(cx.node);
        let str_arg = |me: &Self, i: usize| me.string_of(&v[i]);
        Ok(match name {
            b"last" => {
                arity(0, 0)?;
                XValue::Num(cx.size as f64)
            }
            b"position" => {
                arity(0, 0)?;
                XValue::Num(cx.pos as f64)
            }
            b"count" => {
                arity(1, 1)?;
                match &v[0] {
                    XValue::Nodes(ns) => XValue::Num(ns.len() as f64),
                    _ => return Err(XError::InvalidType),
                }
            }
            b"id" => {
                arity(1, 1)?;
                let wanted: Vec<Vec<u8>> = match &v[0] {
                    XValue::Nodes(ns) => ns.iter().flat_map(|&id| split_ws(&self.string_value(id))).collect(),
                    other => split_ws(&self.string_of(other)),
                };
                let mut found = Vec::new();
                let mut all = Vec::new();
                self.doc.descendants(DOC, &mut all);
                for e in all {
                    if let Some(a) = self.doc.attr_ns(e, Some(crate::parser::XML_NS), b"id")
                        && wanted.iter().any(|w| *w == self.doc.node(a).value)
                    {
                        found.push(e);
                    }
                }
                XValue::Nodes(self.sorted(found))
            }
            b"local-name" | b"namespace-uri" | b"name" => {
                arity(0, 1)?;
                let target = if n == 0 {
                    Some(cx.node)
                } else {
                    match &v[0] {
                        XValue::Nodes(ns) => ns.first().copied(),
                        _ => return Err(XError::InvalidType),
                    }
                };
                let s = match target {
                    None => Vec::new(),
                    Some(id) => {
                        let node = self.doc.node(id);
                        match (name, node.kind) {
                            (b"local-name", Kind::Element | Kind::Attribute | Kind::Pi) => node.name.clone(),
                            (b"namespace-uri", Kind::Element | Kind::Attribute) => node.ns.clone().unwrap_or_default(),
                            (b"name", Kind::Element | Kind::Attribute) => node.qname(),
                            (b"name", Kind::Pi) => node.name.clone(),
                            _ => Vec::new(),
                        }
                    }
                };
                XValue::Str(s)
            }
            b"string" => {
                arity(0, 1)?;
                XValue::Str(if n == 0 { ctx_str(self) } else { str_arg(self, 0) })
            }
            b"concat" => {
                if n < 2 {
                    return Err(XError::InvalidArity);
                }
                XValue::Str((0..n).flat_map(|i| str_arg(self, i)).collect())
            }
            b"starts-with" => {
                arity(2, 2)?;
                XValue::Bool(str_arg(self, 0).starts_with(&str_arg(self, 1)))
            }
            b"contains" => {
                arity(2, 2)?;
                let (h, ndl) = (str_arg(self, 0), str_arg(self, 1));
                XValue::Bool(ndl.is_empty() || h.windows(ndl.len()).any(|w| w == ndl.as_slice()))
            }
            b"substring-before" | b"substring-after" => {
                arity(2, 2)?;
                let (h, ndl) = (str_arg(self, 0), str_arg(self, 1));
                let at = if ndl.is_empty() { Some(0) } else { h.windows(ndl.len()).position(|w| w == ndl.as_slice()) };
                XValue::Str(match (at, name) {
                    (Some(i), b"substring-before") => h[..i].to_vec(),
                    (Some(i), _) => h[i + ndl.len()..].to_vec(),
                    (None, _) => Vec::new(),
                })
            }
            b"substring" => {
                arity(2, 3)?;
                let s = String::from_utf8_lossy(&str_arg(self, 0)).into_owned();
                let chars: Vec<char> = s.chars().collect();
                let start = xround(self.number_of(&v[1]));
                let end = if n == 3 { start + xround(self.number_of(&v[2])) } else { f64::INFINITY };
                let out: String = chars.iter().enumerate().filter(|(i, _)| {
                    let p = (*i + 1) as f64;
                    p >= start && p < end
                }).map(|(_, c)| *c).collect();
                XValue::Str(out.into_bytes())
            }
            b"string-length" => {
                arity(0, 1)?;
                let s = if n == 0 { ctx_str(self) } else { str_arg(self, 0) };
                XValue::Num(String::from_utf8_lossy(&s).chars().count() as f64)
            }
            b"normalize-space" => {
                arity(0, 1)?;
                let s = if n == 0 { ctx_str(self) } else { str_arg(self, 0) };
                XValue::Str(split_ws(&s).join(&b' '))
            }
            b"translate" => {
                arity(3, 3)?;
                let s = String::from_utf8_lossy(&str_arg(self, 0)).into_owned();
                let from: Vec<char> = String::from_utf8_lossy(&str_arg(self, 1)).chars().collect();
                let to: Vec<char> = String::from_utf8_lossy(&str_arg(self, 2)).chars().collect();
                let out: String = s
                    .chars()
                    .filter_map(|c| match from.iter().position(|&f| f == c) {
                        Some(i) => to.get(i).copied(),
                        None => Some(c),
                    })
                    .collect();
                XValue::Str(out.into_bytes())
            }
            b"boolean" => {
                arity(1, 1)?;
                XValue::Bool(to_bool(&v[0]))
            }
            b"not" => {
                arity(1, 1)?;
                XValue::Bool(!to_bool(&v[0]))
            }
            b"true" => {
                arity(0, 0)?;
                XValue::Bool(true)
            }
            b"false" => {
                arity(0, 0)?;
                XValue::Bool(false)
            }
            b"lang" => {
                arity(1, 1)?;
                let want = str_arg(self, 0).to_ascii_lowercase();
                let mut at = Some(cx.node);
                let mut lang = None;
                while let Some(id) = at {
                    if self.doc.node(id).kind == Kind::Element
                        && let Some(a) = self.doc.attr_ns(id, Some(crate::parser::XML_NS), b"lang")
                    {
                        lang = Some(self.doc.node(a).value.to_ascii_lowercase());
                        break;
                    }
                    at = self.doc.parent(id);
                }
                XValue::Bool(lang.is_some_and(|l| l == want || (l.starts_with(&want) && l.get(want.len()) == Some(&b'-'))))
            }
            b"number" => {
                arity(0, 1)?;
                XValue::Num(if n == 0 { parse_number(&ctx_str(self)) } else { self.number_of(&v[0]) })
            }
            b"sum" => {
                arity(1, 1)?;
                match &v[0] {
                    XValue::Nodes(ns) => XValue::Num(ns.iter().map(|&id| parse_number(&self.string_value(id))).sum()),
                    _ => return Err(XError::InvalidType),
                }
            }
            b"floor" => {
                arity(1, 1)?;
                XValue::Num(self.number_of(&v[0]).floor())
            }
            b"ceiling" => {
                arity(1, 1)?;
                XValue::Num(self.number_of(&v[0]).ceil())
            }
            b"round" => {
                arity(1, 1)?;
                XValue::Num(xround(self.number_of(&v[0])))
            }
            _ => return Err(XError::UnregisteredFunction),
        })
    }
}

fn flip(op: &Tok) -> Tok {
    match op {
        Tok::Lt => Tok::Gt,
        Tok::Le => Tok::Ge,
        Tok::Gt => Tok::Lt,
        Tok::Ge => Tok::Le,
        t => t.clone(),
    }
}

fn cmp_eq(op: &Tok, a: &[u8], b: &[u8]) -> bool {
    (a == b) == (*op == Tok::Eq)
}

fn cmp_num(op: &Tok, a: f64, b: f64) -> bool {
    match op {
        Tok::Eq => a == b,
        Tok::Neq => a != b,
        Tok::Lt => a < b,
        Tok::Le => a <= b,
        Tok::Gt => a > b,
        _ => a >= b,
    }
}

pub fn to_bool(v: &XValue) -> bool {
    match v {
        XValue::Nodes(n) => !n.is_empty(),
        XValue::Bool(b) => *b,
        XValue::Num(x) => *x != 0.0 && !x.is_nan(),
        XValue::Str(s) => !s.is_empty(),
    }
}

fn split_ws(s: &[u8]) -> Vec<Vec<u8>> {
    s.split(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r')).filter(|p| !p.is_empty()).map(<[u8]>::to_vec).collect()
}

/// XPath's round: halves up; NaN and infinities as they are.
fn xround(x: f64) -> f64 {
    if x.is_nan() || x.is_infinite() {
        return x;
    }
    let r = (x + 0.5).floor();
    if r == 0.0 && x < 0.0 { -0.0 } else { r }
}

/// xmlXPathStringEvalNumber: optional blanks, an optional '-', digits with
/// an optional fraction, optional blanks; anything else is NaN.
pub fn parse_number(s: &[u8]) -> f64 {
    let t = std::str::from_utf8(s).unwrap_or("").trim_matches(|c| matches!(c, ' ' | '\t' | '\n' | '\r'));
    let body = t.strip_prefix('-').unwrap_or(t);
    // digits, an optional fraction, an optional exponent (libxml2 takes one).
    let (mant, exp) = match body.find(['e', 'E']) {
        Some(i) => (&body[..i], Some(&body[i + 1..])),
        None => (body, None),
    };
    let valid = !mant.is_empty()
        && mant.chars().all(|c| c.is_ascii_digit() || c == '.')
        && mant.matches('.').count() <= 1
        && mant != "."
        && exp.is_none_or(|e| {
            let d = e.strip_prefix(['+', '-']).unwrap_or(e);
            !d.is_empty() && d.chars().all(|c| c.is_ascii_digit())
        });
    if !valid {
        return f64::NAN;
    }
    t.parse().unwrap_or(f64::NAN)
}

/// xmlXPathFormatNumber.
pub fn format_number(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if x == 0.0 {
        return "0".into();
    }
    // Integers in int's range print as integers.
    if x == x.trunc() && x > f64::from(i32::MIN) && x < f64::from(i32::MAX) {
        return format!("{}", x as i64);
    }
    let a = x.abs();
    if (1e-5..1e9).contains(&a) {
        // 15 digits: significant ones from 1 up, decimal places below 1;
        // trailing zeros dropped.
        let frac = if a >= 1.0 { (15 - (a.log10().floor() as i32 + 1)).max(0) as usize } else { 15 };
        let s = format!("{x:.frac$}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        s.to_string()
    } else {
        let s = format!("{x:.14e}");
        let (mant, exp) = s.split_once('e').unwrap_or((&s, "0"));
        let mant = mant.trim_end_matches('0').trim_end_matches('.');
        let e: i32 = exp.parse().unwrap_or(0);
        format!("{mant}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::build::parse;

    fn doc(xml: &str) -> Doc {
        let mut d = Doc::new(b"DOMDocument".to_vec());
        parse(&mut d, xml.as_bytes().to_vec(), None, 0);
        d
    }

    fn run(d: &Doc, expr: &str) -> XValue {
        let ns = vec![(b"p".to_vec(), b"urn:p".to_vec())];
        let c = compile(expr).unwrap();
        Eval::new(d, &ns, None).run(&c, d.root().unwrap()).unwrap()
    }

    fn names(d: &Doc, v: XValue) -> Vec<String> {
        let XValue::Nodes(n) = v else { panic!("not nodes: {v:?}") };
        n.into_iter().map(|id| String::from_utf8(crate::dom::node::node_name(d, id)).unwrap()).collect()
    }

    const XML: &str = "<r a='1'><b id='x'>one</b><c><b>two</b><d/></c><b>3</b><p:e xmlns:p='urn:p'/></r>";

    #[test]
    fn paths_and_axes_in_document_order() {
        let d = doc(XML);
        assert_eq!(names(&d, run(&d, "b")), ["b", "b"]);
        assert_eq!(names(&d, run(&d, "//b")), ["b", "b", "b"]);
        assert_eq!(names(&d, run(&d, "/r/c/*")), ["b", "d"]);
        assert_eq!(names(&d, run(&d, "@*")), ["a"]);
        assert_eq!(names(&d, run(&d, "//d/ancestor::*")), ["r", "c"]);
        assert_eq!(names(&d, run(&d, "c/following-sibling::*")), ["b", "p:e"]);
        assert_eq!(names(&d, run(&d, "//d/preceding::b")), ["b", "b"]);
        assert_eq!(names(&d, run(&d, "b | c | b")), ["b", "c", "b"]);
        assert_eq!(names(&d, run(&d, "p:e")), ["p:e"]);
        assert_eq!(names(&d, run(&d, "//text()/..")), ["b", "b", "b"]);
    }

    #[test]
    fn predicates_count_in_axis_order() {
        let d = doc(XML);
        assert_eq!(run(&d, "string(b[2])"), XValue::Str(b"3".to_vec()));
        assert_eq!(run(&d, "string(//d/ancestor::*[1])"), XValue::Str(b"two".to_vec()));
        assert_eq!(names(&d, run(&d, "//b[last()]")), ["b", "b"]);
        assert_eq!(names(&d, run(&d, "*[@id='x']")), ["b"]);
        assert_eq!(names(&d, run(&d, "(//b)[last()]")), ["b"]);
    }

    #[test]
    fn functions_and_comparisons() {
        let d = doc(XML);
        assert_eq!(run(&d, "count(//b)"), XValue::Num(3.0));
        assert_eq!(run(&d, "//b = 'two'"), XValue::Bool(true));
        assert_eq!(run(&d, "//b != 'two'"), XValue::Bool(true));
        assert_eq!(run(&d, "//b > 2"), XValue::Bool(true));
        assert_eq!(run(&d, "concat('a', 1, true())"), XValue::Str(b"a1true".to_vec()));
        assert_eq!(run(&d, "substring('12345', 1.5, 2.6)"), XValue::Str(b"234".to_vec()));
        assert_eq!(run(&d, "normalize-space('  a  b ')"), XValue::Str(b"a b".to_vec()));
        assert_eq!(run(&d, "translate('bar','abc','ABC')"), XValue::Str(b"BAr".to_vec()));
        assert_eq!(run(&d, "string(1 div 0)"), XValue::Str(b"Infinity".to_vec()));
        assert_eq!(run(&d, "string(0.1 + 0.2)"), XValue::Str(b"0.3".to_vec()));
        assert_eq!(run(&d, "2 * 3 mod 4"), XValue::Num(2.0));
        assert_eq!(run(&d, "local-name(p:e)"), XValue::Str(b"e".to_vec()));
        assert_eq!(run(&d, "sum(//b[. > 0])"), XValue::Num(3.0));
        assert_eq!(run(&d, "round(-2.5)"), XValue::Num(-2.0));
    }

    #[test]
    fn errors_are_libxml2s() {
        assert_eq!(compile("**").err(), Some(XError::InvalidExpression));
        assert!(compile("***").is_ok());
        assert_eq!(compile("'abc").err(), Some(XError::UnfinishedLiteral));
        let d = doc(XML);
        let c = compile("q:x").unwrap();
        assert_eq!(Eval::new(&d, &[], None).run(&c, DOC).err(), Some(XError::UndefinedPrefix));
        let c = compile("nofunc()").unwrap();
        assert_eq!(Eval::new(&d, &[], None).run(&c, DOC).err(), Some(XError::UnregisteredFunction));
    }

    #[test]
    fn numbers_format_as_libxml2() {
        assert_eq!(format_number(3.0), "3");
        assert_eq!(format_number(-0.5), "-0.5");
        // From php:8.4-cli's DOMXPath::evaluate("string(...)").
        assert_eq!(format_number(1.0 / 3.0), "0.333333333333333");
        assert_eq!(format_number(2.0 / 3.0), "0.666666666666667");
        assert_eq!(format_number(1.0 / 7.0 * 1e6), "142857.142857143");
        assert_eq!(format_number(0.00001234), "0.00001234");
        assert_eq!(format_number(12345678901.0), "1.2345678901e+10");
        assert_eq!(format_number(1e20), "1e+20");
        assert_eq!(format_number(0.000001), "1e-06");
        assert_eq!(format_number(2.5), "2.5");
        assert_eq!(parse_number(b" 12 "), 12.0);
        assert!(parse_number(b"+1").is_nan());
        assert_eq!(parse_number(b"1e3"), 1000.0);
        assert_eq!(compile("1e3").map(|c| matches!(c.0, Expr::Number(n) if n == 1000.0)).ok(), Some(true));
    }
}
