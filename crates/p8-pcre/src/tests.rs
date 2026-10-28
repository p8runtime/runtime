use super::*;

/// The match of `pat` in `subject` as group texts, or None.
fn m(pat: &str, subject: &str) -> Option<Vec<Option<String>>> {
    m_opts(pat, subject, &Options::default())
}

fn m_opts(pat: &str, subject: &str, o: &Options) -> Option<Vec<Option<String>>> {
    let re = Regex::new(pat.as_bytes(), o).unwrap_or_else(|e| panic!("{pat}: {e}"));
    let c = re.exec(subject.as_bytes(), 0, &MatchOptions::default()).unwrap()?;
    Some((0..c.len()).map(|i| c.get(i).map(|(a, b)| subject[a..b].to_string())).collect())
}

fn whole(pat: &str, subject: &str) -> Option<String> {
    m(pat, subject).map(|v| v[0].clone().unwrap())
}

fn utf() -> Options {
    Options { utf: true, ucp: true, ..Options::default() }
}

#[test]
fn literals_and_classes() {
    assert_eq!(whole("abc", "xxabcxx").as_deref(), Some("abc"));
    assert_eq!(whole("a[bc]+d", "abcbcd").as_deref(), Some("abcbcd"));
    assert_eq!(whole("[^a-c]+", "abcxyzabc").as_deref(), Some("xyz"));
    assert_eq!(whole(r"\d+", "ab 123 c").as_deref(), Some("123"));
    assert_eq!(whole(r"[[:alpha:]]+", "12ab3").as_deref(), Some("ab"));
    assert_eq!(whole(r"[\w.-]+@\w+", "mail: a.b-c@host!").as_deref(), Some("a.b-c@host"));
    assert_eq!(whole("x", "abc"), None);
}

#[test]
fn quantifiers() {
    assert_eq!(whole("a.*c", "abcbc").as_deref(), Some("abcbc"));
    assert_eq!(whole("a.*?c", "abcbc").as_deref(), Some("abc"));
    assert_eq!(whole("a{2,3}", "aaaa").as_deref(), Some("aaa"));
    assert_eq!(whole("(ab){2}", "ababab").as_deref(), Some("abab"));
    assert_eq!(whole("a++b", "aaab").as_deref(), Some("aaab"));
    assert_eq!(whole("a++a", "aaaa"), None);
    assert_eq!(whole("(?:a|b)*c", "ababc").as_deref(), Some("ababc"));
    assert_eq!(whole("(a*)*b", "aab").as_deref(), Some("aab"));
    assert_eq!(whole("x{", "x{").as_deref(), Some("x{"));
}

#[test]
fn groups_and_backrefs() {
    let g = m(r"(\w+) (\w+)", "hello world").unwrap();
    assert_eq!(g[1].as_deref(), Some("hello"));
    assert_eq!(g[2].as_deref(), Some("world"));
    assert_eq!(whole(r"(a|b)\1", "abba").as_deref(), Some("bb"));
    assert_eq!(whole(r"(?<x>o)\k<x>", "foo").as_deref(), Some("oo"));
    let g = m("(a)|(b)", "b").unwrap();
    assert_eq!(g, vec![Some("b".into()), None, Some("b".into())]);
    assert_eq!(whole("(?i)ABC", "xabcx").as_deref(), Some("abc"));
    assert_eq!(whole("a(?i)b|c", "C"), Some("C".into()));
    assert_eq!(whole("(?|(a)|(b))", "b").as_deref(), Some("b"));
}

#[test]
fn anchors_and_lookaround() {
    assert_eq!(whole("^b", "ab"), None);
    let multi = Options { multiline: true, ..Options::default() };
    assert_eq!(m_opts("^b", "a\nb", &multi).map(|v| v[0].clone().unwrap()).as_deref(), Some("b"));
    assert_eq!(whole("a$", "a\n").as_deref(), Some("a"));
    assert_eq!(whole(r"a\z", "a\n"), None);
    assert_eq!(whole(r"\bfoo\b", "a foo b").as_deref(), Some("foo"));
    assert_eq!(whole("foo(?=bar)", "foobaz foobar").as_deref(), Some("foo"));
    assert_eq!(whole("foo(?!bar)", "foobar foobaz").as_deref(), Some("foo"));
    assert_eq!(whole("(?<=\\$)\\d+", "cost $42").as_deref(), Some("42"));
    assert_eq!(whole("(?<!\\$)\\b\\d+", "$42 17").as_deref(), Some("17"));
    assert_eq!(whole("(?<=a|bc)x", "bcx").as_deref(), Some("x"));
    assert_eq!(whole(r"foo\Kbar", "foobar").as_deref(), Some("bar"));
}

#[test]
fn recursion_and_conditionals() {
    assert_eq!(whole(r"\((?:[^()]|(?R))*\)", "x(a(b)c)y").as_deref(), Some("(a(b)c)"));
    assert_eq!(whole(r"^(a)?(?(1)b|c)$", "ab").as_deref(), Some("ab"));
    assert_eq!(whole(r"^(a)?(?(1)b|c)$", "c").as_deref(), Some("c"));
    assert_eq!(whole(r"(?(DEFINE)(?<d>\d+))x(?&d)", "x12").as_deref(), Some("x12"));
    assert_eq!(whole(r"^(?:(a)|b)(?(1)A|B)", "bB").as_deref(), Some("bB"));
}

#[test]
fn verbs() {
    assert_eq!(whole("a+(*COMMIT)b", "aaac aab"), None);
    assert_eq!(whole("a(*FAIL)|b", "ab").as_deref(), Some("b"));
    assert_eq!(whole("aaa(*SKIP)b|a+c", "aaac").as_deref(), None);
    assert_eq!(whole("a(*ACCEPT)b", "ac").as_deref(), Some("a"));
}

#[test]
fn unicode() {
    assert_eq!(m_opts(r"\p{Lu}+", "abcÉCOLEdef", &utf()).unwrap()[0].as_deref(), Some("ÉCOLE"));
    assert_eq!(m_opts(r"(?i)école", "ÉCOLE", &utf()).unwrap()[0].as_deref(), Some("ÉCOLE"));
    assert_eq!(m_opts(r"\w+", "日本語!", &utf()).unwrap()[0].as_deref(), Some("日本語"));
    assert_eq!(m_opts(r"\p{Han}", "a日", &utf()).unwrap()[0].as_deref(), Some("日"));
    assert_eq!(m_opts(r".", "é", &utf()).unwrap()[0].as_deref(), Some("é"));
    assert_eq!(m_opts(r"\X", "e\u{301}x", &utf()).unwrap()[0].as_deref(), Some("e\u{301}"));
    assert_eq!(m_opts(r"(?i)k", "\u{212a}", &utf()).unwrap()[0].as_deref(), Some("\u{212a}"));
}

#[test]
fn errors() {
    let e = |p: &str| Regex::new(p.as_bytes(), &Options::default()).err().map(|e| e.to_string());
    assert_eq!(e("(abc").as_deref(), Some("missing closing parenthesis at offset 4"));
    assert_eq!(e("abc)").as_deref(), Some("unmatched closing parenthesis at offset 3"));
    assert_eq!(e("[a").as_deref(), Some("missing terminating ] for character class at offset 2"));
    assert_eq!(e("*a").as_deref(), Some("quantifier does not follow a repeatable item at offset 0"));
    assert_eq!(e(r"\1").as_deref(), Some("reference to non-existent subpattern at offset 1"));
    assert_eq!(e("(?<=a+)b").as_deref(), Some("length of lookbehind assertion is not limited at offset 0"));
}

#[test]
fn limits() {
    let re = Regex::new(b".*\\p{N}", &Options::default()).unwrap();
    let o = MatchOptions { match_limit: 2, ..MatchOptions::default() };
    assert_eq!(re.exec(b"0123456789", 0, &o).err(), Some(MatchError::MatchLimit));
    let re = Regex::new(b"(a+)+$", &Options::default()).unwrap();
    let o = MatchOptions { match_limit: 100_000, ..MatchOptions::default() };
    assert_eq!(re.exec(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaab", 0, &o).err(), Some(MatchError::MatchLimit));
}
