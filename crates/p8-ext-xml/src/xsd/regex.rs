//! XML Schema regular expressions (Appendix F of XSD part 2): implicitly
//! anchored, with \i \c \d \s \w, \p{..} categories and character class
//! subtraction. Compiled to a small NFA and run as a state-set simulation,
//! so matching never backtracks.

#[derive(Debug, Clone)]
enum Item {
    Range(char, char),
    Prop(String),
    Class(Class),
}

#[derive(Debug, Clone)]
pub(crate) struct Class {
    negated: bool,
    items: Vec<Item>,
    minus: Option<Box<Class>>,
}

#[derive(Debug, Clone)]
enum Ast {
    Empty,
    Set(Class),
    Any,
    Cat(Vec<Ast>),
    Alt(Vec<Ast>),
    Rep(Box<Ast>, u32, Option<u32>),
}

enum Node {
    Set(Class, usize),
    Any(usize),
    Split(usize, usize),
    Jump(usize),
    Match,
}

pub(crate) struct Regex {
    nodes: Vec<Node>,
    start: usize,
}

/// Counted repeats are unrolled; beyond this the extra copies are dropped.
const UNROLL_CAP: u32 = 256;

struct Parser<'a> {
    s: &'a [char],
    i: usize,
}

type PResult<T> = Result<T, ()>;

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn alt(&mut self) -> PResult<Ast> {
        let mut branches = vec![self.cat()?];
        while self.peek() == Some('|') {
            self.i += 1;
            branches.push(self.cat()?);
        }
        Ok(if branches.len() == 1 { branches.pop().unwrap_or(Ast::Empty) } else { Ast::Alt(branches) })
    }

    fn cat(&mut self) -> PResult<Ast> {
        let mut parts = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let atom = self.atom()?;
            parts.push(self.quantifier(atom)?);
        }
        Ok(match parts.len() {
            0 => Ast::Empty,
            1 => parts.pop().unwrap_or(Ast::Empty),
            _ => Ast::Cat(parts),
        })
    }

    fn quantifier(&mut self, atom: Ast) -> PResult<Ast> {
        let (min, max) = match self.peek() {
            Some('?') => (0, Some(1)),
            Some('*') => (0, None),
            Some('+') => (1, None),
            Some('{') => {
                self.i += 1;
                let min = self.number()?;
                let max = if self.peek() == Some(',') {
                    self.i += 1;
                    if self.peek() == Some('}') { None } else { Some(self.number()?) }
                } else {
                    Some(min)
                };
                if self.peek() != Some('}') {
                    return Err(());
                }
                if max.is_some_and(|m| m < min) {
                    return Err(());
                }
                (min, max)
            }
            _ => return Ok(atom),
        };
        self.i += 1;
        Ok(Ast::Rep(Box::new(atom), min, max))
    }

    fn number(&mut self) -> PResult<u32> {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        if start == self.i {
            return Err(());
        }
        self.s[start..self.i].iter().collect::<String>().parse().map_err(|_| ())
    }

    fn atom(&mut self) -> PResult<Ast> {
        let c = self.peek().ok_or(())?;
        self.i += 1;
        match c {
            '(' => {
                let inner = self.alt()?;
                if self.peek() != Some(')') {
                    return Err(());
                }
                self.i += 1;
                Ok(inner)
            }
            '.' => Ok(Ast::Any),
            '[' => Ok(Ast::Set(self.class()?)),
            '\\' => Ok(Ast::Set(self.escape()?)),
            '?' | '*' | '+' | '{' | '}' | ']' => Err(()),
            c => Ok(Ast::Set(single(c))),
        }
    }

    /// After a backslash: one escape as a class.
    fn escape(&mut self) -> PResult<Class> {
        let c = self.peek().ok_or(())?;
        self.i += 1;
        Ok(match c {
            'n' => single('\n'),
            'r' => single('\r'),
            't' => single('\t'),
            '\\' | '|' | '.' | '-' | '^' | '?' | '*' | '+' | '{' | '}' | '(' | ')' | '[' | ']' => single(c),
            'd' | 'D' | 's' | 'S' | 'w' | 'W' | 'i' | 'I' | 'c' | 'C' => multi(c),
            'p' | 'P' => {
                if self.peek() != Some('{') {
                    return Err(());
                }
                self.i += 1;
                let start = self.i;
                while self.peek().is_some_and(|c| c != '}') {
                    self.i += 1;
                }
                if self.peek() != Some('}') {
                    return Err(());
                }
                let name: String = self.s[start..self.i].iter().collect();
                self.i += 1;
                if !known_property(&name) {
                    return Err(());
                }
                Class { negated: c == 'P', items: vec![Item::Class(property(&name))], minus: None }
            }
            _ => return Err(()),
        })
    }

    /// After '[': a character class group.
    fn class(&mut self) -> PResult<Class> {
        let mut negated = false;
        if self.peek() == Some('^') {
            negated = true;
            self.i += 1;
        }
        let mut items = Vec::new();
        let mut minus = None;
        loop {
            let c = self.peek().ok_or(())?;
            if c == ']' {
                self.i += 1;
                break;
            }
            if c == '-' && self.s.get(self.i + 1) == Some(&'[') && !items.is_empty() {
                self.i += 2;
                minus = Some(Box::new(self.class()?));
                if self.peek() != Some(']') {
                    return Err(());
                }
                continue;
            }
            let lo = match self.class_char()? {
                Ok(ch) => ch,
                Err(class) => {
                    items.push(Item::Class(class));
                    continue;
                }
            };
            if self.peek() == Some('-') && self.s.get(self.i + 1).is_some_and(|&n| n != ']' && n != '[') {
                self.i += 1;
                match self.class_char()? {
                    Ok(hi) => {
                        if hi < lo {
                            return Err(());
                        }
                        items.push(Item::Range(lo, hi));
                    }
                    Err(_) => return Err(()),
                }
            } else {
                items.push(Item::Range(lo, lo));
            }
        }
        Ok(Class { negated, items, minus })
    }

    /// One class member: a character or a multi-character escape.
    fn class_char(&mut self) -> PResult<Result<char, Class>> {
        let c = self.peek().ok_or(())?;
        self.i += 1;
        if c == '\\' {
            let class = self.escape()?;
            if let Some(ch) = single_char(&class) {
                return Ok(Ok(ch));
            }
            return Ok(Err(class));
        }
        if c == '[' {
            return Err(());
        }
        Ok(Ok(c))
    }
}

fn single(c: char) -> Class {
    Class { negated: false, items: vec![Item::Range(c, c)], minus: None }
}

fn single_char(c: &Class) -> Option<char> {
    match (c.negated, c.items.as_slice(), &c.minus) {
        (false, [Item::Range(a, b)], None) if a == b => Some(*a),
        _ => None,
    }
}

fn multi(c: char) -> Class {
    let (base, negated) = match c {
        'd' => (Item::Class(property("Nd")), false),
        'D' => (Item::Class(property("Nd")), true),
        's' => (Item::Class(Class { negated: false, items: vec![' ', '\t', '\n', '\r'].into_iter().map(|c| Item::Range(c, c)).collect(), minus: None }), false),
        'S' => (Item::Class(Class { negated: false, items: vec![' ', '\t', '\n', '\r'].into_iter().map(|c| Item::Range(c, c)).collect(), minus: None }), true),
        'w' => (Item::Class(Class { negated: false, items: vec![Item::Class(property("P")), Item::Class(property("Z")), Item::Class(property("C"))], minus: None }), true),
        'W' => (Item::Class(Class { negated: false, items: vec![Item::Class(property("P")), Item::Class(property("Z")), Item::Class(property("C"))], minus: None }), false),
        'i' => (Item::Class(name_class(true)), false),
        'I' => (Item::Class(name_class(true)), true),
        'c' => (Item::Class(name_class(false)), false),
        _ => (Item::Class(name_class(false)), true),
    };
    Class { negated, items: vec![base], minus: None }
}

/// \i (initial name characters) or \c (name characters): represented as a
/// class holding one predicate through property names.
fn name_class(initial: bool) -> Class {
    Class { negated: false, items: vec![Item::Class(property(if initial { "#i" } else { "#c" }))], minus: None }
}

const CATEGORIES: &[&str] = &[
    "L", "Lu", "Ll", "Lt", "Lm", "Lo", "M", "Mn", "Mc", "Me", "N", "Nd", "Nl", "No", "P", "Pc", "Pd", "Ps", "Pe", "Pi", "Pf", "Po", "Z", "Zs", "Zl", "Zp", "S", "Sm", "Sc", "Sk", "So", "C", "Cc", "Cf", "Co", "Cn",
];

fn known_property(name: &str) -> bool {
    CATEGORIES.contains(&name) || block(name).is_some()
}

/// The ranges of a \p{IsXxx} block we know.
fn block(name: &str) -> Option<(char, char)> {
    let n = name.strip_prefix("Is")?;
    Some(match n {
        "BasicLatin" => ('\u{0}', '\u{7F}'),
        "Latin-1Supplement" => ('\u{80}', '\u{FF}'),
        "LatinExtended-A" => ('\u{100}', '\u{17F}'),
        "LatinExtended-B" => ('\u{180}', '\u{24F}'),
        "Greek" | "GreekandCoptic" => ('\u{370}', '\u{3FF}'),
        "Cyrillic" => ('\u{400}', '\u{4FF}'),
        "Hebrew" => ('\u{590}', '\u{5FF}'),
        "Arabic" => ('\u{600}', '\u{6FF}'),
        "Hiragana" => ('\u{3040}', '\u{309F}'),
        "Katakana" => ('\u{30A0}', '\u{30FF}'),
        "CJKUnifiedIdeographs" => ('\u{4E00}', '\u{9FFF}'),
        "HangulSyllables" => ('\u{AC00}', '\u{D7A3}'),
        "GeneralPunctuation" => ('\u{2000}', '\u{206F}'),
        "CurrencySymbols" => ('\u{20A0}', '\u{20CF}'),
        _ => return None,
    })
}

fn property(name: &str) -> Class {
    Class { negated: false, items: vec![Item::Prop(name.to_string())], minus: None }
}

fn category_matches(name: &str, c: char) -> bool {
    match name {
        "#i" => is_name_start(c),
        "#c" => is_name_char(c),
        "L" => c.is_alphabetic() && !is_mark_like(c),
        "Lu" => c.is_uppercase(),
        "Ll" => c.is_lowercase(),
        "Lt" | "Lm" | "Lo" => c.is_alphabetic() && !c.is_uppercase() && !c.is_lowercase(),
        "M" | "Mn" | "Mc" | "Me" => is_mark_like(c),
        "N" => c.is_numeric(),
        "Nd" => is_decimal_digit(c),
        "Nl" | "No" => c.is_numeric() && !is_decimal_digit(c),
        "P" | "Pc" | "Pd" | "Ps" | "Pe" | "Pi" | "Pf" | "Po" => is_punct(name, c),
        "Z" => c == ' ' || c == '\u{A0}' || c == '\u{2028}' || c == '\u{2029}' || (c.is_whitespace() && !c.is_control()),
        "Zs" => c == ' ' || c == '\u{A0}' || (c.is_whitespace() && !c.is_control() && c != '\u{2028}' && c != '\u{2029}'),
        "Zl" => c == '\u{2028}',
        "Zp" => c == '\u{2029}',
        "S" | "Sm" | "Sc" | "Sk" | "So" => is_symbol(name, c),
        "C" => c.is_control() || matches!(c, '\u{AD}' | '\u{200B}'..='\u{200F}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}' | '\u{E000}'..='\u{F8FF}'),
        "Cc" => c.is_control(),
        "Cf" => matches!(c, '\u{AD}' | '\u{200B}'..='\u{200F}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}'),
        "Co" => matches!(c, '\u{E000}'..='\u{F8FF}'),
        "Cn" => false,
        other => match block(other) {
            Some((lo, hi)) => c >= lo && c <= hi,
            None => false,
        },
    }
}

fn is_decimal_digit(c: char) -> bool {
    c.is_ascii_digit() || matches!(c, '\u{660}'..='\u{669}' | '\u{6F0}'..='\u{6F9}' | '\u{966}'..='\u{96F}' | '\u{FF10}'..='\u{FF19}')
}

fn is_mark_like(c: char) -> bool {
    matches!(c, '\u{300}'..='\u{36F}' | '\u{483}'..='\u{489}' | '\u{591}'..='\u{5BD}' | '\u{64B}'..='\u{65F}' | '\u{900}'..='\u{903}' | '\u{93A}'..='\u{94F}' | '\u{20D0}'..='\u{20FF}' | '\u{FE20}'..='\u{FE2F}')
}

fn is_punct(name: &str, c: char) -> bool {
    let sub = match c {
        '_' | '\u{203F}' | '\u{2040}' => 'c',
        '-' | '\u{2010}'..='\u{2015}' => 'd',
        '(' | '[' | '{' => 's',
        ')' | ']' | '}' => 'e',
        '\u{AB}' | '\u{2018}' | '\u{201C}' => 'i',
        '\u{BB}' | '\u{2019}' | '\u{201D}' => 'f',
        '!' | '"' | '#' | '%' | '&' | '\'' | '*' | ',' | '.' | '/' | ':' | ';' | '?' | '@' | '\\' | '\u{A1}' | '\u{A7}' | '\u{B6}' | '\u{B7}' | '\u{BF}' | '\u{2020}'..='\u{2027}' | '\u{2030}'..='\u{2038}' | '\u{3001}'..='\u{3003}' => 'o',
        _ => return false,
    };
    match name {
        "P" => true,
        "Pc" => sub == 'c',
        "Pd" => sub == 'd',
        "Ps" => sub == 's',
        "Pe" => sub == 'e',
        "Pi" => sub == 'i',
        "Pf" => sub == 'f',
        _ => sub == 'o',
    }
}

fn is_symbol(name: &str, c: char) -> bool {
    let sub = match c {
        '+' | '<' | '=' | '>' | '|' | '~' | '\u{AC}' | '\u{B1}' | '\u{D7}' | '\u{F7}' | '\u{2200}'..='\u{22FF}' => 'm',
        '$' | '\u{A2}'..='\u{A5}' | '\u{20A0}'..='\u{20CF}' => 'c',
        '^' | '`' | '\u{A8}' | '\u{AF}' | '\u{B4}' | '\u{B8}' => 'k',
        '\u{A6}' | '\u{A9}' | '\u{AE}' | '\u{B0}' | '\u{2100}'..='\u{214F}' | '\u{2190}'..='\u{21FF}' | '\u{2300}'..='\u{23FF}' | '\u{2500}'..='\u{27BF}' => 'o',
        _ => return false,
    };
    match name {
        "S" => true,
        "Sm" => sub == 'm',
        "Sc" => sub == 'c',
        "Sk" => sub == 'k',
        _ => sub == 'o',
    }
}

pub(crate) fn is_name_start(c: char) -> bool {
    c == ':' || c == '_' || c.is_ascii_alphabetic() || (!c.is_ascii() && (c.is_alphabetic() || matches!(c, '\u{C0}'..='\u{2FF}' | '\u{370}'..='\u{37D}' | '\u{37F}'..='\u{1FFF}' | '\u{200C}'..='\u{200D}' | '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}' | '\u{3001}'..='\u{D7FF}' | '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}' | '\u{10000}'..='\u{EFFFF}')) && c != '\u{D7}' && c != '\u{F7}')
}

pub(crate) fn is_name_char(c: char) -> bool {
    is_name_start(c) || c == '-' || c == '.' || c.is_ascii_digit() || c == '\u{B7}' || matches!(c, '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')
}

impl Class {
    fn matches(&self, c: char) -> bool {
        let mut hit = self.items.iter().any(|item| match item {
            Item::Range(lo, hi) => c >= *lo && c <= *hi,
            Item::Prop(name) => category_matches(name, c),
            Item::Class(class) => class.matches(c),
        });
        if hit && let Some(m) = &self.minus {
            hit = !m.matches(c);
        }
        hit != self.negated
    }
}

impl Regex {
    /// Compiles `pattern`, or None when it is not a valid XSD regex.
    pub(crate) fn new(pattern: &str) -> Option<Regex> {
        let chars: Vec<char> = pattern.chars().collect();
        let mut p = Parser { s: &chars, i: 0 };
        let ast = p.alt().ok()?;
        if p.i != chars.len() {
            return None;
        }
        let mut nodes = vec![Node::Match];
        let start = compile(&ast, 0, &mut nodes);
        Some(Regex { nodes, start })
    }

    pub(crate) fn is_match(&self, text: &str) -> bool {
        let mut current: Vec<usize> = Vec::new();
        let mut seen = vec![false; self.nodes.len()];
        self.add(self.start, &mut current, &mut seen);
        for c in text.chars() {
            let mut next = Vec::new();
            let mut seen = vec![false; self.nodes.len()];
            for &s in &current {
                match &self.nodes[s] {
                    Node::Set(class, to) if class.matches(c) => self.add(*to, &mut next, &mut seen),
                    Node::Any(to) if c != '\n' && c != '\r' => self.add(*to, &mut next, &mut seen),
                    _ => {}
                }
            }
            if next.is_empty() {
                return false;
            }
            current = next;
        }
        current.iter().any(|&s| matches!(self.nodes[s], Node::Match))
    }

    fn add(&self, s: usize, list: &mut Vec<usize>, seen: &mut [bool]) {
        if seen[s] {
            return;
        }
        seen[s] = true;
        match &self.nodes[s] {
            Node::Split(a, b) => {
                self.add(*a, list, seen);
                self.add(*b, list, seen);
            }
            Node::Jump(t) => self.add(*t, list, seen),
            _ => list.push(s),
        }
    }
}

/// Emits `ast` so that it continues at `next`; returns its entry node.
fn compile(ast: &Ast, next: usize, nodes: &mut Vec<Node>) -> usize {
    let push = |n: Node, nodes: &mut Vec<Node>| {
        nodes.push(n);
        nodes.len() - 1
    };
    match ast {
        Ast::Empty => next,
        Ast::Set(c) => push(Node::Set(c.clone(), next), nodes),
        Ast::Any => push(Node::Any(next), nodes),
        Ast::Cat(parts) => {
            let mut at = next;
            for p in parts.iter().rev() {
                at = compile(p, at, nodes);
            }
            at
        }
        Ast::Alt(branches) => {
            let mut entries: Vec<usize> = branches.iter().map(|b| compile(b, next, nodes)).collect();
            let mut at = entries.pop().unwrap_or(next);
            while let Some(e) = entries.pop() {
                at = push(Node::Split(e, at), nodes);
            }
            at
        }
        Ast::Rep(inner, min, max) => {
            let min = (*min).min(UNROLL_CAP);
            let mut at = next;
            match max {
                None => {
                    // inner* then min copies in front.
                    let split = push(Node::Jump(0), nodes);
                    let body = compile(inner, split, nodes);
                    nodes[split] = Node::Split(body, next);
                    at = split;
                }
                Some(max) => {
                    let optional = max.saturating_sub(min).min(UNROLL_CAP);
                    for _ in 0..optional {
                        let body = compile(inner, at, nodes);
                        at = push(Node::Split(body, next), nodes);
                    }
                }
            }
            for _ in 0..min {
                at = compile(inner, at, nodes);
            }
            at
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Regex;

    fn m(p: &str, s: &str) -> bool {
        Regex::new(p).expect("valid").is_match(s)
    }

    #[test]
    fn basics() {
        assert!(m("[a-z]+", "abc"));
        assert!(!m("[a-z]+", "abC"));
        assert!(m("\\d{3}-\\d{4}", "555-1234"));
        assert!(m("a|bc", "bc"));
        assert!(!m("a|bc", "abc"));
        assert!(m("(ab){2,3}", "ababab"));
        assert!(!m("(ab){2,3}", "ab"));
        assert!(m("[a-z-[aeiou]]+", "xyz"));
        assert!(!m("[a-z-[aeiou]]+", "xay"));
        assert!(m("\\p{Lu}\\p{Ll}*", "Hello"));
        assert!(m("\\i\\c*", "foo-bar"));
        assert!(m("[^x]", "y"));
        assert!(Regex::new("[").is_none());
    }
}
