//! ext/standard's math functions. Arguments arrive already checked and
//! converted to their declared types (p8_runtime::args).
//!
//! Reference: $PHP_SRC_PATH/ext/standard/math.c

use crate::arg;
use p8_runtime::ext::{Builtin, ClassSpec, Constant, Ctx, Fault, FastBuiltin, Throw};
use p8_runtime::lent;
use p8_runtime::object::CLASS_ENUM;
use p8_runtime::ops::{self, BinOp};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cmp::Ordering;

/// The math functions on the borrowed ABI (frameless when plainly typed).
pub static FAST: &[FastBuiltin] = &[
    FastBuiltin { name: "abs", run: abs },
];

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "ceil", run: ceil, by_ref: &[] },
    Builtin { name: "floor", run: floor, by_ref: &[] },
    Builtin { name: "round", run: round, by_ref: &[] },
    Builtin { name: "intdiv", run: intdiv, by_ref: &[] },
    Builtin { name: "fmod", run: fmod, by_ref: &[] },
    Builtin { name: "fdiv", run: fdiv, by_ref: &[] },
    Builtin { name: "pow", run: pow, by_ref: &[] },
    Builtin { name: "max", run: max, by_ref: &[] },
    Builtin { name: "min", run: min, by_ref: &[] },
    Builtin { name: "pi", run: pi, by_ref: &[] },
    Builtin { name: "is_nan", run: is_nan, by_ref: &[] },
    Builtin { name: "is_finite", run: is_finite, by_ref: &[] },
    Builtin { name: "is_infinite", run: is_infinite, by_ref: &[] },
    Builtin { name: "sqrt", run: sqrt, by_ref: &[] },
    Builtin { name: "exp", run: exp, by_ref: &[] },
    Builtin { name: "log", run: log, by_ref: &[] },
    Builtin { name: "log10", run: log10, by_ref: &[] },
    Builtin { name: "sin", run: sin, by_ref: &[] },
    Builtin { name: "cos", run: cos, by_ref: &[] },
    Builtin { name: "tan", run: tan, by_ref: &[] },
    Builtin { name: "asin", run: asin, by_ref: &[] },
    Builtin { name: "acos", run: acos, by_ref: &[] },
    Builtin { name: "atan", run: atan, by_ref: &[] },
    Builtin { name: "atan2", run: atan2, by_ref: &[] },
    Builtin { name: "hypot", run: hypot, by_ref: &[] },
    Builtin { name: "deg2rad", run: deg2rad, by_ref: &[] },
    Builtin { name: "rad2deg", run: rad2deg, by_ref: &[] },
    Builtin { name: "decbin", run: decbin, by_ref: &[] },
    Builtin { name: "decoct", run: decoct, by_ref: &[] },
    Builtin { name: "bindec", run: bindec, by_ref: &[] },
    Builtin { name: "octdec", run: octdec, by_ref: &[] },
    Builtin { name: "hexdec", run: hexdec, by_ref: &[] },
    Builtin { name: "base_convert", run: base_convert, by_ref: &[] },
];

fn float(args: &[Value], i: usize) -> f64 {
    arg(args, i).to_float()
}

fn abs(_: &mut dyn Ctx, args: &[Value]) -> Result<Value, Fault> {
    let v = lent::at(args, 0);
    Ok(match v.as_int() {
        Some(i) => i.checked_abs().map_or(Value::float(-(i as f64)), Value::int),
        None => Value::float(lent::float(args, 0).abs()),
    })
}

fn ceil(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::float(float(args, 0).ceil()))
}

fn floor(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::float(float(args, 0).floor()))
}

const HALF_UP: i64 = 1;
const HALF_DOWN: i64 = 2;
const HALF_EVEN: i64 = 3;
const HALF_ODD: i64 = 4;
const CEILING: i64 = 5;
const FLOOR: i64 = 6;
const TOWARD_ZERO: i64 = 7;
const AWAY_FROM_ZERO: i64 = 8;

/// enum RoundingMode (PHP 8.4, basic_functions.stub.php), a pure enum:
/// round()'s `$mode`, each case one of the PHP_ROUND_* modes
/// (ROUNDING_MODES, in case order).
pub const CLASSES: &[ClassSpec] = &[ClassSpec {
    name: "RoundingMode",
    parent: None,
    interfaces: &[],
    // Not final: zend_register_internal_enum sets only ZEND_ACC_ENUM.
    flags: CLASS_ENUM,
    properties: &[],
    methods: &[],
    constants: &[
        ("HalfAwayFromZero", Constant::Null),
        ("HalfTowardsZero", Constant::Null),
        ("HalfEven", Constant::Null),
        ("HalfOdd", Constant::Null),
        ("TowardsZero", Constant::Null),
        ("AwayFromZero", Constant::Null),
        ("NegativeInfinity", Constant::Null),
        ("PositiveInfinity", Constant::Null),
    ],
    debug_info: None,
}];

/// php_math_round_mode_from_enum: RoundingMode's cases, in order, as modes.
const ROUNDING_MODES: [i64; 8] = [HALF_UP, HALF_DOWN, HALF_EVEN, HALF_ODD, TOWARD_ZERO, AWAY_FROM_ZERO, FLOOR, CEILING];

/// round($num, $precision = 0, int|RoundingMode $mode = RoundingMode::HalfAwayFromZero)
fn round(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let places = if args.len() > 1 { arg(args, 1).to_int().clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32 } else { 0 };
    let mode = match args.get(2).map(Value::deref) {
        None => HALF_UP,
        // A RoundingMode case (the argument check let no other object in).
        Some(v) => match v.as_object() {
            Some(o) => {
                let name = o.class.read(o, b"name", &o.class.lower);
                let case = CLASSES[0].constants.iter().position(|(n, _)| name.to_php_string().is_some_and(|s| s.as_bytes() == n.as_bytes()));
                case.map_or(HALF_UP, |i| ROUNDING_MODES[i])
            }
            None => v.to_int(),
        },
    };
    if !(HALF_UP..=AWAY_FROM_ZERO).contains(&mode) {
        return Err(ctx.throw("ValueError", "round(): Argument #3 ($mode) must be a valid rounding mode (RoundingMode::*)".into()));
    }
    let v = arg(args, 0);
    Ok(Value::float(match v.as_int() {
        Some(i) if places >= 0 => i as f64,
        _ => php_round(v.to_float(), places, mode),
    }))
}

fn intpow10(power: i32) -> f64 {
    if !(0..=22).contains(&power) {
        return 10f64.powi(power);
    }
    [1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16, 1e17, 1e18, 1e19, 1e20, 1e21, 1e22]
        [power as usize]
}

/// _php_math_round: round to `places` decimals in `mode`, correcting for
/// the error in scaling a decimal by a power of ten.
pub fn php_round(value: f64, places: i32, mode: i64) -> f64 {
    if !value.is_finite() || value == 0.0 {
        return value;
    }
    if places == 0 && value == value.trunc() {
        return value;
    }
    let places = places.max(i32::MIN + 1);
    let exponent = intpow10(places.abs());
    let scaled = if places > 0 { value * exponent } else { value / exponent };
    let (mut tmp, tmp2) = if value >= 0.0 { (scaled.floor(), scaled.floor() + 1.0) } else { (scaled.ceil(), scaled.ceil() - 1.0) };
    if (if places > 0 { tmp2 / exponent } else { tmp2 * exponent }) == value {
        tmp = tmp2;
    }
    if tmp.abs() >= 1e16 {
        return value;
    }
    tmp = round_helper(tmp, value, exponent, places, mode);
    if places.abs() < 23 {
        if places > 0 { tmp / exponent } else { tmp * exponent }
    } else {
        // "%15fe%d" read back with strtod: the nearest float to that text.
        format!("{tmp:15.6}e{}", -places).trim().parse::<f64>().ok().filter(|f| f.is_finite()).unwrap_or(value)
    }
}

fn round_helper(integral: f64, value: f64, exponent: f64, places: i32, mode: i64) -> f64 {
    let value_abs = value.abs();
    let scale = |x: f64| if places > 0 { (x / exponent).abs() } else { (x * exponent).abs() };
    let basic_edge = || scale(integral + 0.5f64.copysign(integral));
    let zero_edge = || scale(integral);
    let away = integral + 1f64.copysign(integral);
    match mode {
        HALF_UP if value_abs >= basic_edge() => away,
        HALF_DOWN if value_abs > basic_edge() => away,
        HALF_EVEN | HALF_ODD => {
            let edge = basic_edge();
            let even = integral % 2.0 == 0.0;
            if value_abs > edge || (value_abs == edge && even != (mode == HALF_EVEN)) { away } else { integral }
        }
        CEILING if value > 0.0 && value_abs > zero_edge() => integral + 1.0,
        FLOOR if value < 0.0 && value_abs > zero_edge() => integral - 1.0,
        AWAY_FROM_ZERO if value_abs > zero_edge() => away,
        TOWARD_ZERO => integral,
        _ => integral,
    }
}

fn intdiv(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (a, b) = (arg(args, 0).to_int(), arg(args, 1).to_int());
    if b == 0 {
        return Err(ctx.throw("DivisionByZeroError", "Division by zero".into()));
    }
    match a.checked_div(b) {
        Some(q) => Ok(Value::int(q)),
        None => Err(ctx.throw("ArithmeticError", "Division of PHP_INT_MIN by -1 is not an integer".into())),
    }
}

fn fmod(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::float(float(args, 0) % float(args, 1)))
}

fn fdiv(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::float(float(args, 0) / float(args, 1)))
}

/// pow() is the ** operator.
fn pow(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mut diags = Vec::new();
    let r = ops::binary(BinOp::Pow, &arg(args, 0), &arg(args, 1), &mut diags);
    for d in diags {
        match d {
            ops::Diag::Warning(m) => ctx.warn(&m),
            ops::Diag::Deprecated(m) => ctx.deprecated(&m),
        }
    }
    r.map_err(|e| match e {
        ops::OpError::Type(m) => ctx.throw("TypeError", m),
        ops::OpError::DivisionByZero(m) => ctx.throw("DivisionByZeroError", m.into()),
        ops::OpError::Arithmetic(m) => ctx.throw("ArithmeticError", m.into()),
        ops::OpError::Error(m) => ctx.throw("Error", m),
    })
}

/// max/min: of one array's values, or of the arguments themselves.
fn extreme(ctx: &dyn Ctx, func: &str, args: &[Value], want: Ordering) -> Result<Value, Throw> {
    let first = arg(args, 0);
    let values: Vec<Value> = match (args.len(), first.as_array()) {
        (1, Some(a)) => {
            if a.is_empty() {
                return Err(ctx.throw("ValueError", format!("{func}(): Argument #1 ($value) must contain at least one element")));
            }
            a.values().cloned().collect()
        }
        (1, None) => {
            return Err(ctx.throw("TypeError", format!("{func}(): Argument #1 ($value) must be of type array, {} given", first.type_name())));
        }
        _ => args.iter().map(Value::deref).collect(),
    };
    let mut best = values[0].clone();
    for v in &values[1..] {
        // PHP keeps the first of equal values: only a strict win replaces it.
        if v.compare(&best) == Some(want) {
            best = v.clone();
        }
    }
    Ok(best)
}

fn max(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    extreme(ctx, "max", args, Ordering::Greater)
}

fn min(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    extreme(ctx, "min", args, Ordering::Less)
}

fn pi(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::float(std::f64::consts::PI))
}

fn is_nan(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::bool(float(args, 0).is_nan()))
}

fn is_finite(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::bool(float(args, 0).is_finite()))
}

fn is_infinite(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::bool(float(args, 0).is_infinite()))
}

macro_rules! float_fn {
    ($($name:ident => $f:expr),* $(,)?) => {
        $(fn $name(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
            Ok(Value::float($f(float(args, 0))))
        })*
    };
}

float_fn! {
    sqrt => f64::sqrt,
    exp => f64::exp,
    log10 => f64::log10,
    sin => f64::sin,
    cos => f64::cos,
    tan => f64::tan,
    asin => f64::asin,
    acos => f64::acos,
    atan => f64::atan,
    deg2rad => f64::to_radians,
    rad2deg => f64::to_degrees,
}

/// log($num, $base = M_E)
fn log(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let x = float(args, 0);
    if args.len() < 2 {
        return Ok(Value::float(x.ln()));
    }
    let base = float(args, 1);
    if base <= 0.0 {
        return Err(ctx.throw("ValueError", "log(): Argument #2 ($base) must be greater than 0".into()));
    }
    if base == 1.0 {
        return Err(ctx.throw("ValueError", "log(): Argument #2 ($base) must not be equal to 1".into()));
    }
    Ok(Value::float(if base == 2.0 { x.log2() } else if base == 10.0 { x.log10() } else { x.ln() / base.ln() }))
}

fn atan2(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::float(float(args, 0).atan2(float(args, 1))))
}

fn hypot(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::float(float(args, 0).hypot(float(args, 1))))
}

fn digits(n: u64, radix: u64) -> Result<Value, Throw> {
    let mut out = Vec::new();
    p8_runtime::numeric::push_digits(&mut out, n, radix, false);
    Ok(Value::str(PhpStr::new(out)))
}

fn decbin(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    digits(arg(args, 0).to_int() as u64, 2)
}

fn decoct(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    digits(arg(args, 0).to_int() as u64, 8)
}

/// _php_math_basetozval: digits of `radix` read from a string, skipping
/// any other byte (with a deprecation), becoming a float past PHP_INT_MAX.
fn from_base(ctx: &mut dyn Ctx, s: &[u8], radix: u32) -> Value {
    let mut n: i64 = 0;
    let mut f: Option<f64> = None;
    let mut invalid = false;
    let body = match (radix, s) {
        (16, [b'0', b'x' | b'X', rest @ ..]) | (8, [b'0', b'o' | b'O', rest @ ..]) | (2, [b'0', b'b' | b'B', rest @ ..]) => rest,
        _ => s,
    };
    for &c in body {
        let Some(d) = (c as char).to_digit(36).filter(|d| *d < radix) else {
            invalid = true;
            continue;
        };
        match f {
            Some(x) => f = Some(x * f64::from(radix) + f64::from(d)),
            None => match n.checked_mul(i64::from(radix)).and_then(|n| n.checked_add(i64::from(d))) {
                Some(next) => n = next,
                None => f = Some(n as f64 * f64::from(radix) + f64::from(d)),
            },
        }
    }
    if invalid {
        ctx.deprecated("Invalid characters passed for attempted conversion, these have been ignored");
    }
    f.map_or(Value::int(n), Value::float)
}

fn bindec(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = crate::str_arg(args, 0);
    Ok(from_base(ctx, s.as_bytes(), 2))
}

fn octdec(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = crate::str_arg(args, 0);
    Ok(from_base(ctx, s.as_bytes(), 8))
}

fn hexdec(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = crate::str_arg(args, 0);
    Ok(from_base(ctx, s.as_bytes(), 16))
}

fn base_convert(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let s = crate::str_arg(args, 0);
    let (from, to) = (arg(args, 1).to_int(), arg(args, 2).to_int());
    if !(2..=36).contains(&from) {
        return Err(ctx.throw("ValueError", "base_convert(): Argument #2 ($from_base) must be between 2 and 36 (inclusive)".into()));
    }
    if !(2..=36).contains(&to) {
        return Err(ctx.throw("ValueError", "base_convert(): Argument #3 ($to_base) must be between 2 and 36 (inclusive)".into()));
    }
    let lower = s.as_bytes().to_ascii_lowercase();
    let n = from_base(ctx, &lower, from as u32);
    let mut out = Vec::new();
    match n.as_float() {
        Some(f) => {
            // _php_math_zvaltobase for a float: digit by digit with fmod.
            let mut f = f.abs().floor();
            let to = to as f64;
            if f == 0.0 {
                out.push(b'0');
            }
            while f >= 1.0 {
                let d = (f % to) as usize;
                out.push(b"0123456789abcdefghijklmnopqrstuvwxyz"[d]);
                f = (f / to).floor();
            }
            out.reverse();
        }
        None => {
            let n = n.to_int() as u64;
            let mut x = n;
            loop {
                out.push(b"0123456789abcdefghijklmnopqrstuvwxyz"[(x % to as u64) as usize]);
                x /= to as u64;
                if x == 0 {
                    break;
                }
            }
            out.reverse();
        }
    }
    Ok(Value::str(PhpStr::new(out)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cases the rounding RFC and php-src's round tests use.
    #[test]
    fn round_matches_php() {
        assert_eq!(php_round(0.285, 2, HALF_UP), 0.29);
        assert_eq!(php_round(1.955, 2, HALF_UP), 1.96);
        assert_eq!(php_round(5.045, 2, HALF_UP), 5.05);
        assert_eq!(php_round(5.055, 2, HALF_UP), 5.06);
        assert_eq!(php_round(-0.5, 0, HALF_UP), -1.0);
        assert_eq!(php_round(2.5, 0, HALF_EVEN), 2.0);
        assert_eq!(php_round(3.5, 0, HALF_EVEN), 4.0);
        assert_eq!(php_round(2.5, 0, HALF_ODD), 3.0);
        assert_eq!(php_round(1234.5678, -2, HALF_UP), 1200.0);
        assert_eq!(php_round(1.1, 0, CEILING), 2.0);
        assert_eq!(php_round(-1.1, 0, FLOOR), -2.0);
    }
}
