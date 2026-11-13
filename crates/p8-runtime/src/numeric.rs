//! PHP's numeric strings (zend_operators.c, is_numeric_string_ex), shared
//! by both engines.

/// A number parsed from a string.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Number {
    Int(i64),
    Float(f64),
}

const WHITESPACE: &[u8] = b" \t\n\r\x0b\x0c";

/// The numeric prefix of `s` after leading whitespace (may be empty).
pub fn numeric_prefix(s: &[u8]) -> &[u8] {
    let start = s.iter().position(|b| !WHITESPACE.contains(b)).unwrap_or(s.len());
    let s = &s[start..];
    let digits = |from: usize| s[from..].iter().take_while(|b| b.is_ascii_digit()).count();
    let mut end = usize::from(matches!(s.first(), Some(b'+' | b'-')));
    let int_digits = digits(end);
    end += int_digits;
    let mut frac_digits = 0;
    if s.get(end) == Some(&b'.') {
        frac_digits = digits(end + 1);
        if int_digits + frac_digits > 0 {
            end += 1 + frac_digits;
        }
    }
    if int_digits + frac_digits == 0 {
        return &s[..0];
    }
    if matches!(s.get(end), Some(b'e' | b'E')) {
        let sign = usize::from(matches!(s.get(end + 1), Some(b'+' | b'-')));
        let exp_digits = digits(end + 1 + sign);
        if exp_digits > 0 {
            end += 1 + sign + exp_digits;
        }
    }
    &s[..end]
}

/// The value of the numeric prefix of `s` ("12abc" -> 12) and how many
/// bytes of `s` it covers, or None when `s` does not start with a number.
/// An integer that overflows is a float, as in PHP.
///
/// A number starts with whitespace, a sign, a digit or a `.`, all at or
/// below `'9'`: any other first byte, or an empty string, is "not a
/// number" without the parse (is_numeric_string_ex's `*str > '9'` test,
/// inline in front of the out-of-line parse as in PHP).
#[inline]
pub fn leading_numeric(s: &[u8]) -> Option<(Number, usize)> {
    match s.first() {
        Some(&b) if b <= b'9' => parse_leading_numeric(s),
        _ => None,
    }
}

fn parse_leading_numeric(s: &[u8]) -> Option<(Number, usize)> {
    let prefix = numeric_prefix(s);
    if prefix.is_empty() {
        return None;
    }
    let end = (prefix.as_ptr() as usize - s.as_ptr() as usize) + prefix.len();
    let text = std::str::from_utf8(prefix).ok()?;
    let value = match text.parse::<i64>() {
        Ok(i) if !text.contains(['.', 'e', 'E']) => Number::Int(i),
        _ => Number::Float(text.parse::<f64>().ok()?),
    };
    Some((value, end))
}

/// The value of `s` if the whole string is numeric (leading and trailing
/// whitespace allowed, as in PHP 8).
#[inline]
pub fn numeric_string(s: &[u8]) -> Option<Number> {
    let (value, end) = leading_numeric(s)?;
    s[end..].iter().all(|b| WHITESPACE.contains(b)).then_some(value)
}

/// Perl-style increment of an alphanumeric string ("a" -> "b", "Az" ->
/// "Ba", "zz" -> "aaa"), as zend_operators.c increment_string does after
/// ruling out numeric strings. Bytes after the last alphanumeric are kept.
pub fn increment_alphanumeric(s: &mut Vec<u8>) {
    let mut pos = s.len();
    while pos > 0 {
        pos -= 1;
        let (next, carry, first) = match s[pos] {
            b'0'..=b'8' | b'a'..=b'y' | b'A'..=b'Y' => (s[pos] + 1, false, 0),
            b'9' => (b'0', true, b'1'),
            b'z' => (b'a', true, b'a'),
            b'Z' => (b'A', true, b'A'),
            _ => return,
        };
        s[pos] = next;
        if !carry {
            return;
        }
        if pos == 0 {
            s.insert(0, first);
            return;
        }
    }
}

/// An int's decimal digits, appended (what PHP prints for an int).
pub fn push_int(out: &mut Vec<u8>, i: i64) {
    push_digits(out, i.unsigned_abs(), 10, i < 0);
}

/// `n` in base `radix` (2 to 16, lowercase), appended, after a `-` if
/// `negative`.
pub fn push_digits(out: &mut Vec<u8>, mut n: u64, radix: u64, negative: bool) {
    let mut buf = [0u8; 64];
    let mut at = buf.len();
    loop {
        at -= 1;
        buf[at] = b"0123456789abcdef"[(n % radix) as usize];
        n /= radix;
        if n == 0 {
            break;
        }
    }
    if negative {
        out.push(b'-');
    }
    out.extend_from_slice(&buf[at..]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_strings_follow_php_8() {
        assert_eq!(numeric_string(b" 12 "), Some(Number::Int(12)));
        assert_eq!(numeric_string(b"1e3"), Some(Number::Float(1000.0)));
        assert_eq!(numeric_string(b".5"), Some(Number::Float(0.5)));
        assert_eq!(numeric_string(b"12abc"), None);
        assert_eq!(leading_numeric(b"12abc"), Some((Number::Int(12), 2)));
        assert_eq!(numeric_string(b"9223372036854775808"), Some(Number::Float(9.223372036854776e18)));
        assert_eq!(numeric_string(b""), None);
        assert_eq!(numeric_string(b"-"), None);
        assert_eq!(numeric_string(b"1e"), None);
        assert_eq!(leading_numeric(b"1e"), Some((Number::Int(1), 1)));
        // Every byte a number can start with is at or below '9'.
        for (s, n) in [(&b" 1"[..], Some(Number::Int(1))), (b"\t-1", Some(Number::Int(-1))), (b"+1", Some(Number::Int(1))), (b".5", Some(Number::Float(0.5))), (b"9", Some(Number::Int(9)))] {
            assert_eq!(leading_numeric(s).map(|(v, _)| v), n, "{s:?}");
        }
        for s in [&b"x1"[..], b":1", b"A1", b"_1", b"~1", b"\xd9\xa1", b"\x001", b"", b" ", b"-", b"."] {
            assert_eq!(leading_numeric(s), None, "{s:?}");
        }
    }

    #[test]
    fn alphanumeric_increment() {
        for (from, to) in [("a", "b"), ("z", "aa"), ("Az", "Ba"), ("zZ9", "aaA0"), ("a9", "b0"), ("a-", "a-")] {
            let mut s = from.as_bytes().to_vec();
            increment_alphanumeric(&mut s);
            assert_eq!(s, to.as_bytes(), "{from}");
        }
    }
}
