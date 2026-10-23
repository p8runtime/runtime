//! PHP's float conversions: to int (zend_dval_to_lval) and to text (zend_gcvt).

/// A float as an int, as zend_dval_to_lval does on 64-bit: NAN and the
/// infinities are 0, and values out of range wrap modulo 2^64.
pub fn dval_to_lval(d: f64) -> i64 {
    if !d.is_finite() {
        return 0;
    }
    if (-9.223_372_036_854_776e18..9.223_372_036_854_776e18).contains(&d) {
        return d as i64;
    }
    // zend_dval_to_lval_slow (64-bit): the value modulo 2^64, as unsigned
    let two_pow_64 = 18_446_744_073_709_551_616.0_f64;
    let mut dmod = d % two_pow_64;
    if dmod < 0.0 {
        dmod += two_pow_64;
    }
    dmod as u64 as i64
}

/// How many significant digits a float is printed with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precision {
    /// PHP's `precision` ini: echo, string casts, print_r.
    Digits(usize),
    /// PHP's `serialize_precision = -1`: shortest round-trip digits; var_dump,
    /// var_export, json_encode and serialize.
    Shortest,
}

impl Precision {
    /// The `precision` ini default (14).
    pub const ECHO: Precision = Precision::Digits(14);
    /// The `serialize_precision` ini default (-1).
    pub const SERIALIZE: Precision = Precision::Shortest;

    /// From an ini value: -1 means the shortest round-trip form, 0 one digit.
    pub fn from_ini(value: i64) -> Precision {
        if value < 0 {
            Precision::Shortest
        } else {
            Precision::Digits((value as usize).clamp(1, 40))
        }
    }
}

/// Format a float the way zend_gcvt does: `precision` significant digits,
/// exponent form (`1.0E+25`) when the decimal point falls outside the digits.
/// Reference: $PHP_SRC_PATH/main/snprintf.c - php_gcvt
pub fn float_to_string(f: f64, precision: Precision, exp_char: char) -> String {
    if f.is_nan() {
        return "NAN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "INF" } else { "-INF" }.into();
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0" } else { "0" }.into();
    }
    let (sci, ndigit) = match precision {
        Precision::Shortest => (format!("{:e}", f.abs()), 17),
        Precision::Digits(p) => {
            let p = p.max(1);
            (format!("{:.*e}", p - 1, f.abs()), p as i64)
        }
    };
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let mut digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    while digits.len() > 1 && digits.ends_with('0') {
        digits.pop();
    }
    let decpt = exp.parse::<i64>().unwrap_or(0) + 1;
    let mut out = String::new();
    if f < 0.0 {
        out.push('-');
    }
    if decpt < -3 || decpt > ndigit {
        out.push_str(&digits[..1]);
        out.push('.');
        out.push_str(if digits.len() > 1 { &digits[1..] } else { "0" });
        out.push(exp_char);
        out.push(if decpt - 1 < 0 { '-' } else { '+' });
        out.push_str(&(decpt - 1).abs().to_string());
    } else if decpt <= 0 {
        out.push_str("0.");
        out.extend(std::iter::repeat_n('0', (-decpt) as usize));
        out.push_str(&digits);
    } else if digits.len() as i64 <= decpt {
        out.push_str(&digits);
        out.extend(std::iter::repeat_n('0', decpt as usize - digits.len()));
    } else {
        out.push_str(&digits[..decpt as usize]);
        out.push('.');
        out.push_str(&digits[decpt as usize..]);
    }
    out
}
