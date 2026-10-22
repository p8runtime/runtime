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
pub fn leading_numeric(s: &[u8]) -> Option<(Number, usize)> {
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
pub fn numeric_string(s: &[u8]) -> Option<Number> {
    let (value, end) = leading_numeric(s)?;
    s[end..].iter().all(|b| WHITESPACE.contains(b)).then_some(value)
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
    }
}
