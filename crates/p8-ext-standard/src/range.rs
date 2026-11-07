//! range(): ints, floats and single-byte characters, with PHP 8.3's
//! argument rules (ext/standard/array.c, PHP_FUNCTION(range)).

use p8_runtime::array::PhpArray;
use p8_runtime::ext::{Ctx, Throw};
use p8_runtime::numeric::{numeric_string, Number};
use p8_runtime::string::PhpStr;
use p8_runtime::value::{Kind as ValueKind, Value};

/// HT_MAX_SIZE on 64-bit: the most entries an array may have.
const MAX_SIZE: u64 = 0x4000_0000;

/// What an argument can be read as (php_range_process_input).
#[derive(Clone, Copy, PartialEq, PartialOrd)]
enum Kind {
    Long,
    Double,
    /// A string that is no number: its first byte.
    Str,
    /// A one-byte numeric string ("5"): a digit or a number.
    Digit,
}

/// One argument: its kind, as an int and as a float.
struct Input {
    kind: Kind,
    long: i64,
    double: f64,
}

fn value_error(ctx: &dyn Ctx, message: String) -> Throw {
    ctx.throw("ValueError", format!("range(): {message}"))
}

fn finite(ctx: &dyn Ctx, n: usize, d: f64) -> Result<f64, Throw> {
    if d.is_infinite() {
        return Err(value_error(ctx, format!("Argument #{n} (${}) must be a finite number, INF provided", name(n))));
    }
    if d.is_nan() {
        return Err(value_error(ctx, format!("Argument #{n} (${}) must be a finite number, NAN provided", name(n))));
    }
    Ok(d)
}

fn name(n: usize) -> &'static str {
    match n {
        1 => "start",
        2 => "end",
        _ => "step",
    }
}

fn input(ctx: &mut dyn Ctx, n: usize, v: &Value) -> Result<Input, Throw> {
    Ok(match v.kind() {
        ValueKind::Int => {
            let i = v.as_int().expect("an int");
            Input { kind: Kind::Long, long: i, double: i as f64 }
        }
        ValueKind::Float => Input { kind: Kind::Double, long: 0, double: finite(ctx, n, v.as_float().expect("a float"))? },
        ValueKind::Str => {
            let bytes = v.as_str().expect("a string").as_bytes();
            if bytes.is_empty() {
                ctx.warn(&format!("range(): Argument #{n} (${}) must not be empty, casted to 0", name(n)));
                return Ok(Input { kind: Kind::Long, long: 0, double: 0.0 });
            }
            match numeric_string(bytes) {
                Some(Number::Float(d)) => Input { kind: Kind::Double, long: 0, double: finite(ctx, n, d)? },
                Some(Number::Int(i)) => Input { kind: if bytes.len() == 1 { Kind::Digit } else { Kind::Long }, long: i, double: i as f64 },
                None => {
                    if bytes.len() != 1 {
                        ctx.warn(&format!("range(): Argument #{n} (${}) must be a single byte, subsequent bytes are ignored", name(n)));
                    }
                    Input { kind: Kind::Str, long: 0, double: 0.0 }
                }
            }
        }
        _ => Input { kind: Kind::Long, long: v.to_int(), double: v.to_int() as f64 },
    })
}

fn list(values: Vec<Value>) -> Result<Value, Throw> {
    Ok(Value::array(PhpArray::from_list(values)))
}

/// range($start, $end, $step = 1)
pub(crate) fn range(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let get = |i: usize| args.get(i).map(Value::deref);
    let (user_start, user_end, user_step) = (get(0).unwrap_or(Value::NULL), get(1).unwrap_or(Value::NULL), get(2));
    let mut step_double_is = false;
    let mut negative = false;
    let mut step_double = 1.0;
    let mut step: i64 = 1;
    if let Some(s) = &user_step {
        if let Some(d) = s.as_float() {
            step_double = finite(ctx, 3, d)?;
            if step_double < 0.0 {
                negative = true;
                step_double = -step_double;
            }
            step = p8_runtime::number::dval_to_lval(step_double);
            // zend_is_long_compatible: the float is that int exactly.
            if step as f64 != step_double {
                step_double_is = true;
            }
        } else {
            step = s.to_int();
            if step < 0 {
                if step == i64::MIN {
                    return Err(value_error(ctx, format!("Argument #3 ($step) must be greater than {}", i64::MIN)));
                }
                negative = true;
                step = -step;
            }
            step_double = step as f64;
        }
        if step_double == 0.0 {
            return Err(value_error(ctx, "Argument #3 ($step) cannot be 0".into()));
        }
    }
    let mut start = input(ctx, 1, &user_start)?;
    let mut end = input(ctx, 2, &user_end)?;

    // Strings (not numbers): an array of characters.
    if start.kind >= Kind::Str || end.kind >= Kind::Str {
        let numeric = if start.kind < Kind::Str || end.kind < Kind::Str {
            if start.kind < Kind::Str {
                if end.kind != Kind::Digit {
                    ctx.warn("range(): Argument #1 ($start) must be a single byte string if argument #2 ($end) is a single byte string, argument #2 ($end) converted to 0");
                }
                end.kind = Kind::Long;
            } else {
                if start.kind != Kind::Digit {
                    ctx.warn("range(): Argument #2 ($end) must be a single byte string if argument #1 ($start) is a single byte string, argument #1 ($start) converted to 0");
                }
                start.kind = Kind::Long;
            }
            true
        } else if step_double_is {
            if start.kind == Kind::Str || end.kind == Kind::Str {
                ctx.warn("range(): Argument #3 ($step) must be of type int when generating an array of characters, inputs converted to 0");
            }
            start.kind = Kind::Long;
            end.kind = Kind::Long;
            true
        } else {
            false
        };
        if !numeric {
            let byte = |v: &Value| v.to_php_string().and_then(|s| s.as_bytes().first().copied()).unwrap_or(0) as i64;
            let (low, high) = (byte(&user_start), byte(&user_end));
            let step = step as u64;
            let chars = |from: i64, to: i64, down: bool| -> Vec<Value> {
                let mut out = Vec::new();
                let mut c = from;
                loop {
                    out.push(Value::str(PhpStr::byte(c as u8)));
                    let next = if down { c - step as i64 } else { c + step as i64 };
                    if (down && (next < to || next < 0)) || (!down && (next > to || next > 255)) {
                        break;
                    }
                    c = next;
                }
                out
            };
            return if low > high {
                if ((low - high) as u64) < step {
                    return Err(boundary(ctx));
                }
                list(chars(low, high, true))
            } else if high > low {
                if negative {
                    return Err(negative_step(ctx));
                }
                if ((high - low) as u64) < step {
                    return Err(boundary(ctx));
                }
                list(chars(low, high, false))
            } else {
                list(vec![Value::str(PhpStr::byte(low as u8))])
            };
        }
    }

    // Numbers.
    if start.kind == Kind::Double || end.kind == Kind::Double || step_double_is {
        let (s, e) = (start.double, end.double);
        let floats = |from: f64, to: f64, down: bool| -> Result<Vec<Value>, Throw> {
            // RANGE_CHECK_DOUBLE_INIT_ARRAY(larger, smaller, step).
            let (big, small) = if down { (from, to) } else { (to, from) };
            let calc = (big - small) / step_double + 1.0;
            if calc >= MAX_SIZE as f64 {
                return Err(too_large(ctx, format!("start={small:.1} end={big:.1} step={step_double:.1}")));
            }
            // _php_math_round(.., 0, PHP_ROUND_HALF_UP)
            let size = calc.round() as u32;
            let mut out = Vec::with_capacity(size as usize);
            let mut i = 0u32;
            let mut element = from;
            while i < size && if down { element >= to } else { element <= to } {
                out.push(Value::float(element));
                i += 1;
                element = if down { from - (i as f64 * step_double) } else { from + (i as f64 * step_double) };
            }
            Ok(out)
        };
        return if s > e {
            if s - e < step_double {
                return Err(boundary(ctx));
            }
            list(floats(s, e, true)?)
        } else if e > s {
            if negative {
                return Err(negative_step(ctx));
            }
            if e - s < step_double {
                return Err(boundary(ctx));
            }
            list(floats(s, e, false)?)
        } else {
            list(vec![Value::float(s)])
        };
    }
    let (s, e) = (start.long, end.long);
    let ustep = step as u64;
    let ints = |from: i64, to: i64, down: bool| -> Result<Vec<Value>, Throw> {
        let span = if down { (from as u64).wrapping_sub(to as u64) } else { (to as u64).wrapping_sub(from as u64) };
        let calc = span / ustep;
        if calc >= MAX_SIZE - 1 {
            let (lo, hi) = if down { (to, from) } else { (from, to) };
            return Err(too_large(ctx, format!("start={lo} end={hi} step={ustep}")));
        }
        let size = calc + 1;
        Ok((0..size)
            .map(|i| {
                let d = i.wrapping_mul(ustep);
                Value::int(if down { (from as u64).wrapping_sub(d) as i64 } else { (from as u64).wrapping_add(d) as i64 })
            })
            .collect())
    };
    if s > e {
        if (s as u64).wrapping_sub(e as u64) < ustep {
            return Err(boundary(ctx));
        }
        list(ints(s, e, true)?)
    } else if e > s {
        if negative {
            return Err(negative_step(ctx));
        }
        if (e as u64).wrapping_sub(s as u64) < ustep {
            return Err(boundary(ctx));
        }
        list(ints(s, e, false)?)
    } else {
        list(vec![Value::int(s)])
    }
}

/// zend_value_error (not an argument's): no function name in front.
fn too_large(ctx: &dyn Ctx, bounds: String) -> Throw {
    ctx.throw("ValueError", format!("The supplied range exceeds the maximum array size: {bounds}"))
}

fn negative_step(ctx: &dyn Ctx) -> Throw {
    value_error(ctx, "Argument #3 ($step) must be greater than 0 for increasing ranges".into())
}

fn boundary(ctx: &dyn Ctx) -> Throw {
    value_error(ctx, "Argument #3 ($step) must be less than the range spanned by argument #1 ($start) and argument #2 ($end)".into())
}
