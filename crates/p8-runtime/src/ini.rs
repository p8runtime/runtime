//! ini values as PHP reads them.
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_ini.c (zend_ini_parse_quantity)

/// A quantity ("128M", "8k", "0x10", "-1"): an integer, then an optional
/// K, M or G (any case) multiplying it by 1024, 1024² or 1024³. What does
/// not read as a number is 0, as zend_ini_parse_quantity gives (it warns).
pub fn quantity(text: &[u8]) -> i64 {
    let s = text.trim_ascii();
    let (negative, s) = match s.split_first() {
        Some((b'-', rest)) => (true, rest),
        Some((b'+', rest)) => (false, rest),
        _ => (false, s),
    };
    let (radix, s) = match s {
        [b'0', b'x' | b'X', rest @ ..] => (16, rest),
        [b'0', b'o' | b'O', rest @ ..] => (8, rest),
        [b'0', b'b' | b'B', rest @ ..] => (2, rest),
        _ => (10, s),
    };
    let digits = s.iter().take_while(|c| (**c as char).is_digit(radix)).count();
    let mut n: i64 = 0;
    for &c in &s[..digits] {
        n = n.saturating_mul(i64::from(radix)).saturating_add((c as char).to_digit(radix).map_or(0, i64::from));
    }
    let factor = match s.get(digits..).map(<[u8]>::trim_ascii) {
        Some([b'k' | b'K']) => 1 << 10,
        Some([b'm' | b'M']) => 1 << 20,
        Some([b'g' | b'G']) => 1 << 30,
        _ => 1,
    };
    let n = n.saturating_mul(factor);
    if negative { -n } else { n }
}

#[cfg(test)]
mod tests {
    use super::quantity;

    #[test]
    fn quantities_read_as_php_reads_them() {
        assert_eq!(quantity(b"128M"), 128 << 20);
        assert_eq!(quantity(b"8k"), 8 << 10);
        assert_eq!(quantity(b" 2 "), 2);
        assert_eq!(quantity(b"-1"), -1);
        assert_eq!(quantity(b"0x10"), 16);
        assert_eq!(quantity(b"1G"), 1 << 30);
        assert_eq!(quantity(b"abc"), 0);
    }
}
