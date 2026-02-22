use crate::core::value::{Handle, Val};
use crate::runtime::core_extension::CoreExtensionData;
use crate::vm::engine::VM;
use std::f64::consts::PI;
use std::rc::Rc;

pub fn php_abs(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("abs() expects exactly 1 parameter".into());
    }

    let val = vm.arena.get(args[0]);
    match &val.value {
        Val::Int(i) => Ok(vm.arena.alloc(Val::Int(i.abs()))),
        Val::Float(f) => Ok(vm.arena.alloc(Val::Float(f.abs()))),
        Val::String(s) => {
            // String coercion: only in weak mode
            if vm.builtin_call_strict {
                return Err("abs(): Argument #1 must be of type int|float, string given".into());
            }
            // Weak mode: try to parse as number
            let s_str = String::from_utf8_lossy(s);
            if let Ok(i) = s_str.parse::<i64>() {
                Ok(vm.arena.alloc(Val::Int(i.abs())))
            } else if let Ok(f) = s_str.parse::<f64>() {
                Ok(vm.arena.alloc(Val::Float(f.abs())))
            } else {
                Ok(vm.arena.alloc(Val::Int(0)))
            }
        }
        Val::Bool(b) => {
            if vm.builtin_call_strict {
                return Err("abs(): Argument #1 must be of type int|float, bool given".into());
            }
            Ok(vm.arena.alloc(Val::Int(if *b { 1 } else { 0 })))
        }
        Val::Null => {
            if vm.builtin_call_strict {
                return Err("abs(): Argument #1 must be of type int|float, null given".into());
            }
            Ok(vm.arena.alloc(Val::Int(0)))
        }
        _ => {
            if vm.builtin_call_strict {
                Err(format!(
                    "abs(): Argument #1 must be of type int|float, {} given",
                    val.value.type_name()
                ))
            } else {
                Ok(vm.arena.alloc(Val::Int(0)))
            }
        }
    }
}

pub fn php_max(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("max() expects at least 1 parameter".into());
    }

    if args.len() == 1 {
        // Single array argument
        let val = vm.arena.get(args[0]);
        if let Val::Array(arr_rc) = &val.value {
            if arr_rc.map.is_empty() {
                return Err("max(): Array must contain at least one element".into());
            }
            let mut max_handle = *arr_rc.map.values().next().unwrap();
            for &handle in arr_rc.map.values().skip(1) {
                if compare_values(vm, handle, max_handle) > 0 {
                    max_handle = handle;
                }
            }
            return Ok(max_handle);
        }
    }

    // Multiple arguments
    let mut max_handle = args[0];
    for &handle in &args[1..] {
        if compare_values(vm, handle, max_handle) > 0 {
            max_handle = handle;
        }
    }
    Ok(max_handle)
}

pub fn php_min(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("min() expects at least 1 parameter".into());
    }

    if args.len() == 1 {
        // Single array argument
        let val = vm.arena.get(args[0]);
        if let Val::Array(arr_rc) = &val.value {
            if arr_rc.map.is_empty() {
                return Err("min(): Array must contain at least one element".into());
            }
            let mut min_handle = *arr_rc.map.values().next().unwrap();
            for &handle in arr_rc.map.values().skip(1) {
                if compare_values(vm, handle, min_handle) < 0 {
                    min_handle = handle;
                }
            }
            return Ok(min_handle);
        }
    }

    // Multiple arguments
    let mut min_handle = args[0];
    for &handle in &args[1..] {
        if compare_values(vm, handle, min_handle) < 0 {
            min_handle = handle;
        }
    }
    Ok(min_handle)
}

fn compare_values(vm: &VM, a: Handle, b: Handle) -> i32 {
    let a_val = vm.arena.get(a);
    let b_val = vm.arena.get(b);

    match (&a_val.value, &b_val.value) {
        (Val::Int(i1), Val::Int(i2)) => i1.cmp(i2) as i32,
        (Val::Float(f1), Val::Float(f2)) => {
            if f1 < f2 {
                -1
            } else if f1 > f2 {
                1
            } else {
                0
            }
        }
        (Val::Int(i), Val::Float(f)) | (Val::Float(f), Val::Int(i)) => {
            let i_f = *i as f64;
            if i_f < *f {
                -1
            } else if i_f > *f {
                1
            } else {
                0
            }
        }
        _ => 0,
    }
}

pub fn php_pi(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(vm.arena.alloc(Val::Float(PI)))
}

pub fn php_pow(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 2 {
        return Err("pow() expects exactly 2 parameters".into());
    }

    let base = vm.arena.get(args[0]).value.to_float();
    let exponent = vm.arena.get(args[1]).value.to_float();
    let result = base.powf(exponent);
    Ok(vm.arena.alloc(Val::Float(result)))
}

pub fn php_fpow(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_pow(vm, args)
}

pub fn php_ceil(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("ceil() expects exactly 1 parameter".into());
    }
    let value = vm.arena.get(args[0]).value.to_float();
    Ok(vm.arena.alloc(Val::Float(value.ceil())))
}

pub fn php_floor(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("floor() expects exactly 1 parameter".into());
    }
    let value = vm.arena.get(args[0]).value.to_float();
    Ok(vm.arena.alloc(Val::Float(value.floor())))
}

pub fn php_round(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() || args.len() > 2 {
        return Err("round() expects 1 or 2 parameters".into());
    }
    let value = vm.arena.get(args[0]).value.to_float();
    let precision = if args.len() == 2 {
        vm.arena.get(args[1]).value.to_int()
    } else {
        0
    };
    let factor = 10_f64.powi(precision as i32);
    let result = if factor == 0.0 {
        value
    } else {
        (value * factor).round() / factor
    };
    Ok(vm.arena.alloc(Val::Float(result)))
}

pub fn php_sin(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    one_arg_float_fn(vm, args, "sin", f64::sin)
}

pub fn php_sinh(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    one_arg_float_fn(vm, args, "sinh", f64::sinh)
}

pub fn php_cos(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    one_arg_float_fn(vm, args, "cos", f64::cos)
}

pub fn php_cosh(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    one_arg_float_fn(vm, args, "cosh", f64::cosh)
}

pub fn php_tan(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    one_arg_float_fn(vm, args, "tan", f64::tan)
}

pub fn php_tanh(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    one_arg_float_fn(vm, args, "tanh", f64::tanh)
}

pub fn php_asin(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    one_arg_float_fn(vm, args, "asin", f64::asin)
}

pub fn php_asinh(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    one_arg_float_fn(vm, args, "asinh", f64::asinh)
}

pub fn php_acos(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    one_arg_float_fn(vm, args, "acos", f64::acos)
}

pub fn php_acosh(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    one_arg_float_fn(vm, args, "acosh", f64::acosh)
}

pub fn php_atan(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    one_arg_float_fn(vm, args, "atan", f64::atan)
}

pub fn php_atanh(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    one_arg_float_fn(vm, args, "atanh", f64::atanh)
}

pub fn php_atan2(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 2 {
        return Err("atan2() expects exactly 2 parameters".into());
    }
    let y = vm.arena.get(args[0]).value.to_float();
    let x = vm.arena.get(args[1]).value.to_float();
    Ok(vm.arena.alloc(Val::Float(y.atan2(x))))
}

pub fn php_deg2rad(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("deg2rad() expects exactly 1 parameter".into());
    }
    let deg = vm.arena.get(args[0]).value.to_float();
    Ok(vm.arena.alloc(Val::Float(deg * (PI / 180.0))))
}

pub fn php_rad2deg(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("rad2deg() expects exactly 1 parameter".into());
    }
    let rad = vm.arena.get(args[0]).value.to_float();
    Ok(vm.arena.alloc(Val::Float(rad * (180.0 / PI))))
}

pub fn php_exp(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    one_arg_float_fn(vm, args, "exp", f64::exp)
}

pub fn php_expm1(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    one_arg_float_fn(vm, args, "expm1", f64::exp_m1)
}

pub fn php_log(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() || args.len() > 2 {
        return Err("log() expects 1 or 2 parameters".into());
    }
    let value = vm.arena.get(args[0]).value.to_float();
    let result = if args.len() == 2 {
        let base = vm.arena.get(args[1]).value.to_float();
        value.log(base)
    } else {
        value.ln()
    };
    Ok(vm.arena.alloc(Val::Float(result)))
}

pub fn php_log10(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    one_arg_float_fn(vm, args, "log10", f64::log10)
}

pub fn php_log1p(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    one_arg_float_fn(vm, args, "log1p", f64::ln_1p)
}

pub fn php_sqrt(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    one_arg_float_fn(vm, args, "sqrt", f64::sqrt)
}

pub fn php_fdiv(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 2 {
        return Err("fdiv() expects exactly 2 parameters".into());
    }
    let numerator = vm.arena.get(args[0]).value.to_float();
    let denominator = vm.arena.get(args[1]).value.to_float();
    let result = if denominator == 0.0 {
        if numerator == 0.0 {
            f64::NAN
        } else if numerator.is_sign_negative() {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        }
    } else {
        numerator / denominator
    };
    Ok(vm.arena.alloc(Val::Float(result)))
}

pub fn php_intdiv(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 2 {
        return Err("intdiv() expects exactly 2 parameters".into());
    }
    let numerator = vm.arena.get(args[0]).value.to_int();
    let denominator = vm.arena.get(args[1]).value.to_int();
    if denominator == 0 {
        return Err("intdiv(): Division by zero".into());
    }
    Ok(vm.arena.alloc(Val::Int(numerator / denominator)))
}

pub fn php_fmod(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 2 {
        return Err("fmod() expects exactly 2 parameters".into());
    }
    let x = vm.arena.get(args[0]).value.to_float();
    let y = vm.arena.get(args[1]).value.to_float();
    Ok(vm.arena.alloc(Val::Float(x % y)))
}

pub fn php_hypot(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 2 {
        return Err("hypot() expects exactly 2 parameters".into());
    }
    let x = vm.arena.get(args[0]).value.to_float();
    let y = vm.arena.get(args[1]).value.to_float();
    Ok(vm.arena.alloc(Val::Float(x.hypot(y))))
}

pub fn php_is_finite(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("is_finite() expects exactly 1 parameter".into());
    }
    let value = vm.arena.get(args[0]).value.to_float();
    Ok(vm.arena.alloc(Val::Bool(value.is_finite())))
}

pub fn php_is_infinite(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("is_infinite() expects exactly 1 parameter".into());
    }
    let value = vm.arena.get(args[0]).value.to_float();
    Ok(vm.arena.alloc(Val::Bool(value.is_infinite())))
}

pub fn php_is_nan(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("is_nan() expects exactly 1 parameter".into());
    }
    let value = vm.arena.get(args[0]).value.to_float();
    Ok(vm.arena.alloc(Val::Bool(value.is_nan())))
}

pub fn php_base_convert(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 3 {
        return Err("base_convert() expects exactly 3 parameters".into());
    }
    let number = vm.arena.get(args[0]).value.to_php_string_bytes();
    let from_base = vm.arena.get(args[1]).value.to_int();
    let to_base = vm.arena.get(args[2]).value.to_int();
    let result = match parse_base(&number, from_base) {
        Some(value) => format_base(value, to_base),
        None => "0".to_string(),
    };
    Ok(vm.arena.alloc(Val::String(Rc::new(result.into_bytes()))))
}

pub fn php_decbin(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    convert_from_int(vm, args, 2, "decbin")
}

pub fn php_dechex(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    convert_from_int(vm, args, 16, "dechex")
}

pub fn php_decoct(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    convert_from_int(vm, args, 8, "decoct")
}

pub fn php_bindec(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    convert_to_int(vm, args, 2, "bindec")
}

pub fn php_hexdec(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    convert_to_int(vm, args, 16, "hexdec")
}

pub fn php_octdec(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    convert_to_int(vm, args, 8, "octdec")
}

fn one_arg_float_fn(
    vm: &mut VM,
    args: &[Handle],
    name: &str,
    func: fn(f64) -> f64,
) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err(format!("{name}() expects exactly 1 parameter"));
    }
    let value = vm.arena.get(args[0]).value.to_float();
    Ok(vm.arena.alloc(Val::Float(func(value))))
}

fn parse_base(input: &[u8], base: i64) -> Option<u128> {
    if base < 2 || base > 36 {
        return None;
    }
    let s = String::from_utf8_lossy(input).trim().to_lowercase();
    u128::from_str_radix(&s, base as u32).ok()
}

fn format_base(value: u128, base: i64) -> String {
    if base < 2 || base > 36 {
        return "0".to_string();
    }
    let mut n = value;
    if n == 0 {
        return "0".to_string();
    }
    let mut digits = Vec::new();
    while n > 0 {
        let rem = (n % base as u128) as u8;
        let ch = if rem < 10 {
            (b'0' + rem) as char
        } else {
            (b'a' + (rem - 10)) as char
        };
        digits.push(ch);
        n /= base as u128;
    }
    digits.iter().rev().collect()
}

fn convert_from_int(vm: &mut VM, args: &[Handle], base: u32, name: &str) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err(format!("{name}() expects exactly 1 parameter"));
    }
    let mut value = vm.arena.get(args[0]).value.to_int();
    let negative = value < 0;
    if negative {
        value = -value;
    }
    let mut s = format_base(value as u128, base as i64);
    if negative {
        s = format!("-{s}");
    }
    Ok(vm.arena.alloc(Val::String(Rc::new(s.into_bytes()))))
}

fn convert_to_int(vm: &mut VM, args: &[Handle], base: u32, name: &str) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err(format!("{name}() expects exactly 1 parameter"));
    }
    let s = vm.arena.get(args[0]).value.to_php_string_bytes();
    let value = parse_base(&s, base as i64).unwrap_or(0);
    Ok(vm.arena.alloc(Val::Int(value as i64)))
}

pub fn php_srand(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() > 1 {
        return Err("srand() expects at most 1 parameter".into());
    }

    let seed = if args.is_empty() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        now as i64
    } else {
        vm.arena.get(args[0]).value.to_int()
    };

    let core = vm
        .context
        .get_or_init_extension_data(CoreExtensionData::default);
    core.rng_seed(seed as u32);

    Ok(vm.arena.alloc(Val::Null))
}

pub fn php_rand(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() > 2 {
        return Err("rand() expects at most 2 parameters".into());
    }

    let core = vm
        .context
        .get_or_init_extension_data(CoreExtensionData::default);

    let rand_max = 2147483647i64;
    if args.is_empty() {
        let value = core.rng_next_u31() as i64;
        return Ok(vm.arena.alloc(Val::Int(value)));
    }

    let mut min = vm.arena.get(args[0]).value.to_int();
    let mut max = if args.len() == 2 {
        vm.arena.get(args[1]).value.to_int()
    } else {
        rand_max
    };

    if min > max {
        std::mem::swap(&mut min, &mut max);
    }

    let range = (max - min + 1) as u64;
    let value = if range == 0 {
        min
    } else {
        let roll = core.rng_next_u32() as u64;
        min + (roll % range) as i64
    };

    Ok(vm.arena.alloc(Val::Int(value)))
}

pub fn php_getrandmax(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(vm.arena.alloc(Val::Int(2147483647)))
}
