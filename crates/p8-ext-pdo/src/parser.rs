//! Finding the placeholders of a query: `pdo_sql_parser.re`,
//! `sqlite_sql_parser.re`, `pgsql_sql_parser.re` and `mysql_sql_parser.re`. String literals and
//! comments are skipped; `:name` and `?` are placeholders (`::` and `??` are
//! text, except that pgsql's `??` is an escaped `?`).

use crate::driver::Scanner;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    Text,
    Bind,
    BindPos,
    /// pgsql's `??`: a `?` that is not a placeholder.
    EscapedQuestion,
    /// pgsql's `$$` or `$tag$`.
    CustomQuote,
}

/// One placeholder in the query text.
#[derive(Debug, Clone)]
pub struct Placeholder {
    pub pos: usize,
    pub len: usize,
    /// Its number among the placeholders, from 0.
    pub bindno: usize,
    /// An escaped `?` (pgsql's `??`): the query gets a `?` there.
    pub escaped: bool,
}

/// What the scan found.
#[derive(Debug, Clone, Default)]
pub struct Scan {
    pub placeholders: Vec<Placeholder>,
    pub named: bool,
    pub positional: bool,
    /// `??` met inside a dollar quoted string (PHP deprecates that).
    pub escapes_in_quote: bool,
}

impl Scan {
    /// The placeholders values are bound to (not the escaped `??`).
    pub fn binds(&self) -> impl Iterator<Item = &Placeholder> {
        self.placeholders.iter().filter(|p| !p.escaped)
    }
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

/// The pgsql scanner's `E'...'`: a backslash escapes any byte.
fn e_quoted(s: &[u8]) -> Option<usize> {
    let mut i = 2;
    while i < s.len() {
        match s[i] {
            b'\\' if i + 1 < s.len() => i += 2,
            b'\'' if s.get(i + 1) == Some(&b'\'') => i += 2,
            b'\'' => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

fn dolq_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b >= 0x80
}

fn pgsql_next(sql: &[u8], i: usize) -> (Tok, usize) {
    let s = &sql[i..];
    let c = s[0];
    match c {
        b'e' | b'E' if s.get(1) == Some(&b'\'') => match e_quoted(s) {
            Some(n) => (Tok::Text, n),
            None => (Tok::Text, 1),
        },
        b'"' | b'\'' => match quoted(s, c) {
            Some(n) => (Tok::Text, n),
            None => (Tok::Text, 1),
        },
        b'$' => {
            let w = if s.get(1).is_some_and(|&b| dolq_start(b)) { 1 + s[2..].iter().take_while(|&&b| dolq_start(b) || b.is_ascii_digit()).count() } else { 0 };
            if s.get(1 + w) == Some(&b'$') {
                return (Tok::CustomQuote, w + 2);
            }
            (Tok::Text, 1)
        }
        b':' => {
            let n = s.iter().take_while(|&&b| b == b':').count();
            if n >= 2 {
                return (Tok::Text, n);
            }
            let w = s[1..].iter().take_while(|&&b| alnum(b) || b == b'_').count();
            if w > 0 { (Tok::Bind, 1 + w) } else { (Tok::Text, 1) }
        }
        b'?' => {
            if s.get(1) == Some(&b'?') {
                return (Tok::EscapedQuestion, 2);
            }
            (Tok::BindPos, 1)
        }
        b'/' | b'-' => next(sql, i, Scanner::Default),
        b'e' | b'E' => (Tok::Text, 1),
        _ => {
            let n = s.iter().take_while(|b| !b"$eE:?\"'/-".contains(b)).count();
            (Tok::Text, n.max(1))
        }
    }
}

/// mysql_sql_parser.re: a backslash escapes in '' and "" strings, ``
/// quotes identifiers, `#` and `-- ` (a dash pair and a space) start
/// comments.
fn mysql_next(sql: &[u8], i: usize) -> (Tok, usize) {
    let s = &sql[i..];
    let c = s[0];
    match c {
        b'"' | b'\'' => {
            let mut j = 1;
            while j < s.len() {
                match s[j] {
                    b'\\' if j + 1 < s.len() => j += 2,
                    q if q == c && s.get(j + 1) == Some(&c) => j += 2,
                    q if q == c => return (Tok::Text, j + 1),
                    _ => j += 1,
                }
            }
            (Tok::Text, 1)
        }
        b'`' => match quoted(s, b'`') {
            Some(n) => (Tok::Text, n),
            None => (Tok::Text, 1),
        },
        b'#' => (Tok::Text, s.iter().position(|&b| b == b'\n').unwrap_or(s.len())),
        b'-' if s.get(1) == Some(&b'-') && s.get(2).is_some_and(|b| b" \t\x0b\x0c\r".contains(b)) => (Tok::Text, s.iter().position(|&b| b == b'\n').unwrap_or(s.len())),
        b'-' => (Tok::Text, 1),
        b':' | b'?' | b'/' => next(sql, i, Scanner::Default),
        _ => {
            let n = s.iter().take_while(|b| !b":?\"'`/#-".contains(b)).count();
            (Tok::Text, n.max(1))
        }
    }
}

fn next(sql: &[u8], i: usize, scanner: Scanner) -> (Tok, usize) {
    if scanner == Scanner::Pgsql {
        return pgsql_next(sql, i);
    }
    if scanner == Scanner::Mysql {
        return mysql_next(sql, i);
    }
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
    // Inside a dollar quoted string: where its opening quote is.
    let mut custom: Option<(usize, usize)> = None;
    while i < sql.len() {
        let (tok, n) = next(sql, i, scanner);
        if let Some((pos, len)) = custom {
            if tok == Tok::CustomQuote && n == len && sql[i..i + n] == sql[pos..pos + len] {
                custom = None;
            } else if tok == Tok::EscapedQuestion {
                out.escapes_in_quote = true;
                out.placeholders.push(Placeholder { pos: i, len: n, bindno: usize::MAX, escaped: true });
            }
            i += n;
            continue;
        }
        match tok {
            Tok::CustomQuote => custom = Some((i, n)),
            Tok::EscapedQuestion => out.placeholders.push(Placeholder { pos: i, len: n, bindno: usize::MAX, escaped: true }),
            Tok::Bind => {
                // `x:y` is not a placeholder: an alphanumeric precedes the colon.
                if i > 0 && alnum(sql[i - 1]) {
                    i += n;
                    continue;
                }
                out.named = true;
                out.placeholders.push(Placeholder { pos: i, len: n, bindno, escaped: false });
                bindno += 1;
            }
            Tok::BindPos => {
                out.positional = true;
                out.placeholders.push(Placeholder { pos: i, len: n, bindno, escaped: false });
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

/// How `?` and `:name` were turned into `$1`, `$2` (pdo_parse_params()
/// with `named_rewrite_template`): each name keeps its first number.
#[derive(Debug, Clone, Default)]
pub struct DollarMap {
    /// `:name` to its 0-based position.
    pub names: Vec<(String, usize)>,
    /// How many placeholders the query had (bindno 0..count is defined).
    pub count: usize,
}

/// Rewrite the placeholders of a scan into `$N`. None for the map when the
/// query had nothing to rewrite (no bound_param_map in PHP).
pub fn dollar_rewrite(sql: &[u8], scan: &Scan) -> (Vec<u8>, Option<DollarMap>) {
    if scan.placeholders.is_empty() {
        return (sql.to_vec(), None);
    }
    let mut map = DollarMap::default();
    let mut next = 1usize;
    let out = rewrite(sql, &scan.placeholders, |p| {
        if p.escaped {
            return b"?".to_vec();
        }
        map.count = map.count.max(p.bindno + 1);
        let name = String::from_utf8_lossy(&sql[p.pos..p.pos + p.len]).into_owned();
        if name != "?"
            && let Some((_, n)) = map.names.iter().find(|(x, _)| *x == name)
        {
            return format!("${}", n + 1).into_bytes();
        }
        let n = next;
        next += 1;
        if name != "?" {
            map.names.push((name, n - 1));
        }
        format!("${n}").into_bytes()
    });
    (out, Some(map))
}

/// Rewrite the `:name` placeholders of a scan into `?` (pdo_parse_params()
/// for PDO_PLACEHOLDER_POSITIONAL): the names, by position, when there
/// were any (the bound_param_map); a query of `?` is left as it is.
pub fn qmark_rewrite(sql: &[u8], scan: &Scan) -> (Vec<u8>, Option<Vec<String>>) {
    if !scan.named {
        return (sql.to_vec(), None);
    }
    let names = scan.binds().map(|p| String::from_utf8_lossy(&sql[p.pos..p.pos + p.len]).into_owned()).collect();
    (rewrite(sql, &scan.placeholders, |_| b"?".to_vec()), Some(names))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_strings_comments_and_names() {
        let sql = b"SELECT 'it\\'s ?', \"\\\"?\", `?x`, ? # :a\n, :b -- ?\n, 1--:c";
        let s = scan(sql, Scanner::Mysql);
        let found: Vec<_> = s.placeholders.iter().map(|p| &sql[p.pos..p.pos + p.len]).collect();
        assert_eq!(found, vec![&b"?"[..], b":b", b":c"]);
        let (out, names) = qmark_rewrite(b"SELECT :a, ':b', :a", &scan(b"SELECT :a, ':b', :a", Scanner::Mysql));
        assert_eq!(out, b"SELECT ?, ':b', ?".to_vec());
        assert_eq!(names, Some(vec![":a".to_string(), ":a".to_string()]));
    }

    #[test]
    fn skips_strings_and_comments() {
        let s = scan(b"SELECT ':a', \"?\" , ? -- :b\n, :c /* ? */", Scanner::Default);
        let names: Vec<_> = s.placeholders.iter().map(|p| p.len).collect();
        assert_eq!(names, vec![1, 2]);
        assert!(s.named && s.positional);
    }

    #[test]
    fn pgsql_quotes_and_escapes() {
        let s = scan(b"SELECT E'\\'?', $$ ? $$, $a$ ?? $a$, ??, ?, :x::int", Scanner::Pgsql);
        let kinds: Vec<_> = s.placeholders.iter().map(|p| (p.len, p.escaped)).collect();
        assert_eq!(kinds, vec![(2, true), (2, true), (1, false), (2, false)]);
        assert!(s.escapes_in_quote);
        assert_eq!(s.binds().count(), 2);
    }

    #[test]
    fn dollar_rewrite_reuses_names() {
        let sql = b"SELECT :a, :b, :a, ?? x";
        let s = scan(sql, Scanner::Pgsql);
        let (out, map) = dollar_rewrite(sql, &s);
        assert_eq!(out, b"SELECT $1, $2, $1, ? x".to_vec());
        let map = map.expect("map");
        assert_eq!(map.names, vec![(":a".to_string(), 0), (":b".to_string(), 1)]);
        assert_eq!(map.count, 3);
    }

    #[test]
    fn double_colon_is_text() {
        let s = scan(b"SELECT a::int, ??, :x", Scanner::Default);
        assert_eq!(s.placeholders.len(), 1);
    }
}
