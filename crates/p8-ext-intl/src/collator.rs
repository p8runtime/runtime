//! Collator and collator_* (collator/*.c) over icu_collator: comparison
//! with the strength, alternate handling, case-first, case-level and
//! numeric attributes, PHP's three sort flavors (regular, string, numeric)
//! sorted the way zend_sort does, and sort keys.
//!
//! Sort keys are ICU4X's: they order like ICU's but their bytes differ.
//! FRENCH_COLLATION (backwards secondary) follows the locale's data and
//! cannot be switched; HIRAGANA_QUATERNARY_MODE is ignored by ICU too.

use crate::{IntlError, R, U_ILLEGAL_ARGUMENT_ERROR, U_ZERO_ERROR, arg, b, default_locale, s, str_arg};
use icu_collator::options::{AlternateHandling, CaseLevel, CollatorOptions, Strength};
use icu_collator::preferences::{CollationCaseFirst, CollationNumericOrdering};
use icu_collator::{CollatorBorrowed, CollatorPreferences};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, ClassHooks, ClassSpec, Ctx, Registry, Throw};
use p8_runtime::object::Object;
use p8_runtime::value::Value;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::rc::Rc;

const DEFAULT_VALUE: i64 = -1;
const PRIMARY: i64 = 0;
const SECONDARY: i64 = 1;
const TERTIARY: i64 = 2;
const QUATERNARY: i64 = 3;
const IDENTICAL: i64 = 15;
const OFF: i64 = 16;
const ON: i64 = 17;
const SHIFTED: i64 = 20;
const NON_IGNORABLE: i64 = 21;
const LOWER_FIRST: i64 = 24;
const UPPER_FIRST: i64 = 25;

const A_FRENCH: i64 = 0;
const A_ALTERNATE: i64 = 1;
const A_CASE_FIRST: i64 = 2;
const A_CASE_LEVEL: i64 = 3;
const A_NORMALIZATION: i64 = 4;
const A_STRENGTH: i64 = 5;
const A_HIRAGANA: i64 = 6;
const A_NUMERIC: i64 = 7;

const SORT_REGULAR: i64 = 0;
const SORT_STRING: i64 = 1;
const SORT_NUMERIC: i64 = 2;

#[derive(Clone, Debug)]
pub struct Coll {
    pub locale: String,
    /// The attribute values, by UColAttribute.
    pub attrs: [i64; 8],
    pub french_default: bool,
}

impl Coll {
    fn new(locale: &str) -> Coll {
        let mut attrs = [OFF, NON_IGNORABLE, OFF, OFF, OFF, TERTIARY, OFF, OFF];
        let p = crate::uloc::parts(locale);
        if p.lang == "da" || p.lang == "mt" {
            attrs[A_CASE_FIRST as usize] = UPPER_FIRST;
        }
        if p.lang == "th" {
            attrs[A_ALTERNATE as usize] = SHIFTED;
        }
        let french_default = p.lang == "fr" && p.region == "CA";
        if french_default {
            attrs[A_FRENCH as usize] = ON;
        }
        for (k, v) in &p.keywords {
            match (k.as_str(), v.as_str()) {
                ("colnumeric", "yes") => attrs[A_NUMERIC as usize] = ON,
                ("colcasefirst", "upper") => attrs[A_CASE_FIRST as usize] = UPPER_FIRST,
                ("colcasefirst", "lower") => attrs[A_CASE_FIRST as usize] = LOWER_FIRST,
                ("colstrength", "primary") => attrs[A_STRENGTH as usize] = PRIMARY,
                ("colstrength", "secondary") => attrs[A_STRENGTH as usize] = SECONDARY,
                ("colstrength", "quaternary") => attrs[A_STRENGTH as usize] = QUATERNARY,
                ("colstrength", "identical") => attrs[A_STRENGTH as usize] = IDENTICAL,
                ("colalternate", "shifted") => attrs[A_ALTERNATE as usize] = SHIFTED,
                _ => {}
            }
        }
        Coll { locale: locale.to_string(), attrs, french_default }
    }

    /// The ICU4X collator these attributes stand for.
    pub fn collator(&self) -> Option<CollatorBorrowed<'static>> {
        let loc = crate::icu4x_locale(&self.locale);
        let mut prefs = CollatorPreferences::from(&loc);
        prefs.case_first = Some(match self.attrs[A_CASE_FIRST as usize] {
            UPPER_FIRST => CollationCaseFirst::Upper,
            LOWER_FIRST => CollationCaseFirst::Lower,
            _ => CollationCaseFirst::False,
        });
        prefs.numeric_ordering = Some(if self.attrs[A_NUMERIC as usize] == ON { CollationNumericOrdering::True } else { CollationNumericOrdering::False });
        let mut o = CollatorOptions::default();
        o.strength = Some(match self.attrs[A_STRENGTH as usize] {
            PRIMARY => Strength::Primary,
            SECONDARY => Strength::Secondary,
            QUATERNARY => Strength::Quaternary,
            IDENTICAL => Strength::Identical,
            _ => Strength::Tertiary,
        });
        o.alternate_handling = Some(if self.attrs[A_ALTERNATE as usize] == SHIFTED { AlternateHandling::Shifted } else { AlternateHandling::NonIgnorable });
        o.case_level = Some(if self.attrs[A_CASE_LEVEL as usize] == ON { CaseLevel::On } else { CaseLevel::Off });
        CollatorBorrowed::try_new(prefs, o).ok()
    }
}

#[derive(Default)]
pub struct State {
    pub coll: RefCell<Option<Coll>>,
    pub err: IntlError,
}

fn state(o: &Object) -> &State {
    o.native::<State>().expect("Collator state")
}

pub fn register(registry: &mut Registry) {
    registry.add_data("Collator", || Rc::new(State::default()));
    registry.add_hooks(
        "Collator",
        ClassHooks {
            clone_data: Some(|o: &Object| {
                let s = state(o);
                Rc::new(State { coll: RefCell::new(s.coll.borrow().clone()), err: s.err.clone() })
            }),
            ..Default::default()
        },
    );
}

pub static BUILTINS: &[Builtin] = &[
    b!("collator_create", f_create),
    b!("collator_compare", f_compare),
    b!("collator_get_attribute", f_get_attribute),
    b!("collator_set_attribute", f_set_attribute),
    b!("collator_get_strength", f_get_strength),
    b!("collator_set_strength", f_set_strength),
    Builtin { name: "collator_sort", run: f_sort, by_ref: &[1] },
    Builtin { name: "collator_sort_with_sort_keys", run: f_sort_with_sort_keys, by_ref: &[1] },
    Builtin { name: "collator_asort", run: f_asort, by_ref: &[1] },
    b!("collator_get_locale", f_get_locale),
    b!("collator_get_error_code", f_get_error_code),
    b!("collator_get_error_message", f_get_error_message),
    b!("collator_get_sort_key", f_get_sort_key),
];

pub static CLASSES: &[ClassSpec] = &[ClassSpec {
    name: "Collator",
    parent: None,
    interfaces: &[],
    flags: p8_runtime::object::CLASS_NOT_SERIALIZABLE,
    properties: &[],
    methods: crate::methods! {
        "Collator";
        statics: [("create", create_static)];
        "__construct" => m_construct;
        "compare" => f_compare;
        "sort" => f_sort;
        "sortWithSortKeys" => f_sort_with_sort_keys;
        "asort" => f_asort;
        "getAttribute" => f_get_attribute;
        "setAttribute" => f_set_attribute;
        "getStrength" => f_get_strength;
        "setStrength" => f_set_strength;
        "getLocale" => f_get_locale;
        "getErrorCode" => f_get_error_code;
        "getErrorMessage" => f_get_error_message;
        "getSortKey" => f_get_sort_key;
    },
    constants: crate::consts::COLLATOR,
    debug_info: None,
}];

fn create_static(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    crate::prep(ctx, "Collator::create", args)?;
    f_create(ctx, args)
}

fn construct(ctx: &mut dyn Ctx, st: &State, args: &[Value]) -> Result<Result<(), ()>, Throw> {
    crate::reset_global(ctx);
    let mut locale = str_arg(args, 0);
    if locale.len() > crate::locale::MAX_LOCALE_LEN {
        crate::set_error(ctx, None, U_ILLEGAL_ARGUMENT_ERROR, &format!("Locale string too long, should be no longer than {} characters", crate::locale::MAX_LOCALE_LEN))?;
        return Ok(Err(()));
    }
    if st.coll.borrow().is_some() {
        return Err(ctx.throw("Error", "Collator object is already constructed".into()));
    }
    if locale.is_empty() {
        locale = default_locale(ctx);
    }
    let c = Coll::new(&crate::uloc::get_name(&locale));
    // ICU refuses a keyword without a value ("@currency=;...")
    let empty_keyword = locale.split_once('@').is_some_and(|(_, kw)| kw.split(';').any(|kv| kv.split_once('=').is_some_and(|(_, v)| v.trim().is_empty())));
    if empty_keyword || c.collator().is_none() {
        st.err.code.set(U_ILLEGAL_ARGUMENT_ERROR);
        crate::set_global_code(ctx, U_ILLEGAL_ARGUMENT_ERROR);
        crate::set_message(ctx, Some(&st.err), "collator_create: unable to open ICU collator")?;
        return Ok(Err(()));
    }
    *st.coll.borrow_mut() = Some(c);
    Ok(Ok(()))
}

fn f_create(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let class = ctx.class(b"Collator").expect("Collator");
    let o = ctx.new_object(&class);
    match construct(ctx, state(&o), args)? {
        Ok(()) => Ok(Value::Object(o)),
        Err(()) => Ok(Value::Null),
    }
}

fn m_construct(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Value::Object(this) = arg(args, 0) else { unreachable!() };
    match construct(ctx, state(&this), &args[1..])? {
        Ok(()) => Ok(Value::Null),
        Err(()) => Err(ctx.throw("IntlException", "Constructor failed".into())),
    }
}

fn obj(ctx: &mut dyn Ctx, args: &[Value]) -> Result<Rc<Object>, Throw> {
    let Value::Object(o) = arg(args, 0) else { unreachable!("checked against the stub") };
    if state(&o).coll.borrow().is_none() {
        return Err(ctx.throw("Error", "Object not initialized".into()));
    }
    state(&o).err.reset();
    Ok(o)
}

fn coll(o: &Object) -> Coll {
    state(o).coll.borrow().clone().expect("constructed")
}

fn check(ctx: &mut dyn Ctx, st: &State, msg: &str) -> Result<bool, Throw> {
    let code = st.err.code.get();
    crate::set_global_code(ctx, code);
    if code > U_ZERO_ERROR {
        crate::set_message(ctx, Some(&st.err), msg)?;
        return Ok(false);
    }
    Ok(true)
}

fn f_compare(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let st = state(&o);
    let a = crate::bytes_arg(args, 1);
    let b2 = crate::bytes_arg(args, 2);
    let (Ok(a), Ok(b2)) = (std::str::from_utf8(&a), std::str::from_utf8(&b2)) else {
        let which = if std::str::from_utf8(&a).is_err() { "first" } else { "second" };
        st.err.code.set(10);
        crate::set_global_code(ctx, 10);
        crate::set_message(ctx, Some(&st.err), &format!("Error converting {which} argument to UTF-16"))?;
        return Ok(Value::Bool(false));
    };
    let c = coll(&o).collator().expect("collator");
    Ok(Value::Int(match c.compare(a, b2) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }))
}

fn ord(o: Option<Ordering>) -> Ordering {
    o.unwrap_or(Ordering::Equal)
}

/// The comparison a sort flag stands for (collator_get_compare_function).
fn compare_values(c: &CollatorBorrowed, flags: i64, a: &Value, b2: &Value) -> Ordering {
    let as_str = |v: &Value| v.to_php_string().map(|s| String::from_utf8_lossy(s.as_bytes()).into_owned()).unwrap_or_default();
    match flags {
        SORT_NUMERIC => {
            let n = |v: &Value| match v {
                Value::Str(s) => p8_runtime::numeric::leading_numeric(s.as_bytes()).map(|(n, _)| match n {
                    p8_runtime::numeric::Number::Int(i) => i as f64,
                    p8_runtime::numeric::Number::Float(f) => f,
                }).unwrap_or(0.0),
                v => v.to_float(),
            };
            ord(n(a).partial_cmp(&n(b2)))
        }
        SORT_STRING => c.compare(&as_str(a), &as_str(b2)),
        _ => {
            let numeric = |v: &Value| matches!(v, Value::Str(s) if p8_runtime::numeric::numeric_string(s.as_bytes()).is_some());
            match (a, b2) {
                (Value::Str(_), Value::Str(_)) if !numeric(a) || !numeric(b2) => c.compare(&as_str(a), &as_str(b2)),
                (Value::Str(_), Value::Str(_)) => ord(a.compare(b2)),
                // a string against a non-string: the string's numeric value, else as strings
                _ => ord(a.compare(b2)),
            }
        }
    }
}

fn sort_impl(ctx: &mut dyn Ctx, args: &mut [Value], keep_keys: bool) -> R {
    let o = obj(ctx, args)?;
    let st = state(&o);
    let flags = if args.len() > 2 { arg(args, 2).to_int() } else { SORT_REGULAR };
    let Some(Value::Ref(cell)) = args.get(1).cloned() else { return Ok(Value::Bool(false)) };
    let current = cell.borrow().deref();
    let Value::Array(arr) = current else { return Ok(Value::Bool(false)) };
    let c = coll(&o).collator().expect("collator");
    let mut entries: Vec<(Key, Value)> = arr.iter().map(|(k, v)| (k, v.clone())).collect();
    for (_, v) in &entries {
        if let Value::Str(s) = v.deref()
            && std::str::from_utf8(s.as_bytes()).is_err()
        {
            st.err.code.set(10);
            check(ctx, st, "Error converting hash from UTF-8 to UTF-16")?;
            return Ok(Value::Bool(false));
        }
    }
    p8_runtime::sort::zend_sort(&mut entries, &mut |x: &(Key, Value), y: &(Key, Value)| compare_values(&c, flags, &x.1.deref(), &y.1.deref()));
    let mut out = PhpArray::new();
    for (k, v) in entries {
        if keep_keys {
            out.insert(k, v);
        } else {
            let _ = out.push(v);
        }
    }
    *cell.borrow_mut() = Value::Array(Rc::new(out));
    Ok(Value::Bool(true))
}

fn f_sort(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    sort_impl(ctx, args, false)
}

fn f_asort(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    sort_impl(ctx, args, true)
}

fn f_sort_with_sort_keys(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let Some(Value::Ref(cell)) = args.get(1).cloned() else { return Ok(Value::Bool(false)) };
    let current = cell.borrow().deref();
    let Value::Array(arr) = current else { return Ok(Value::Bool(false)) };
    let c = coll(&o).collator().expect("collator");
    let mut entries: Vec<(Vec<u8>, Value)> = arr
        .values()
        .map(|v| {
            let text = v.deref().to_php_string().map(|s| String::from_utf8_lossy(s.as_bytes()).into_owned()).unwrap_or_default();
            (sort_key(&c, &text), v.clone())
        })
        .collect();
    // ICU's keys compared as C strings (strcmp), with zend_sort
    p8_runtime::sort::zend_sort(&mut entries, &mut |x: &(Vec<u8>, Value), y: &(Vec<u8>, Value)| x.0.cmp(&y.0));
    let out = PhpArray::from_list(entries.into_iter().map(|(_, v)| v).collect());
    *cell.borrow_mut() = Value::Array(Rc::new(out));
    Ok(Value::Bool(true))
}

fn sort_key(c: &CollatorBorrowed, text: &str) -> Vec<u8> {
    let mut k: Vec<u8> = Vec::new();
    let _ = c.write_sort_key_to(text, &mut k);
    k
}

fn f_get_sort_key(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let st = state(&o);
    let text = crate::bytes_arg(args, 1);
    let Ok(text) = std::str::from_utf8(&text) else {
        st.err.code.set(10);
        check(ctx, st, "Error converting first argument to UTF-16")?;
        return Ok(Value::Bool(false));
    };
    let c = coll(&o).collator().expect("collator");
    Ok(Value::Str(p8_runtime::string::PhpStr::new(sort_key(&c, text))))
}

fn f_get_attribute(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let st = state(&o);
    let a = arg(args, 1).to_int();
    if !(0..8).contains(&a) {
        st.err.code.set(U_ILLEGAL_ARGUMENT_ERROR);
        check(ctx, st, "Error getting attribute value")?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::Int(coll(&o).attrs[a as usize]))
}

fn valid(a: i64, v: i64, c: &Coll) -> bool {
    if v == DEFAULT_VALUE {
        return true;
    }
    match a {
        A_FRENCH => v == OFF && !c.french_default || v == ON && c.french_default,
        A_ALTERNATE => v == SHIFTED || v == NON_IGNORABLE,
        A_CASE_FIRST => matches!(v, OFF | LOWER_FIRST | UPPER_FIRST),
        A_CASE_LEVEL | A_NORMALIZATION | A_HIRAGANA | A_NUMERIC => v == OFF || v == ON,
        A_STRENGTH => matches!(v, PRIMARY | SECONDARY | TERTIARY | QUATERNARY | IDENTICAL),
        _ => false,
    }
}

fn set_attr(o: &Object, a: i64, v: i64) -> bool {
    let st = state(o);
    let mut b2 = st.coll.borrow_mut();
    let c = b2.as_mut().expect("constructed");
    if !(0..8).contains(&a) || !valid(a, v, c) {
        return false;
    }
    let v = if v == DEFAULT_VALUE { Coll::new(&c.locale).attrs[a as usize] } else { v };
    c.attrs[a as usize] = v;
    true
}

fn f_set_attribute(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let st = state(&o);
    if !set_attr(&o, arg(args, 1).to_int(), arg(args, 2).to_int()) {
        st.err.code.set(U_ILLEGAL_ARGUMENT_ERROR);
        check(ctx, st, "Error setting attribute value")?;
        return Ok(Value::Bool(false));
    }
    check(ctx, st, "Error setting attribute value")?;
    Ok(Value::Bool(true))
}

fn f_get_strength(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    Ok(Value::Int(coll(&o).attrs[A_STRENGTH as usize]))
}

fn f_set_strength(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    // ucol_setStrength with an invalid value: ICU reports nothing; PHP returns true
    let _ = set_attr(&o, A_STRENGTH, arg(args, 1).to_int());
    Ok(Value::Bool(true))
}

fn f_get_locale(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let o = obj(ctx, args)?;
    let st = state(&o);
    let ty = if args.len() > 1 { arg(args, 1).to_int() } else { 0 };
    if ty != 0 && ty != 1 {
        st.err.code.set(U_ILLEGAL_ARGUMENT_ERROR);
        check(ctx, st, "Error getting locale by type")?;
        return Ok(Value::Bool(false));
    }
    let locale = coll(&o).locale;
    let p = crate::uloc::parts(&locale);
    if p.keywords.iter().any(|(k, _)| k == "collation") {
        return Ok(s(locale));
    }
    let valid = crate::valid_locale(&locale);
    // the collation data is per language, but for these
    const REGIONAL: &[&str] = &["en_US", "en_US_POSIX", "de_AT", "fr_CA", "zh_Hant", "sr_Latn", "bs_Cyrl", "es_419"];
    let valid = if valid.contains('_') && !REGIONAL.contains(&valid.as_str()) { crate::uloc::subtags(&valid).lang } else { valid };
    if ty == 1 {
        return Ok(s(valid));
    }
    // the actual locale: the tailoring's (root when CLDR has none for the language)
    let actual = if has_tailoring(&valid) { valid } else { "root".to_string() };
    Ok(s(actual))
}

/// Whether CLDR tailors collation for the locale's language (ICU4X has tailoring data).
fn has_tailoring(locale: &str) -> bool {
    let lang = crate::uloc::subtags(locale).lang;
    // ICU4X loads a tailoring only for these; the root order serves the others
    const TAILORED: &[&str] = &[
        "af", "am", "ar", "as", "az", "be", "bg", "bn", "bo", "br", "bs", "ca", "ceb", "chr", "cs", "cy", "da", "de_AT", "dsb", "dz", "ee", "el", "eo", "es", "et", "fa", "ff", "fi", "fil", "fo", "fr_CA", "fy", "gl", "gu", "ha", "haw", "he", "hi", "hr", "hsb", "hu", "hy", "ig", "is", "ja", "ka", "kk", "kl", "km", "kn", "ko", "kok", "ku", "ky", "lkt", "ln", "lo", "lt", "lv", "mk", "ml", "mn", "mr", "mt", "my", "nb", "ne", "nn", "no", "om", "or", "pa", "pl", "ps", "ro", "ru", "se", "si", "sk", "sl", "smn", "sq", "sr", "sv", "ta", "te", "th", "tk", "to", "tr", "ug", "uk", "ur", "uz", "vi", "wae", "wo", "xh", "yi", "yo", "zh", "zu",
    ];
    TAILORED.contains(&lang.as_str()) || TAILORED.contains(&locale)
}

fn f_get_error_code(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Value::Object(o) = arg(args, 0) else { unreachable!() };
    Ok(Value::Int(state(&o).err.code.get()))
}

fn f_get_error_message(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Value::Object(o) = arg(args, 0) else { unreachable!() };
    Ok(s(state(&o).err.message()))
}
