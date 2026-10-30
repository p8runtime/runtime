//! An exact decimal number for formatting (ICU's DecimalQuantity): the
//! digits of an integer or of a double's shortest round-trip form, and
//! rounding in decimal with ICU's rounding modes.

/// `digits` × 10^(`point` − digits.len()), no leading or trailing zero digit
/// (zero is no digits).
#[derive(Clone, Debug, PartialEq)]
pub struct Dec {
    pub neg: bool,
    pub digits: Vec<u8>,
    /// How many of the digits are before the decimal point (may be negative
    /// or past the end).
    pub point: i32,
}

pub const ROUND_CEILING: i64 = 0;
pub const ROUND_FLOOR: i64 = 1;
pub const ROUND_DOWN: i64 = 2;
pub const ROUND_UP: i64 = 3;
pub const ROUND_HALFEVEN: i64 = 4;
pub const ROUND_HALFDOWN: i64 = 5;
pub const ROUND_HALFUP: i64 = 6;
pub const ROUND_UNNECESSARY: i64 = 7;
pub const ROUND_HALFODD: i64 = 8;
pub const ROUND_HALF_CEILING: i64 = 9;
pub const ROUND_HALF_FLOOR: i64 = 10;

impl Dec {
    pub fn from_i64(v: i64) -> Dec {
        let mut d = Dec { neg: v < 0, digits: v.unsigned_abs().to_string().into_bytes().iter().map(|c| c - b'0').collect(), point: 0 };
        d.point = d.digits.len() as i32;
        d.trim();
        d
    }

    /// A finite double, by its shortest round-trip digits.
    pub fn from_f64(v: f64) -> Dec {
        let neg = v.is_sign_negative();
        let a = v.abs();
        if a == 0.0 {
            return Dec { neg, digits: Vec::new(), point: 0 };
        }
        let s = format!("{a:e}");
        let (mant, exp) = s.split_once('e').unwrap_or((&s, "0"));
        let exp: i32 = exp.parse().unwrap_or(0);
        let digits: Vec<u8> = mant.bytes().filter(u8::is_ascii_digit).map(|c| c - b'0').collect();
        let mut d = Dec { neg, digits, point: exp + 1 };
        d.trim();
        d
    }

    /// From a decimal string ("-12.50", "1e3"); None when it is not one.
    pub fn parse(s: &str) -> Option<Dec> {
        let s = s.trim();
        let (neg, s) = match s.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, s.strip_prefix('+').unwrap_or(s)),
        };
        let (mant, exp) = match s.find(['e', 'E']) {
            Some(i) => (&s[..i], s[i + 1..].parse::<i32>().ok()?),
            None => (s, 0),
        };
        let (int, frac) = mant.split_once('.').unwrap_or((mant, ""));
        if int.is_empty() && frac.is_empty() || !int.bytes().chain(frac.bytes()).all(|c| c.is_ascii_digit()) {
            return None;
        }
        let mut digits: Vec<u8> = int.bytes().chain(frac.bytes()).map(|c| c - b'0').collect();
        let mut point = int.len() as i32 + exp;
        while digits.first() == Some(&0) {
            digits.remove(0);
            point -= 1;
        }
        let mut d = Dec { neg, digits, point };
        d.trim();
        Some(d)
    }

    fn trim(&mut self) {
        while self.digits.last() == Some(&0) {
            self.digits.pop();
        }
        while self.digits.first() == Some(&0) {
            self.digits.remove(0);
            self.point -= 1;
        }
        if self.digits.is_empty() {
            self.point = 0;
        }
    }

    pub fn is_zero(&self) -> bool {
        self.digits.is_empty()
    }

    /// The digit at decimal position `p` (0: units, 1: tens, -1: tenths).
    pub fn digit_at(&self, p: i32) -> u8 {
        let i = self.point - 1 - p;
        if i < 0 || i as usize >= self.digits.len() { 0 } else { self.digits[i as usize] }
    }

    /// The position of the most significant digit (0 for units), or None for zero.
    pub fn magnitude(&self) -> Option<i32> {
        if self.is_zero() { None } else { Some(self.point - 1) }
    }

    /// The position of the least significant non-zero digit.
    pub fn lowest(&self) -> i32 {
        self.point - self.digits.len() as i32
    }

    pub fn shift(&mut self, n: i32) {
        if !self.is_zero() {
            self.point += n;
        }
    }

    /// × a positive integer.
    pub fn multiply(&mut self, m: u64) {
        if self.is_zero() || m == 1 {
            return;
        }
        let low = self.lowest();
        let mut out: Vec<u64> = vec![0; self.digits.len() + 21];
        // digits reversed
        let rev: Vec<u8> = self.digits.iter().rev().copied().collect();
        let mut carry: u128 = 0;
        for (i, slot) in out.iter_mut().enumerate() {
            let d = rev.get(i).copied().unwrap_or(0) as u128;
            let v = d * m as u128 + carry;
            *slot = (v % 10) as u64;
            carry = v / 10;
        }
        let mut digits: Vec<u8> = out.iter().rev().map(|&d| d as u8).collect();
        let lead = digits.iter().take_while(|&&d| d == 0).count();
        digits.drain(..lead);
        let len = digits.len() as i32;
        self.digits = digits;
        self.point = low + len;
        self.trim();
    }

    /// Round so that no digit is below position `pos` (−2: hundredths).
    /// Returns false when ROUND_UNNECESSARY would have to round.
    pub fn round_at(&mut self, pos: i32, mode: i64) -> bool {
        if self.is_zero() || self.lowest() >= pos {
            return true;
        }
        // keep: digits at positions >= pos
        let keep = (self.point - pos).max(0) as usize;
        let keep = keep.min(self.digits.len());
        let first_dropped = if self.point - pos < 0 { 0 } else { self.digits.get(keep).copied().unwrap_or(0) };
        let first_dropped = if self.point - pos < 0 { 0 } else { first_dropped };
        let rest_nonzero = if self.point - pos < 0 { true } else { self.digits[keep + 1..].iter().any(|&d| d != 0) };
        // half comparison: >0.5, =0.5, <0.5 of a unit at pos
        let half = if self.point - pos < 0 {
            std::cmp::Ordering::Less
        } else {
            match first_dropped.cmp(&5) {
                std::cmp::Ordering::Equal if rest_nonzero => std::cmp::Ordering::Greater,
                o => o,
            }
        };
        let last_kept_odd = keep > 0 && self.digits[keep - 1] % 2 == 1;
        let up = match mode {
            ROUND_CEILING => !self.neg,
            ROUND_FLOOR => self.neg,
            ROUND_DOWN => false,
            ROUND_UP => true,
            ROUND_HALFEVEN => half == std::cmp::Ordering::Greater || (half == std::cmp::Ordering::Equal && last_kept_odd),
            ROUND_HALFODD => half == std::cmp::Ordering::Greater || (half == std::cmp::Ordering::Equal && !last_kept_odd),
            ROUND_HALFDOWN => half == std::cmp::Ordering::Greater,
            ROUND_HALFUP => half != std::cmp::Ordering::Less,
            ROUND_HALF_CEILING => half == std::cmp::Ordering::Greater || (half == std::cmp::Ordering::Equal && !self.neg),
            ROUND_HALF_FLOOR => half == std::cmp::Ordering::Greater || (half == std::cmp::Ordering::Equal && self.neg),
            ROUND_UNNECESSARY => return false,
            _ => half == std::cmp::Ordering::Greater || (half == std::cmp::Ordering::Equal && last_kept_odd),
        };
        let mut digits: Vec<u8> = self.digits[..keep].to_vec();
        let mut point = self.point;
        if self.point - pos < 0 {
            digits.clear();
            point = pos;
        }
        if up {
            // add one unit at the last kept position
            let mut len = (point - pos) as usize;
            digits.resize(len, 0);
            let mut i = len;
            loop {
                if i == 0 {
                    digits.insert(0, 1);
                    point += 1;
                    len += 1;
                    break;
                }
                i -= 1;
                if digits[i] == 9 {
                    digits[i] = 0;
                } else {
                    digits[i] += 1;
                    break;
                }
            }
            let _ = len;
        }
        self.digits = digits;
        self.point = point;
        self.trim();
        true
    }

    /// Round to a multiple of `inc` (> 0).
    pub fn round_increment(&mut self, inc: &Dec, mode: i64) {
        if inc.is_zero() {
            return;
        }
        // inc = m × 10^low
        let low = inc.lowest();
        let m: u64 = inc.digits.iter().fold(0u64, |a, &d| a.saturating_mul(10).saturating_add(d as u64));
        if m == 1 {
            self.round_at(low, mode);
            return;
        }
        // q = self / 10^low, divided by m: long division of the integer part
        let mut scaled = self.clone();
        scaled.point -= low;
        scaled.neg = false;
        let int_digits: Vec<u8> = (0..scaled.point.max(0)).rev().map(|p| scaled.digit_at(p)).collect();
        let frac_nonzero = scaled.lowest() < 0 && !scaled.is_zero();
        let mut q: Vec<u8> = Vec::new();
        let mut r: u128 = 0;
        for d in int_digits {
            r = r * 10 + d as u128;
            q.push((r / m as u128) as u8);
            r %= m as u128;
        }
        // remainder fraction: (r + frac) / m against one half
        let twice = 2 * r;
        let half = match twice.cmp(&(m as u128)) {
            std::cmp::Ordering::Equal if frac_nonzero => std::cmp::Ordering::Greater,
            o => o,
        };
        let exact = r == 0 && !frac_nonzero;
        let mut qd = Dec { neg: false, digits: q.clone(), point: q.len() as i32 };
        qd.trim();
        let last_odd = qd.digit_at(0) % 2 == 1;
        let up = if exact {
            false
        } else {
            match mode {
                ROUND_CEILING => !self.neg,
                ROUND_FLOOR => self.neg,
                ROUND_DOWN => false,
                ROUND_UP => true,
                ROUND_HALFDOWN => half == std::cmp::Ordering::Greater,
                ROUND_HALFUP => half != std::cmp::Ordering::Less,
                ROUND_HALFODD => half == std::cmp::Ordering::Greater || (half == std::cmp::Ordering::Equal && !last_odd),
                _ => half == std::cmp::Ordering::Greater || (half == std::cmp::Ordering::Equal && last_odd),
            }
        };
        if up {
            let one = Dec::from_i64(1);
            qd = qd.add_abs(&one);
        }
        qd.multiply(m);
        qd.shift(low);
        qd.neg = self.neg;
        *self = qd;
    }

    fn add_abs(&self, other: &Dec) -> Dec {
        let low = self.lowest().min(other.lowest());
        let high = self.point.max(other.point) + 1;
        let mut out = Vec::new();
        let mut carry = 0;
        for p in low..high {
            let s = self.digit_at(p) + other.digit_at(p) + carry;
            out.push(s % 10);
            carry = s / 10;
        }
        out.reverse();
        let mut d = Dec { neg: self.neg, digits: out, point: high };
        d.trim();
        d
    }

    /// Keep only the lowest `n` integer digits (ICU's maximum integer digits).
    pub fn truncate_int(&mut self, n: i32) {
        if self.point > n {
            let drop = (self.point - n) as usize;
            if drop >= self.digits.len() {
                self.digits.clear();
                self.point = 0;
            } else {
                self.digits.drain(..drop);
                self.point = n;
            }
            self.trim();
        }
    }

    pub fn to_f64(&self) -> f64 {
        let s = self.to_plain();
        s.parse().unwrap_or(0.0)
    }

    /// "-123.45".
    pub fn to_plain(&self) -> String {
        let mut s = String::new();
        if self.neg {
            s.push('-');
        }
        if self.point <= 0 {
            s.push('0');
        } else {
            for p in (0..self.point).rev() {
                s.push((b'0' + self.digit_at(p)) as char);
            }
        }
        if self.lowest() < 0 {
            s.push('.');
            for p in (self.lowest()..0).rev() {
                s.push((b'0' + self.digit_at(p)) as char);
            }
        }
        s
    }

    /// The value as an i64 when it is an integer in range.
    pub fn to_i64(&self) -> Option<i64> {
        if self.lowest() < 0 && !self.is_zero() {
            return None;
        }
        let mut v: i128 = 0;
        for p in (0..self.point.max(0)).rev() {
            v = v * 10 + self.digit_at(p) as i128;
            if v > i64::MAX as i128 + 1 {
                return None;
            }
        }
        if self.neg {
            v = -v;
        }
        i64::try_from(v).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_like_icu() {
        let mut d = Dec::from_f64(2.5);
        d.round_at(0, ROUND_HALFEVEN);
        assert_eq!(d.to_plain(), "2");
        let mut d = Dec::from_f64(1234.5678);
        d.round_at(-3, ROUND_HALFEVEN);
        assert_eq!(d.to_plain(), "1234.568");
        let mut d = Dec::from_f64(-0.001);
        d.round_at(-2, ROUND_HALFEVEN);
        assert!(d.is_zero() && d.neg);
        let mut d = Dec::from_f64(999.96);
        d.round_at(-1, ROUND_HALFEVEN);
        assert_eq!(d.to_plain(), "1000");
        let mut d = Dec::from_f64(1234.5678);
        d.round_increment(&Dec::parse("0.05").unwrap(), ROUND_HALFEVEN);
        assert_eq!(d.to_plain(), "1234.55");
        let mut d = Dec::from_f64(0.01234);
        d.multiply(100);
        assert_eq!(d.to_plain(), "1.234");
        assert_eq!(Dec::from_f64(1e20).to_plain(), "100000000000000000000");
        assert_eq!(Dec::from_i64(i64::MIN).to_plain(), "-9223372036854775808");
    }
}
