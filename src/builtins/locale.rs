//! setlocale(), localeconv(), nl_langinfo() and strcoll(), in Rust.
//!
//! PHP 8 starts every request in the "C" locale (LC_CTYPE as "C.UTF-8")
//! and code rarely leaves it, so this models the C and C.UTF-8 locales
//! exactly, as glibc reports them, instead of calling the C library (which
//! needs `unsafe` and shares one process-wide locale between threads).
//! Any other locale name is "not installed": setlocale() returns false, the
//! same answer PHP gives on a system without that locale.
//!
//! Constant values are glibc's, so scripts behave the same on every host as
//! on the Linux reference PHP.
//! Reference: $PHP_SRC_PATH/ext/standard/string.c - setlocale, localeconv, nl_langinfo

use crate::core::value::{ArrayData, ArrayKey, Handle, Val};
use crate::vm::engine::{ErrorLevel, VM};
use std::rc::Rc;

pub const LC_CTYPE: i64 = 0;
pub const LC_NUMERIC: i64 = 1;
pub const LC_TIME: i64 = 2;
pub const LC_COLLATE: i64 = 3;
pub const LC_MONETARY: i64 = 4;
pub const LC_MESSAGES: i64 = 5;
pub const LC_ALL: i64 = 6;

/// The categories PHP can set, in glibc's order.
const CATEGORIES: [(i64, &str); 6] = [
    (LC_CTYPE, "LC_CTYPE"),
    (LC_NUMERIC, "LC_NUMERIC"),
    (LC_TIME, "LC_TIME"),
    (LC_COLLATE, "LC_COLLATE"),
    (LC_MONETARY, "LC_MONETARY"),
    (LC_MESSAGES, "LC_MESSAGES"),
];

/// glibc categories PHP cannot set; they stay "C" and appear in LC_ALL queries.
const OTHER_CATEGORIES: [&str; 6] = [
    "LC_PAPER",
    "LC_NAME",
    "LC_ADDRESS",
    "LC_TELEPHONE",
    "LC_MEASUREMENT",
    "LC_IDENTIFICATION",
];

/// The locale of each settable category for one request.
#[derive(Debug, Clone)]
pub struct LocaleState {
    names: [&'static str; 6],
}

impl Default for LocaleState {
    /// PHP 8's startup locale.
    fn default() -> Self {
        Self {
            names: ["C.UTF-8", "C", "C", "C", "C", "C"],
        }
    }
}

/// The canonical name of a locale this runtime provides.
fn known_locale(name: &[u8]) -> Option<&'static str> {
    match name {
        b"C" | b"POSIX" => Some("C"),
        b"C.UTF-8" | b"C.utf8" | b"C.UTF8" | b"C.utf-8" => Some("C.UTF-8"),
        _ => None,
    }
}

/// The locale "" stands for: the environment's LC_ALL, the category's own
/// variable, then LANG; "C" when none is set.
fn environment_locale(category: &str) -> Option<&'static str> {
    for var in ["LC_ALL", category, "LANG"] {
        if let Ok(value) = std::env::var(var)
            && !value.is_empty()
        {
            return known_locale(value.as_bytes());
        }
    }
    Some("C")
}

impl LocaleState {
    fn query(&self, category: i64) -> Option<String> {
        if category == LC_ALL {
            if self.names.iter().all(|n| *n == self.names[0]) {
                return Some(self.names[0].to_string());
            }
            let mut parts: Vec<String> = CATEGORIES
                .iter()
                .zip(self.names)
                .map(|((_, cat), name)| format!("{cat}={name}"))
                .collect();
            parts.extend(OTHER_CATEGORIES.iter().map(|cat| format!("{cat}=C")));
            return Some(parts.join(";"));
        }
        let idx = CATEGORIES.iter().position(|(c, _)| *c == category)?;
        Some(self.names[idx].to_string())
    }

    fn set(&mut self, category: i64, requested: &[u8]) -> Option<&'static str> {
        let resolve = |cat: &str| {
            if requested.is_empty() {
                environment_locale(cat)
            } else {
                known_locale(requested)
            }
        };
        if category == LC_ALL {
            let name = resolve("LC_ALL")?;
            self.names = [name; 6];
            return Some(name);
        }
        let idx = CATEGORIES.iter().position(|(c, _)| *c == category)?;
        let name = resolve(CATEGORIES[idx].1)?;
        self.names[idx] = name;
        Some(name)
    }

    fn ctype_is_utf8(&self) -> bool {
        self.names[0] == "C.UTF-8"
    }
}

fn string_handle(vm: &mut VM, s: &[u8]) -> Handle {
    vm.arena.alloc(Val::String(Rc::new(s.to_vec())))
}

/// setlocale(int $category, string|array|null $locales, string ...$rest): string|false
pub fn php_setlocale(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("setlocale() expects at least 2 parameters".into());
    }
    let category = vm.check_builtin_param_int(args[0], 1, "setlocale")?;
    if category != LC_ALL && !CATEGORIES.iter().any(|(c, _)| *c == category) {
        let msg = "setlocale(): Argument #1 ($category) must be one of LC_ALL, LC_COLLATE, LC_CTYPE, LC_MONETARY, LC_NUMERIC, LC_TIME, or LC_MESSAGES";
        return Err(vm.throw_from_native(b"ValueError", msg));
    }

    let mut candidates: Vec<Option<Vec<u8>>> = Vec::new();
    for &arg in &args[1..] {
        match vm.arena.get(arg).value.clone() {
            Val::Null => candidates.push(None),
            Val::Array(arr) => {
                for &h in arr.map.values() {
                    candidates.push(Some(vm.arena.get(h).value.to_php_string_bytes()));
                }
            }
            other => candidates.push(Some(other.to_php_string_bytes())),
        }
    }

    for candidate in candidates {
        // null and "0" ask for the current locale without changing it.
        let result = match candidate.as_deref() {
            None | Some(b"0") => vm.context.locale.query(category),
            Some(name) => vm.context.locale.set(category, name).map(str::to_string),
        };
        if let Some(name) = result {
            return Ok(string_handle(vm, name.as_bytes()));
        }
    }
    Ok(vm.arena.alloc(Val::Bool(false)))
}

/// localeconv(): the C locale's numeric and monetary formatting.
pub fn php_localeconv(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if !args.is_empty() {
        return Err("localeconv() expects exactly 0 parameters".into());
    }
    let mut data = ArrayData::new();
    let strings: [(&str, &[u8]); 8] = [
        ("decimal_point", b"."),
        ("thousands_sep", b""),
        ("int_curr_symbol", b""),
        ("currency_symbol", b""),
        ("mon_decimal_point", b""),
        ("mon_thousands_sep", b""),
        ("positive_sign", b""),
        ("negative_sign", b""),
    ];
    for (key, value) in strings {
        let h = string_handle(vm, value);
        data.insert(ArrayKey::Str(Rc::new(key.as_bytes().to_vec())), h);
    }
    // CHAR_MAX: "not available in this locale"
    for key in [
        "int_frac_digits",
        "frac_digits",
        "p_cs_precedes",
        "p_sep_by_space",
        "n_cs_precedes",
        "n_sep_by_space",
        "p_sign_posn",
        "n_sign_posn",
    ] {
        let h = vm.arena.alloc(Val::Int(127));
        data.insert(ArrayKey::Str(Rc::new(key.as_bytes().to_vec())), h);
    }
    for key in ["grouping", "mon_grouping"] {
        let h = vm.arena.alloc(Val::Array(Rc::new(ArrayData::new())));
        data.insert(ArrayKey::Str(Rc::new(key.as_bytes().to_vec())), h);
    }
    Ok(vm.arena.alloc(Val::Array(Rc::new(data))))
}

/// glibc's nl_item values: (category << 16) | index.
pub mod item {
    pub const CODESET: i64 = 14;
    pub const RADIXCHAR: i64 = 0x10000;
    pub const THOUSEP: i64 = 0x10001;
    pub const ABDAY_1: i64 = 0x20000;
    pub const DAY_1: i64 = 0x20007;
    pub const ABMON_1: i64 = 0x2000E;
    pub const MON_1: i64 = 0x2001A;
    pub const AM_STR: i64 = 0x20026;
    pub const PM_STR: i64 = 0x20027;
    pub const D_T_FMT: i64 = 0x20028;
    pub const D_FMT: i64 = 0x20029;
    pub const T_FMT: i64 = 0x2002A;
    pub const T_FMT_AMPM: i64 = 0x2002B;
    pub const ERA: i64 = 0x2002C;
    pub const ERA_D_FMT: i64 = 0x2002E;
    pub const ALT_DIGITS: i64 = 0x2002F;
    pub const ERA_D_T_FMT: i64 = 0x20030;
    pub const ERA_T_FMT: i64 = 0x20031;
    pub const CRNCYSTR: i64 = 0x4000F;
    pub const YESEXPR: i64 = 0x50000;
    pub const NOEXPR: i64 = 0x50001;
}

/// The nl_langinfo constants PHP defines, as (name, value).
pub fn langinfo_constants() -> Vec<(String, i64)> {
    use item::*;
    let mut out: Vec<(String, i64)> = [
        ("CODESET", CODESET),
        ("RADIXCHAR", RADIXCHAR),
        ("THOUSEP", THOUSEP),
        ("AM_STR", AM_STR),
        ("PM_STR", PM_STR),
        ("D_T_FMT", D_T_FMT),
        ("D_FMT", D_FMT),
        ("T_FMT", T_FMT),
        ("T_FMT_AMPM", T_FMT_AMPM),
        ("ERA", ERA),
        ("ERA_D_FMT", ERA_D_FMT),
        ("ALT_DIGITS", ALT_DIGITS),
        ("ERA_D_T_FMT", ERA_D_T_FMT),
        ("ERA_T_FMT", ERA_T_FMT),
        ("CRNCYSTR", CRNCYSTR),
        ("YESEXPR", YESEXPR),
        ("NOEXPR", NOEXPR),
    ]
    .into_iter()
    .map(|(n, v)| (n.to_string(), v))
    .collect();
    for i in 0..7 {
        out.push((format!("ABDAY_{}", i + 1), ABDAY_1 + i));
        out.push((format!("DAY_{}", i + 1), DAY_1 + i));
    }
    for i in 0..12 {
        out.push((format!("ABMON_{}", i + 1), ABMON_1 + i));
        out.push((format!("MON_{}", i + 1), MON_1 + i));
    }
    out
}

const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October",
    "November", "December",
];

/// nl_langinfo(int $item): string|false
pub fn php_nl_langinfo(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    use item::*;
    if args.len() != 1 {
        return Err("nl_langinfo() expects exactly 1 parameter".into());
    }
    let item = vm.check_builtin_param_int(args[0], 1, "nl_langinfo")?;
    let value: Option<String> = match item {
        CODESET => Some(if vm.context.locale.ctype_is_utf8() { "UTF-8" } else { "ANSI_X3.4-1968" }.into()),
        RADIXCHAR => Some(".".into()),
        THOUSEP => Some(String::new()),
        AM_STR => Some("AM".into()),
        PM_STR => Some("PM".into()),
        D_T_FMT => Some("%a %b %e %H:%M:%S %Y".into()),
        D_FMT => Some("%m/%d/%y".into()),
        T_FMT => Some("%H:%M:%S".into()),
        T_FMT_AMPM => Some("%I:%M:%S %p".into()),
        ERA | ERA_D_FMT | ALT_DIGITS | ERA_D_T_FMT | ERA_T_FMT => Some(String::new()),
        CRNCYSTR => Some("-".into()),
        YESEXPR => Some("^[yY]".into()),
        NOEXPR => Some("^[nN]".into()),
        i if (ABDAY_1..ABDAY_1 + 7).contains(&i) => Some(DAYS[(i - ABDAY_1) as usize][..3].into()),
        i if (DAY_1..DAY_1 + 7).contains(&i) => Some(DAYS[(i - DAY_1) as usize].into()),
        i if (ABMON_1..ABMON_1 + 12).contains(&i) => Some(MONTHS[(i - ABMON_1) as usize][..3].into()),
        i if (MON_1..MON_1 + 12).contains(&i) => Some(MONTHS[(i - MON_1) as usize].into()),
        _ => None,
    };
    match value {
        Some(s) => Ok(string_handle(vm, s.as_bytes())),
        None => {
            vm.report_error(
                ErrorLevel::Warning,
                &format!("nl_langinfo(): Item '{}' is not valid", item),
            );
            Ok(vm.arena.alloc(Val::Bool(false)))
        }
    }
}

/// strcoll(string $string1, string $string2): int
///
/// The C locales collate by byte; glibc returns the difference of the
/// first differing bytes (a string's end counts as 0).
pub fn php_strcoll(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 2 {
        return Err("strcoll() expects exactly 2 parameters".into());
    }
    let a = vm.check_builtin_param_string(args[0], 1, "strcoll")?;
    let b = vm.check_builtin_param_string(args[1], 2, "strcoll")?;
    Ok(vm.arena.alloc(Val::Int(strcoll_c(&a, &b))))
}

fn strcoll_c(a: &[u8], b: &[u8]) -> i64 {
    // Like strcmp on C strings: the comparison stops at the first NUL.
    let end = |s: &[u8]| s.iter().position(|c| *c == 0).unwrap_or(s.len());
    let (a, b) = (&a[..end(a)], &b[..end(b)]);
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return i64::from(x) - i64::from(y);
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected values are PHP 8.4's on Linux (glibc) in the C locale.
    #[test]
    fn strcoll_matches_glibc_c_locale() {
        assert_eq!(strcoll_c(b"a", b"b"), -1);
        assert_eq!(strcoll_c(b"b", b"a"), 1);
        assert_eq!(strcoll_c(b"abc", b"abc"), 0);
        assert_eq!(strcoll_c(b"a", b"ab"), -98);
        assert_eq!(strcoll_c(b"B", b"a"), -31);
    }

    #[test]
    fn lc_all_query_lists_categories_when_mixed() {
        let state = LocaleState::default();
        assert_eq!(
            state.query(LC_ALL).unwrap(),
            "LC_CTYPE=C.UTF-8;LC_NUMERIC=C;LC_TIME=C;LC_COLLATE=C;LC_MONETARY=C;LC_MESSAGES=C;LC_PAPER=C;LC_NAME=C;LC_ADDRESS=C;LC_TELEPHONE=C;LC_MEASUREMENT=C;LC_IDENTIFICATION=C"
        );
        let mut state = state;
        assert_eq!(state.set(LC_ALL, b"POSIX"), Some("C"));
        assert_eq!(state.query(LC_ALL).unwrap(), "C");
        assert_eq!(state.set(LC_ALL, b"xx_XX"), None);
    }
}
