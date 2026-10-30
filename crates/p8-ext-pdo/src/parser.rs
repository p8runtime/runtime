//! Finding the placeholders of a query: `pdo_sql_parser.re` and
//! `sqlite_sql_parser.re`. String literals and comments are skipped; `:name`
//! and `?` are placeholders (`::` and `??` are text).

use crate::driver::Scanner;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    Text,
    Bind,
    BindPos,
}

/// One placeholder in the query text.
#[derive(Debug, Clone)]
pub struct Placeholder {
    pub pos: usize,
    pub len: usize,
    /// Its number among the placeholders, from 0.
    pub bindno: usize,
}

/// What the scan found.
#[derive(Debug, Clone, Default)]
pub struct Scan {
    pub placeholders: Vec<Placeholder>,
    pub named: bool,
    pub positional: bool,
}

fn alnum(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}

/// The end of a quoted run starting at `s[0]` (the quote `q`) when `greedy`
/// is false: doubled quotes escape, the first lone quote ends it.
fn quoted(s: &[u8], q: u8) -> Option<usize> {
    let mut i = 1;
    while i < s.len() {
        if s[i] == q {
            if s.get(i + 1) == Some(&q) {
                i += 2;
                continue;
            }
            return Some(i + 1);
        }
        i += 1;
    }
    None
}

/// SQLite's scanner lets a quoted run contain its own quote character, so
/// the longest match runs to the last quote of the query.
fn quoted_greedy(s: &[u8], q: u8) -> Option<usize> {
    s.iter().rposition(|&b| b == q).filter(|&i| i > 0).map(|i| i + 1)
}

fn next(sql: &[u8], i: usize, scanner: Scanner) -> (Tok, usize) {
    let s = &sql[i..];
    let c = s[0];
    match c {
        b'"' | b'\'' => {
            let n = if scanner == Scanner::Sqlite { quoted_greedy(s, c) } else { quoted(s, c) };
            if let Some(n) = n {
                return (Tok::Text, n);
            }
            (Tok::Text, 1)
        }
        b'`' if scanner == Scanner::Sqlite => match quoted_greedy(s, b'`') {
            Some(n) => (Tok::Text, n),
            None => (Tok::Text, 1),
        },
        b'[' if scanner == Scanner::Sqlite => match s.iter().rposition(|&b| b == b']') {
            Some(n) if n > 0 => (Tok::Text, n + 1),
            _ => (Tok::Text, 1),
        },
        b':' | b'?' => {
            let n = s.iter().take_while(|&&b| b == c).count();
            if n >= 2 {
                return (Tok::Text, n);
            }
            if c == b'?' {
                return (Tok::BindPos, 1);
            }
            let w = s[1..].iter().take_while(|&&b| alnum(b) || b == b'_').count();
            if w > 0 { (Tok::Bind, 1 + w) } else { (Tok::Text, 1) }
        }
        b'/' => {
            if s.get(1) == Some(&b'*')
                && let Some(end) = find(&s[2..], b"*/")
            {
                return (Tok::Text, 2 + end + 2);
            }
            (Tok::Text, 1)
        }
        b'-' => {
            if s.get(1) == Some(&b'-') {
                let n = s.iter().position(|&b| b == b'\n').unwrap_or(s.len());
                return (Tok::Text, n);
            }
            (Tok::Text, 1)
        }
        _ => {
            let specials: &[u8] = if scanner == Scanner::Sqlite { b":?\"'`/[-" } else { b":?\"'/-" };
            let n = s.iter().take_while(|b| !specials.contains(b)).count();
            (Tok::Text, n.max(1))
        }
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Phase 1 of `pdo_parse_params()`: look for the placeholders.
pub fn scan(sql: &[u8], scanner: Scanner) -> Scan {
    let mut out = Scan::default();
    let mut i = 0;
    let mut bindno = 0;
    while i < sql.len() {
        let (tok, n) = next(sql, i, scanner);
        match tok {
            Tok::Bind => {
                // `x:y` is not a placeholder: an alphanumeric precedes the colon.
                if i > 0 && alnum(sql[i - 1]) {
                    i += n;
                    continue;
                }
                out.named = true;
                out.placeholders.push(Placeholder { pos: i, len: n, bindno });
                bindno += 1;
            }
            Tok::BindPos => {
                out.positional = true;
                out.placeholders.push(Placeholder { pos: i, len: n, bindno });
                bindno += 1;
            }
            Tok::Text => {}
        }
        i += n;
    }
    out
}

/// The query with every placeholder replaced by `with(placeholder)`.
pub fn rewrite(sql: &[u8], placeholders: &[Placeholder], mut with: impl FnMut(&Placeholder) -> Vec<u8>) -> Vec<u8> {
    let mut out = Vec::with_capacity(sql.len());
    let mut at = 0;
    for p in placeholders {
        out.extend_from_slice(&sql[at..p.pos]);
        out.extend_from_slice(&with(p));
        at = p.pos + p.len;
    }
    out.extend_from_slice(&sql[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_strings_and_comments() {
        let s = scan(b"SELECT ':a', \"?\" , ? -- :b\n, :c /* ? */", Scanner::Default);
        let names: Vec<_> = s.placeholders.iter().map(|p| p.len).collect();
        assert_eq!(names, vec![1, 2]);
        assert!(s.named && s.positional);
    }

    #[test]
    fn double_colon_is_text() {
        let s = scan(b"SELECT a::int, ??, :x", Scanner::Default);
        assert_eq!(s.placeholders.len(), 1);
    }
}
