//! The values of literal tokens, as Zend's scanner computes them.

/// An integer literal's value: an int, or a float when too large for one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Number {
    Int(i64),
    Float(f64),
}

/// Unescape a double-quoted string (or heredoc / interpolated part) the way
/// Zend's scanner does: \n \t \r \v \e \f \\ \$ \", octal \[0-7]{1,3},
/// hex \x[0-9a-f]{1,2} and \u{codepoint}; any other backslash is kept.
/// Reference: $PHP_SRC_PATH/Zend/zend_language_scanner.l - zend_scan_escape_string
pub fn unescape(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] != b'\\' || i + 1 >= s.len() {
            out.push(s[i]);
            i += 1;
            continue;
        }
        let c = s[i + 1];
        i += 2;
        match c {
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'r' => out.push(b'\r'),
            b'v' => out.push(0x0B),
            b'e' => out.push(0x1B),
            b'f' => out.push(0x0C),
            b'\\' | b'$' | b'"' => out.push(c),
            b'0'..=b'7' => {
                let mut value = u32::from(c - b'0');
                let mut digits = 1;
                while digits < 3 && i < s.len() && (b'0'..=b'7').contains(&s[i]) {
                    value = value * 8 + u32::from(s[i] - b'0');
                    i += 1;
                    digits += 1;
                }
                out.push(value as u8); // "\400" wraps, as in PHP
            }
            b'x' if i < s.len() && s[i].is_ascii_hexdigit() => {
                let mut value = 0u32;
                let mut digits = 0;
                while digits < 2 && i < s.len() && s[i].is_ascii_hexdigit() {
                    value = value * 16 + char::from(s[i]).to_digit(16).unwrap_or(0);
                    i += 1;
                    digits += 1;
                }
                out.push(value as u8);
            }
            b'u' if s.get(i) == Some(&b'{') => {
                let end = s[i..].iter().position(|b| *b == b'}').map(|p| i + p);
                let code = end.and_then(|end| {
                    std::str::from_utf8(&s[i + 1..end])
                        .ok()
                        .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                        .and_then(char::from_u32)
                });
                match (end, code) {
                    (Some(end), Some(ch)) => {
                        let mut buf = [0u8; 4];
                        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        i = end + 1;
                    }
                    _ => out.extend_from_slice(b"\\u"),
                }
            }
            _ => {
                out.push(b'\\');
                out.push(c);
            }
        }
    }
    out
}

/// The value of an integer literal: decimal, 0x hex, 0b binary, 0o or
/// leading-0 octal, with `_` separators; too large for an int, it is a float.
/// Reference: $PHP_SRC_PATH/Zend/zend_language_scanner.l - LNUM/HNUM/BNUM/ONUM
pub fn integer(value: &[u8]) -> Number {
    let digits: Vec<u8> = value.iter().copied().filter(|b| *b != b'_').collect();
    let (radix, body) = match digits.as_slice() {
        [b'0', b'x' | b'X', rest @ ..] => (16, rest),
        [b'0', b'b' | b'B', rest @ ..] => (2, rest),
        [b'0', b'o' | b'O', rest @ ..] => (8, rest),
        [b'0', rest @ ..] if !rest.is_empty() => (8, rest),
        all => (10, all),
    };
    let text = std::str::from_utf8(body).unwrap_or("0");
    match i64::from_str_radix(text, radix) {
        Ok(i) => Number::Int(i),
        // Too large for an int: a float, correctly rounded for decimals
        Err(_) if radix == 10 => Number::Float(text.parse().unwrap_or(0.0)),
        Err(_) => {
            let f = text
                .bytes()
                .filter_map(|b| char::from(b).to_digit(radix))
                .fold(0.0, |acc, d| acc * f64::from(radix) + f64::from(d));
            Number::Float(f)
        }
    }
}

/// The value of a float literal (`_` separators allowed).
pub fn float(value: &[u8]) -> f64 {
    let text: String = value.iter().filter(|b| **b != b'_').map(|b| char::from(*b)).collect();
    text.parse().unwrap_or(0.0)
}

/// The value of a string literal token: '...' (only \\ and \' are escapes),
/// "..." (full escapes) or an unquoted interpolation/heredoc part.
pub fn string(value: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    use std::borrow::Cow;
    let quoted = |q: u8| value.len() >= 2 && value[0] == q && value[value.len() - 1] == q;
    // Without a backslash the value is its own bytes: nothing to copy.
    let plain = |inner: &[u8]| !inner.contains(&b'\\');
    if quoted(b'\'') {
        let inner = &value[1..value.len() - 1];
        if plain(inner) {
            return Cow::Borrowed(inner);
        }
        let mut out = Vec::with_capacity(inner.len());
        let mut i = 0;
        while i < inner.len() {
            if inner[i] == b'\\' && matches!(inner.get(i + 1), Some(b'\'' | b'\\')) {
                out.push(inner[i + 1]);
                i += 2;
            } else {
                out.push(inner[i]);
                i += 1;
            }
        }
        Cow::Owned(out)
    } else if quoted(b'"') {
        let inner = &value[1..value.len() - 1];
        if plain(inner) { Cow::Borrowed(inner) } else { Cow::Owned(unescape(inner)) }
    } else if plain(value) {
        Cow::Borrowed(value)
    } else {
        Cow::Owned(unescape(value))
    }
}
