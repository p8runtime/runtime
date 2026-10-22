//! ext/ctype: character class checks in the C locale (ctype.c). A string
//! matches when every byte is in the class (an empty string never does);
//! an int is read as a character code, with a deprecation, as PHP 8.1+
//! does; anything else is false.

use crate::core::value::{Handle, Val};
use crate::vm::engine::{ErrorLevel, VM};

/// One ctype function: the class, and the answer for an int outside the
/// byte range (allow_digits for >= 256, allow_minus for < -128).
fn ctype(
    vm: &mut VM,
    args: &[Handle],
    name: &str,
    class: fn(u8) -> bool,
    allow_digits: bool,
    allow_minus: bool,
) -> Result<Handle, String> {
    let Some(&arg) = args.first() else {
        return Err(vm.throw_from_native(
            b"ArgumentCountError",
            &format!("{}() expects exactly 1 argument, 0 given", name),
        ));
    };
    let value = vm.arena.get(arg).value.clone();
    let result = match value {
        Val::String(s) => !s.is_empty() && s.iter().all(|&b| class(b)),
        other => {
            vm.report_error(
                ErrorLevel::Deprecated,
                &format!(
                    "{}(): Argument of type {} will be interpreted as string in the future",
                    name,
                    other.type_name()
                ),
            );
            match other {
                Val::Int(i @ 0..=255) => class(i as u8),
                Val::Int(i @ -128..=-1) => class((i + 256) as u8),
                Val::Int(i) if i >= 0 => allow_digits,
                Val::Int(_) => allow_minus,
                _ => false,
            }
        }
    };
    Ok(vm.arena.alloc(Val::Bool(result)))
}

macro_rules! ctype_fn {
    ($fn_name:ident, $php:literal, $class:expr, $digits:literal, $minus:literal) => {
        #[doc = concat!($php, "(mixed $text): bool")]
        pub fn $fn_name(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
            ctype(vm, args, $php, $class, $digits, $minus)
        }
    };
}

ctype_fn!(php_ctype_alnum, "ctype_alnum", |b: u8| b.is_ascii_alphanumeric(), true, false);
ctype_fn!(php_ctype_alpha, "ctype_alpha", |b: u8| b.is_ascii_alphabetic(), false, false);
ctype_fn!(php_ctype_cntrl, "ctype_cntrl", |b: u8| b.is_ascii_control(), false, false);
ctype_fn!(php_ctype_digit, "ctype_digit", |b: u8| b.is_ascii_digit(), true, false);
ctype_fn!(php_ctype_lower, "ctype_lower", |b: u8| b.is_ascii_lowercase(), false, false);
ctype_fn!(php_ctype_graph, "ctype_graph", |b: u8| b.is_ascii_graphic(), true, true);
ctype_fn!(php_ctype_print, "ctype_print", |b: u8| b.is_ascii_graphic() || b == b' ', true, true);
ctype_fn!(php_ctype_punct, "ctype_punct", |b: u8| b.is_ascii_punctuation(), false, false);
ctype_fn!(php_ctype_space, "ctype_space", |b: u8| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c), false, false);
ctype_fn!(php_ctype_upper, "ctype_upper", |b: u8| b.is_ascii_uppercase(), false, false);
ctype_fn!(php_ctype_xdigit, "ctype_xdigit", |b: u8| b.is_ascii_hexdigit(), true, false);
