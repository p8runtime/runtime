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

/// The required-literal prefilter rejects only subjects that cannot match
/// (both engines run behind it, so their agreement does not test it).
/// Runs of single-byte characters match as one comparison (Inst::Lit),
/// with jumps into the middle of a run and the subject ending inside one.
#[test]
fn literal_runs() {
    let lits = |pat: &str, o: &Options| Regex::new(pat.as_bytes(), o).unwrap().prog.insts.iter().filter(|i| matches!(i, compile::Inst::Lit { .. })).count();
    assert_eq!(lits("^/users/(\\d+)/posts/(\\d+)", &Options::default()), 2);
    assert_eq!(lits("abc", &Options { caseless: true, ..Options::default() }), 0, "caseless characters are not bytes");
    assert_eq!(lits("a€b", &utf()), 0, "a non-ASCII character in UTF mode is not one byte");
    assert_eq!(lits("ab€cd", &utf()), 2);
    assert_eq!(whole("^/users/(\\d+)/posts", "/users/42/posts/1").as_deref(), Some("/users/42/posts"));
    assert_eq!(whole("abcd", "xabcabcdx").as_deref(), Some("abcd"));
    assert_eq!(whole("abcd", "xabc"), None);
    assert_eq!(whole("(?:ab|b)cd", "xbcd").as_deref(), Some("bcd"));
    assert_eq!(whole("x(?:abc)+y", "xabcabcy").as_deref(), Some("xabcabcy"));
    assert_eq!(whole("x(?:abc)+y", "xabcaby"), None);
    assert_eq!(m_opts("ab€cd", "xab€cd", &utf()).unwrap()[0].as_deref(), Some("ab€cd"));
    let latin1 = "\u{e9}t\u{e9}";
    assert_eq!(lits(latin1, &Options::default()), 1, "outside UTF mode every character is a byte");
}

/// A class's table of the first 256 characters, made item by item, says
/// what testing each character against its items says.
#[test]
fn class_tables() {
    let classes = [
        r"[a-z]", r"[A-Z0-9_]", r"[^a-z]", r"[\w-]", r"[^\s<>]", r"[\d.,]", r"[[:alpha:]]", r"[[:^punct:]x]", r"[\p{L}\p{N}]",
        r"[\p{Lu}]", r"[^\p{Ll}]", r"[k-z]", r"[\x{e0}-\x{ff}]", r"[\x{100}-\x{17f}]", r"[\h\v]", r"[^\H]", r"[µſ]", r"[\x{3bc}]", r"[a-c\W]",
    ];
    for class in classes {
        for (flags, o) in [
            ("", Options::default()),
            ("i", Options { caseless: true, ..Options::default() }),
            ("u", utf()),
            ("iu", Options { caseless: true, ..utf() }),
            ("iur", Options { caseless: true, caseless_restrict: true, ..utf() }),
        ] {
            let Ok(re) = Regex::new(class.as_bytes(), &o) else { continue };
            for set in &re.prog.sets {
                for c in 0..256u32 {
                    let want = set.test_items(c, re.prog.utf) != set.set.negated;
                    assert_eq!(set.matches(c, re.prog.utf), want, "{class} /{flags}: {c:#x}");
                }
            }
        }
    }
}

/// A short haystack is searched by its needle's first byte.
#[test]
fn short_haystacks() {
    let f = memchr::memmem::Finder::new(b"{{").into_owned();
    for (hay, want) in [("", None), ("{", None), ("{{", Some(0)), ("a{b{{", Some(3)), ("{a{", None), ("xx{{", Some(2))] {
        assert_eq!(compile::find_text(&f, hay.as_bytes()), want, "{hay:?}");
    }
    let long = format!("{}{{{{", "{".repeat(70));
    assert_eq!(compile::find_text(&f, long.as_bytes()), Some(0));
    let long = format!("{}{{{{", "x".repeat(70));
    assert_eq!(compile::find_text(&f, long.as_bytes()), Some(70));
}

#[test]
fn required_literal() {
    let req = |pat: &str, o: &Options| Regex::new(pat.as_bytes(), o).unwrap().prog.required.is_some();
    assert!(req(r"\s*wp:core", &Options::default()));
    assert!(req(r"(?i)abc(?-i)D", &Options::default()));
    assert!(!req(r"ab{2,}c|q", &Options::default()));
    assert!(!req(r"a(*ACCEPT)bcd", &Options::default()));
    assert!(!req(r"(?i)k", &utf()));
    assert!(!req("abc", &Options::default()), "the prefix is the first-bytes search's");
    assert_eq!(whole(r"\s*wp:core", "<!-- wp:core/image -->").as_deref(), Some(" wp:core"));
    assert_eq!(whole(r"\s*wp:core", "<!-- wp:paragraph -->"), None);
    assert_eq!(whole(r"(?i)abc(?-i)D", "xABCd"), None);
    assert_eq!(whole(r"(?i)abc(?-i)D", "xabcD").as_deref(), Some("abcD"));
    assert_eq!(whole(r"(?i)abc(?-i)D", "xaBCD").as_deref(), Some("aBCD"));
    assert_eq!(whole(r"a(*ACCEPT)bcd", "ax").as_deref(), Some("a"));
    assert_eq!(whole(r"foo\Kbar", "xfoobar").as_deref(), Some("bar"));
    assert_eq!(whole(r"(?<=foo)bar", "foobar").as_deref(), Some("bar"));
    assert_eq!(whole(r"\d+px|\d+em", "3em").as_deref(), Some("3em"));
    assert_eq!(whole(r"x+yz", "xxyz").as_deref(), Some("xxyz"));
    assert_eq!(whole(r"(ab){2}c", "ababc").as_deref(), Some("ababc"));
    assert_eq!(whole(r"(ab){2}c", "abc"), None);
    let mi = Options { multiline: true, caseless: true, ..Options::default() };
    let header = "<?php\n/**\n * Plugin Name: x\n * post types: a, b\n */";
    assert_eq!(m_opts(r"^[ \t\/*#@]*Post Types:(.*)$", header, &mi).unwrap()[1].as_deref(), Some(" a, b"));
    assert_eq!(m_opts(r"^[ \t\/*#@]*Block Types:(.*)$", header, &mi), None);
    assert_eq!(m_opts(r"(?i)k", "\u{212a}", &utf()).unwrap()[0].as_deref(), Some("\u{212a}"));
}

/// The DFA (dfa.rs) and the machine agree on every match's bounds, over
/// the whole subject and for every start offset, with and without
/// NOTEMPTY_ATSTART. A lowered match limit keeps a search on the machine.
#[test]
fn dfa_agrees_with_machine() {
    let o = |s: &str| Options {
        caseless: s.contains('i'),
        multiline: s.contains('m'),
        dotall: s.contains('s'),
        extended: s.contains('x'),
        ungreedy: s.contains('U'),
        anchored: s.contains('A'),
        dollar_endonly: s.contains('D'),
        utf: s.contains('u'),
        ucp: s.contains('u'),
        ..Options::default()
    };
    let cases: &[(&str, &str, &[&str])] = &[
        ("abc", "", &["xxabcxx", "ab", "", "abcabc"]),
        // Short subjects with a first byte memchr finds: each candidate
        // start, anchored, failing late before one matches.
        ("q[^q]*q", "", &["xqabq", "qqq", "qab", "aqbqcq"]),
        (r"\{\{\s*(\w+)\s*\}\}", "", &["Hello {{ name }}, {{x}}", "{{ a }", "{{{{b}}", "{ {{c}}"]),
        ("(?:xy|z)w", "", &["xyzw xyw", "zzw", "xyz"]),
        ("[ab]c+", "", &["aabbcc", "cab", "bc"]),
        ("a|ab|abc", "", &["abc", "xabcx"]),
        ("(a|ab)(c|bcd)(d*)", "", &["abcd", "abcdd"]),
        ("x*", "", &["axxb", "", "xxx"]),
        ("(?:)", "", &["abc"]),
        ("a.*c", "", &["abcbc", "ab\ncbc", "a"]),
        ("a.*?c", "", &["abcbc"]),
        ("a.*c", "s", &["ab\ncbc"]),
        ("a{2,3}", "", &["aaaa", "a", "aa"]),
        ("(ab){2}", "", &["ababab", "ab"]),
        ("(?:a|b)*c", "", &["ababc", "abab"]),
        ("(a*)*b", "", &["aab", "b", "aaa"]),
        ("(a*)+b", "", &["aab"]),
        ("(a|b)*?b", "", &["aabab"]),
        (r"\d+", "", &["ab 123 c 45", "no digits", "9"]),
        (r"\d+", "U", &["ab 123 c"]),
        (r"\d{2,}?", "", &["12345"]),
        (r"a{,3}", "", &["aaaa", "a{,3}"]),
        (r"[[:alpha:]]+", "", &["12ab3", "naïve café"]),
        (r"[[:alpha:]]+", "u", &["naïve café", "日本語!"]),
        (r"[\w.-]+@\w+", "", &["mail: a.b-c@host!", "@"]),
        (r"\w+", "u", &["naïve café", "日本語!"]),
        (r"\w+", "", &["naïve"]),
        (r"\bfoo\b", "", &["a foo b", "afoo foob foo", "foo"]),
        (r"\Bo\B", "", &["foo boo o", "ooo"]),
        (r"\b\w+\b", "", &["Hello, world! foo_bar 42"]),
        (r"\bé", "u", &["café é", "é"]),
        (r"\b\w", "u", &["日本 語", "a日"]),
        ("^b", "", &["ab", "b", "a\nb"]),
        ("^b", "m", &["a\nb", "ab\nb", "a\n\nb"]),
        ("^", "m", &["a\nb\nc", "", "\n"]),
        ("$", "m", &["a\nb\n", "", "\n\n"]),
        ("a$", "", &["a\n", "a", "a\nb", "ba\n\n", "a\n\n"]),
        ("a$", "D", &["a\n", "a"]),
        ("a$", "m", &["a\nb", "ba\na", "a\n"]),
        (r"a\Z", "", &["a\n", "a", "a\n\n"]),
        (r"a\z", "", &["a\n", "a"]),
        (r"\Aa", "", &["aa", "ba"]),
        (r"^\s*$", "m", &["a\n\n  \nb", "  "]),
        (r"^(?:(\d+)|(\w+))$", "", &["abc", "123", "a1\n"]),
        (r"^$", "", &["", "\n", "a"]),
        (r"^$", "m", &["", "\n", "a\n\nb", "a\n"]),
        ("(?i)straße", "u", &["STRASSE straße STRAßE"]),
        ("(?i)ABC", "", &["xabcx", "ABC", "aBc"]),
        ("(?i)[a-c]+", "", &["ABCabcD"]),
        ("(?i)[^a-c]+", "", &["ABCxyz"]),
        ("(?i)k", "u", &["\u{212a} K k"]),
        ("(?i)école", "u", &["ÉCOLE", "école"]),
        (r"\p{Lu}\p{Ll}+", "u", &["Été Ölm Ab", "abc"]),
        (r"\p{N}+", "u", &["x٣٤5", "abc"]),
        (r"[^\p{L}]+", "u", &["ab12ü!", "abc"]),
        (r"[\p{Han}]+", "u", &["漢字かな", "abc"]),
        (r".", "u", &["héllo wörld", "\n", ""]),
        (r".+", "", &["a\nb", "\n\n"]),
        (r"(?s).+", "", &["a\nb"]),
        (r"[^\x00-\x7F]+", "", &["abc\u{e9}d"]),
        (r"[\d\s]+", "", &["1 2 3x", "x"]),
        (r"\S+", "", &[" ab  cd ", "   "]),
        (r"\h+", "", &["a \t b", "\u{a0}"]),
        (r"\h+", "u", &["a \t\u{a0}b"]),
        (r"\v", "", &["a\nb", "a\u{85}b"]),
        (r"(?x) a  b # comment\n c", "", &["abc", "a b c"]),
        (r"\Qa.b\E+", "", &["a.bbb", "axb"]),
        (r"[a-z-]+", "", &["foo-bar baz"]),
        (r"(\d+)(?:px|em)?", "", &["10px 5 3em"]),
        (r"\d+(?:px|em)", "", &["10px 5 3em", "7p"]),
        (r"(a+)+$", "", &["aaaa", "aaab"]),
        (r"(a+)+b", "", &["aaaaaaaaaaaaaaaaab", "aaaaaaaaaaaaaaaaac"]),
        (r"(?:a|b)+|c", "", &["ababc", "c", "x"]),
        (r"(a)(?:x|(b))*", "", &["axbxb"]),
        (r"(?:(a)|b)*", "", &["ab", "ba", ""]),
        (r"a*?", "", &["aaa"]),
        (r"a*?b", "", &["aaab"]),
        (r"(?U)a+", "", &["aaa"]),
        (r"a+?", "U", &["aaa"]),
        (r"<!--\s*/?wp:([a-z][a-z0-9_-]*/)?([a-z][a-z0-9_-]*)\s+(/)?-->", "s", &["<!-- wp:paragraph -->text<!-- /wp:paragraph -->", "<!-- wp:core/image /-->", "no blocks"]),
        (r"^[a-z0-9_-]+$", "", &["my-slug", "Bad Slug", ""]),
        (r"[^a-z0-9 _-]", "i", &["Hello World!", "ok"]),
        (r"%[a-zA-Z0-9]{2}", "", &["a%20b%zz", "plain"]),
        (r"&(#\d+|#x[a-f0-9]+|[a-z]+);", "i", &["a &amp; b &#39; &#x27; &bad"]),
        (r"<[^>]*>", "", &["<p>hi</p>", "no tags"]),
        (r"[\r\n\t ]+", "", &["a \n\t b", "abc"]),
        (r"\s+", "", &["  a  b  ", "abc"]),
        (r"[^\s]+", "", &["  a  b  "]),
        (r"^(\d+)\.(\d+)", "", &["8.6.0", "x"]),
        (r"^([a-z]+):", "i", &["Content-Type: x", "nocolon"]),
        (r"[A-Z]", "", &["abcDef", "abc"]),
        (r"\A\s*\z", "", &["   ", "a", ""]),
        (r"^\s+|\s+$", "", &["  ab  ", "ab"]),
        (r"\s+$", "", &["ab  \n", "ab"]),
        (r"[\x{80}-\x{10ffff}]", "u", &["abcé", "abc"]),
        (r"\x{e9}", "u", &["café"]),
        (r"\G\d", "", &["123a45", "a1"]),
        (r"(?:ab|a)(?:bc|c)?", "", &["abc", "abbc"]),
        (r"(?:a|ab)(?:c|bcd)", "", &["abcd"]),
        (r"a?a?a?aaa", "", &["aaa", "aa"]),
        (r"(?:x|y|z)*end", "", &["xyzend", "xyzen"]),
        (r"[a-c]{1,3}d|[a-d]+", "", &["abcd", "aabbd"]),
        // Lookarounds made of assertions.
        (r"(?!^)([A-Z])", "", &["fooBarBaz", "Foo", "ab"]),
        (r"(?<!^)x", "", &["xax", "x"]),
        (r"(?=$)", "", &["ab", ""]),
        (r"a(?=\b)", "", &["a b", "ab"]),
        (r"(?!\B)x", "", &["ax x"]),
        // Required literals, exact and caseless.
        (r"^(?:[ \t]*<\?(?:php)?)?[ \t\/*#@]*Post Types:(.*)$", "mi", &["<?php\n/**\n * Plugin Name: x\n * post types: a, b\n */", "<?php\n/**\n * Plugin Name: x\n */", "POST TYPES:", "Post Type"]),
        (r"\s*wp:core", "", &["<!-- wp:core/image -->", "<!-- wp:paragraph -->"]),
        (r"\d+px|\d+em", "", &["3px", "3em", "3"]),
        (r"(?i)abc(?-i)D", "", &["ABCD", "abcd", "ABCd"]),
        (r"(?i)k", "u", &["\u{212a}", "K"]),
        (r"(?i)straße", "u", &["STRASSE", "straße"]),
        (r"(x)?abc", "", &["abc", "xabc", "ab"]),
        (r"ab{2,}c|q", "", &["abbbc", "abc", "q"]),
    ];
    let vm = MatchOptions { match_limit: 999_999, ..MatchOptions::default() };
    let dfa = MatchOptions::default();
    for &(pat, flags, subjects) in cases {
        let opts = o(flags);
        let re = Regex::new(pat.as_bytes(), &opts).unwrap_or_else(|e| panic!("{pat}: {e}"));
        assert!(re.prog.dfa.is_some(), "{pat}: no DFA");
        for subject in subjects {
            let s = subject.as_bytes();
            for start in 0..=s.len() {
                if opts.utf && start < s.len() && s[start] & 0xc0 == 0x80 {
                    continue;
                }
                for not_empty_atstart in [false, true] {
                    let mut caps = Captures::default();
                    let b = crate::exec::exec_into_dfa(&re.prog, s, start, &MatchOptions { not_empty_atstart, ..dfa.clone() }, &mut caps, true)
                        .unwrap()
                        .then(|| (caps.get(0), caps.len()));
                    // The machine may run out of backtracking where the
                    // DFA, which never backtracks, has the answer.
                    let Ok(a) = re.exec(s, start, &MatchOptions { not_empty_atstart, ..vm.clone() }) else { continue };
                    let a = a.map(|c| (c.get(0), c.len()));
                    assert_eq!(a, b, "{pat:?} /{flags} on {subject:?} from {start} (not_empty_atstart {not_empty_atstart})");
                }
            }
        }
    }
    // Patterns the DFA does not take.
    for pat in [r"(a)\1", r"foo(?=bar)", r"(?<=a)b", r"(?>a+)b", r"a++b", r"\R", r"\X", r"foo\Kbar", r"a(*SKIP)b", r"(?R)?x", r"(?(1)a|b)(a)?", r"(*CRLF)a$", r"(*LIMIT_MATCH=100)a"] {
        let re = Regex::new(pat.as_bytes(), &Options::default()).unwrap_or_else(|e| panic!("{pat}: {e}"));
        assert!(re.prog.dfa.is_none(), "{pat}: has a DFA");
    }
}

/// A search with captures in a subject of 32 to 127 bytes tries the
/// machine alone first: every capture is what the DFA path gives, from
/// every start; one that runs over the machine's budget is the DFA path's.
#[test]
fn machine_first_agrees_with_dfa_path() {
    let pad = |n: usize| "x ".repeat(n);
    let subjects = [
        format!("Hello {{{{ name }}}}, you have {{{{ count }}}} items {}", pad(4)),
        format!("{}/users/42/posts/126?page=6", pad(16)),
        format!("{}[tag a=\"1\" b=\"2\"] and [b] {}", pad(10), pad(12)),
        format!("{}aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaac", pad(8)),
        format!("{}12-34 56-78 {}", pad(30), pad(20)),
    ];
    let patterns = [r"\{\{\s*(\w+)\s*\}\}", r"/users/(\d+)/posts/(\d+)(?:\?page=(\d+))?$", r"\[(\w+)(?:\s+(\w+)=([^\]]*))?\]", r"(a|aa)*([bc])", r"(a*)*[cd]", r"(\d+)-(\d+)", r"(x)?( )", r"(q)|(x)"];
    for pat in patterns {
        let re = Regex::new(pat.as_bytes(), &Options::default()).unwrap_or_else(|e| panic!("{pat}: {e}"));
        assert!(re.prog.dfa.is_some() && re.prog.slots > 2, "{pat}");
        for subject in &subjects {
            let s = subject.as_bytes();
            assert!((32..128).contains(&s.len()), "{subject:?}");
            for start in 0..=s.len() {
                for not_empty_atstart in [false, true] {
                    let opts = MatchOptions { not_empty_atstart, ..MatchOptions::default() };
                    let groups = |caps: &Captures| (0..re.groups() + 1).map(|i| caps.get(i)).collect::<Vec<_>>();
                    let mut a = Captures::default();
                    let a = crate::exec::exec_into(&re.prog, s, start, &opts, &mut a).unwrap().then(|| groups(&a));
                    let mut b = Captures::default();
                    let b = crate::exec::exec_into_dfa(&re.prog, s, start, &opts, &mut b, true).unwrap().then(|| groups(&b));
                    assert_eq!(a, b, "{pat:?} on {subject:?} from {start} (not_empty_atstart {not_empty_atstart})");
                }
            }
        }
    }
    // Over the budget (the machine alone would backtrack for long): the
    // DFA path's answer, not a limit.
    let re = Regex::new(br"(a|aa)*([bc])", &Options::default()).unwrap();
    assert!(re.prog.required.is_none());
    let s = "a".repeat(100);
    let mut caps = Captures::default();
    assert_eq!(crate::exec::exec_into(&re.prog, s.as_bytes(), 0, &MatchOptions::default(), &mut caps), Ok(false));
    let s = format!("{}c", "a".repeat(100));
    assert_eq!(crate::exec::exec_into(&re.prog, s.as_bytes(), 0, &MatchOptions::default(), &mut caps), Ok(true));
    assert_eq!((caps.get(0), caps.get(2)), (Some((0, 101)), Some((100, 101))));
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
