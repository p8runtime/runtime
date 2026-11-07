//! Locale and the locale_* functions (locale_methods.c): ICU's locale IDs
//! (`uloc.rs`), CLDR display names (ICU4X), RFC 4647 filtering and lookup,
//! and Accept-Language negotiation against ICU's available locales.

use crate::uloc::{self, Parts};
use crate::{R, U_ILLEGAL_ARGUMENT_ERROR, arg, b, default_locale, lossy, opt_str, reset_global, s, set_error, str_arg};
use icu_experimental::displaynames::DisplayNamesOptions;
use icu_experimental::displaynames::multi::{LanguageDisplayNames, RegionDisplayNames, ScriptDisplayNames, VariantDisplayNames};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, ClassSpec, Ctx, Throw};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;

pub const MAX_LOCALE_LEN: usize = 156;
const FULLNAME_CAPACITY: usize = 157;

pub static BUILTINS: &[Builtin] = &[
    b!("locale_get_default", f_get_default),
    b!("locale_set_default", f_set_default),
    b!("locale_get_primary_language", f_get_primary_language),
    b!("locale_get_script", f_get_script),
    b!("locale_get_region", f_get_region),
    b!("locale_get_keywords", f_get_keywords),
    b!("locale_get_display_script", f_get_display_script),
    b!("locale_get_display_region", f_get_display_region),
    b!("locale_get_display_name", f_get_display_name),
    b!("locale_get_display_language", f_get_display_language),
    b!("locale_get_display_variant", f_get_display_variant),
    b!("locale_compose", f_compose),
    b!("locale_parse", f_parse),
    b!("locale_get_all_variants", f_get_all_variants),
    b!("locale_filter_matches", f_filter_matches),
    b!("locale_canonicalize", f_canonicalize),
    b!("locale_lookup", f_lookup),
    b!("locale_accept_from_http", f_accept_from_http),
];

pub static CLASSES: &[ClassSpec] = &[ClassSpec {
    name: "Locale",
    parent: None,
    interfaces: &[],
    flags: 0,
    properties: &[],
    methods: crate::statics! {
        "getDefault" => f_get_default, "Locale::getDefault";
        "setDefault" => f_set_default, "Locale::setDefault";
        "getPrimaryLanguage" => f_get_primary_language, "Locale::getPrimaryLanguage";
        "getScript" => f_get_script, "Locale::getScript";
        "getRegion" => f_get_region, "Locale::getRegion";
        "getKeywords" => f_get_keywords, "Locale::getKeywords";
        "getDisplayScript" => f_get_display_script, "Locale::getDisplayScript";
        "getDisplayRegion" => f_get_display_region, "Locale::getDisplayRegion";
        "getDisplayName" => f_get_display_name, "Locale::getDisplayName";
        "getDisplayLanguage" => f_get_display_language, "Locale::getDisplayLanguage";
        "getDisplayVariant" => f_get_display_variant, "Locale::getDisplayVariant";
        "composeLocale" => f_compose, "Locale::composeLocale";
        "parseLocale" => f_parse, "Locale::parseLocale";
        "getAllVariants" => f_get_all_variants, "Locale::getAllVariants";
        "filterMatches" => f_filter_matches, "Locale::filterMatches";
        "lookup" => f_lookup, "Locale::lookup";
        "canonicalize" => f_canonicalize, "Locale::canonicalize";
        "acceptFromHttp" => f_accept_from_http, "Locale::acceptFromHttp";
    },
    constants: crate::consts::LOCALE,
    debug_info: None,
}];

fn too_long(ctx: &mut dyn Ctx, len: usize) -> Result<bool, Throw> {
    if len > MAX_LOCALE_LEN {
        set_error(ctx, None, U_ILLEGAL_ARGUMENT_ERROR, &format!("Locale string too long, should be no longer than {MAX_LOCALE_LEN} characters"))?;
        return Ok(true);
    }
    Ok(false)
}

fn f_get_default(ctx: &mut dyn Ctx, _: &mut [Value]) -> R {
    Ok(s(default_locale(ctx)))
}

fn f_set_default(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let mut name = str_arg(args, 0);
    if name.is_empty() {
        name = crate::icu_default_locale();
    }
    ctx.ini_set(b"intl.default_locale", s(name));
    Ok(Value::TRUE)
}

#[derive(Clone, Copy, PartialEq)]
enum Tag {
    Lang,
    Script,
    Region,
    Variant,
    Canonicalize,
}

/// get_icu_value_internal: Ok(Some) a value, Ok(None) no value; Err(()) failure.
fn icu_value(loc: &str, tag: Tag, from_parse: bool) -> Result<Option<String>, ()> {
    if loc.len() > MAX_LOCALE_LEN {
        return Err(());
    }
    let mut name = loc.to_string();
    if tag != Tag::Canonicalize {
        if uloc::grandfathered(loc).is_some() {
            return if tag == Tag::Lang { Ok(Some(loc.to_string())) } else { Err(()) };
        }
        if from_parse {
            if tag == Tag::Lang && is_id_prefix(loc) {
                // the whole name, but reported as no result (as PHP's code does)
                return Err(());
            }
            match singleton_pos(loc) {
                Some(0) => return Err(()),
                Some(p) => name = loc[..p - 1].to_string(),
                None => {}
            }
        }
    }
    let value = match tag {
        Tag::Lang => uloc::subtags(&name).lang,
        Tag::Script => uloc::subtags(&name).script,
        Tag::Region => uloc::subtags(&name).region,
        Tag::Variant => uloc::subtags(&name).variant,
        Tag::Canonicalize => uloc::canonicalize(&name),
    };
    Ok(if value.is_empty() { None } else { Some(value) })
}

fn is_id_prefix(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() > 1 && matches!(b[0], b'x' | b'X' | b'i' | b'I') && matches!(b[1], b'-' | b'_')
}

/// getSingletonPos: where a one-character subtag starts (0: the name begins with one).
fn singleton_pos(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    for i in 0..b.len() {
        if matches!(b[i], b'-' | b'_') {
            if i == 1 {
                return Some(0);
            }
            if i + 2 < b.len() && matches!(b[i + 2], b'-' | b'_') {
                return Some(i + 1);
            }
        }
    }
    None
}

fn get_value(ctx: &mut dyn Ctx, args: &mut [Value], tag: Tag, tag_name: &str) -> R {
    reset_global(ctx);
    let mut loc = str_arg(args, 0);
    if loc.is_empty() {
        loc = default_locale(ctx);
    }
    if too_long(ctx, loc.len())? {
        return Ok(Value::NULL);
    }
    match icu_value(&loc, tag, false) {
        Ok(Some(v)) => Ok(s(v)),
        Ok(None) => Ok(s("")),
        Err(()) => {
            set_error(ctx, None, U_ILLEGAL_ARGUMENT_ERROR, &format!("locale_get_{tag_name} : unable to get locale {tag_name}"))?;
            Ok(Value::NULL)
        }
    }
}

fn f_get_primary_language(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    get_value(ctx, args, Tag::Lang, "language")
}

fn f_get_script(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    get_value(ctx, args, Tag::Script, "script")
}

fn f_get_region(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    get_value(ctx, args, Tag::Region, "region")
}

fn f_canonicalize(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    get_value(ctx, args, Tag::Canonicalize, "canonicalize")
}

fn f_get_keywords(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    reset_global(ctx);
    let mut loc = str_arg(args, 0);
    if too_long(ctx, loc.len())? {
        return Ok(Value::NULL);
    }
    if loc.is_empty() {
        loc = default_locale(ctx);
    }
    let parts = uloc::parts(&loc);
    if parts.keywords.is_empty() {
        return Ok(Value::NULL);
    }
    if parts.keywords.iter().any(|(_, v)| !v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '/' | '.'))) {
        set_error(ctx, None, U_ILLEGAL_ARGUMENT_ERROR, "locale_get_keywords: Error encountered while getting the keyword  value for the  keyword")?;
        return Ok(Value::FALSE);
    }
    let mut out = PhpArray::new();
    for (k, v) in parts.keywords {
        out.insert(Key::from_string(PhpStr::from(k.as_bytes())), s(v));
    }
    Ok(Value::array(out))
}

// ---- display names ----

#[derive(Clone, Copy, PartialEq)]
enum Disp {
    Name,
    Lang,
    Script,
    Region,
    Variant,
}

struct Names {
    lang: Option<LanguageDisplayNames>,
    script: Option<ScriptDisplayNames>,
    region: Option<RegionDisplayNames>,
    variant: Option<VariantDisplayNames>,
}

impl Names {
    fn new(display: &str) -> Names {
        let loc = crate::icu4x_locale(display);
        let prefs = (&loc).into();
        let o = DisplayNamesOptions::default();
        Names {
            lang: LanguageDisplayNames::try_new(prefs, o).ok(),
            script: ScriptDisplayNames::try_new(prefs, o).ok(),
            region: RegionDisplayNames::try_new(prefs, o).ok(),
            variant: VariantDisplayNames::try_new(prefs, o).ok(),
        }
    }

    fn lang(&self, code: &str) -> String {
        icu_locale_core::subtags::Language::try_from_str(code)
            .ok()
            .and_then(|l| self.lang.as_ref().and_then(|n| n.of(l)))
            .map_or_else(|| code.to_string(), str::to_string)
    }

    fn script(&self, code: &str) -> String {
        icu_locale_core::subtags::Script::try_from_str(code)
            .ok()
            .and_then(|l| self.script.as_ref().and_then(|n| n.of(l)))
            .map_or_else(|| code.to_string(), str::to_string)
    }

    fn region(&self, code: &str) -> String {
        icu_locale_core::subtags::Region::try_from_str(code)
            .ok()
            .and_then(|l| self.region.as_ref().and_then(|n| n.of(l)))
            .map_or_else(|| code.to_string(), str::to_string)
    }

    fn variant(&self, code: &str) -> String {
        icu_locale_core::subtags::Variant::try_from_str(code)
            .ok()
            .and_then(|l| self.variant.as_ref().and_then(|n| n.of(l)))
            .map_or_else(|| code.to_string(), str::to_string)
    }
}

/// uloc_getDisplayName and friends, from CLDR's names in `display`.
fn display(parts: &Parts, what: Disp, display: &str) -> String {
    let names = Names::new(display);
    let lang = || if parts.lang.is_empty() { String::new() } else { names.lang(&parts.lang) };
    let variants = || -> Vec<String> { parts.variant.split('_').filter(|v| !v.is_empty()).map(|v| names.variant(v)).collect() };
    match what {
        Disp::Lang => lang(),
        Disp::Script => if parts.script.is_empty() { String::new() } else { names.script(&parts.script) },
        Disp::Region => if parts.region.is_empty() { String::new() } else { names.region(&parts.region) },
        Disp::Variant => variants().join(", "),
        Disp::Name => {
            let mut quals = Vec::new();
            if !parts.script.is_empty() {
                quals.push(names.script(&parts.script));
            }
            if !parts.region.is_empty() {
                quals.push(names.region(&parts.region));
            }
            quals.extend(variants());
            for (k, v) in &parts.keywords {
                quals.push(format!("{k}={v}"));
            }
            let l = lang();
            let (open, close, sep) = if matches!(crate::uloc::parts(display).lang.as_str(), "zh" | "ja" | "yue") { ("（", "）", "，") } else { (" (", ")", ", ") };
            match (l.is_empty(), quals.is_empty()) {
                (_, true) => l,
                (true, false) => quals.join(sep),
                (false, false) => format!("{l}{open}{}{close}", quals.join(sep)),
            }
        }
    }
}

fn get_display(ctx: &mut dyn Ctx, args: &mut [Value], what: Disp, tag_name: &str) -> R {
    reset_global(ctx);
    let mut loc = str_arg(args, 0);
    if loc.len() > FULLNAME_CAPACITY {
        set_error(ctx, None, U_ILLEGAL_ARGUMENT_ERROR, &format!("locale_get_display_{tag_name} : name too long"))?;
        return Ok(Value::FALSE);
    }
    if loc.is_empty() {
        loc = default_locale(ctx);
    }
    if what != Disp::Name
        && let Some((tag, preferred)) = uloc::grandfathered(&loc)
    {
        if what == Disp::Lang {
            loc = preferred.unwrap_or(tag).to_string();
        } else {
            return Ok(Value::FALSE);
        }
    }
    let display_loc = match opt_str(args, 1) {
        Some(d) => d,
        None => default_locale(ctx),
    };
    let parts = uloc::parts(&loc);
    Ok(s(display(&parts, what, &display_loc)))
}

fn f_get_display_name(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    get_display(ctx, args, Disp::Name, "name")
}

fn f_get_display_language(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    get_display(ctx, args, Disp::Lang, "language")
}

fn f_get_display_script(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    get_display(ctx, args, Disp::Script, "script")
}

fn f_get_display_region(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    get_display(ctx, args, Disp::Region, "region")
}

fn f_get_display_variant(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    get_display(ctx, args, Disp::Variant, "variant")
}

// ---- compose / parse ----

fn find<'a>(a: &'a PhpArray, key: &str) -> Option<&'a Value> {
    a.get(&Key::from_string(PhpStr::from(key.as_bytes())))
}

enum Appended {
    Done,
    Missing,
    NotString,
}

fn append_key(out: &mut String, a: &PhpArray, key: &str) -> Appended {
    let Some(v) = find(a, key).map(Value::deref) else { return Appended::Missing };
    let Some(v) = v.as_str() else { return Appended::NotString };
    if key != "language" && key != "grandfathered" {
        out.push('_');
    }
    out.push_str(&lossy(v.as_bytes()));
    Appended::Done
}

fn append_multiple(out: &mut String, a: &PhpArray, key: &str) -> Appended {
    let prefix = |out: &mut String| {
        if key.starts_with("private") {
            out.push_str("_x");
        }
    };
    if let Some(found) = find(a, key).map(Value::deref) {
        if let Some(v) = found.as_str() {
            prefix(out);
            out.push('_');
            out.push_str(&lossy(v.as_bytes()));
            return Appended::Done;
        }
        if let Some(list) = found.as_array() {
            let mut first = true;
            for v in list.values() {
                let v = v.deref();
                let Some(v) = v.as_str() else { return Appended::NotString };
                if first {
                    prefix(out);
                    first = false;
                }
                out.push('_');
                out.push_str(&lossy(v.as_bytes()));
            }
            return Appended::Done;
        }
        return Appended::NotString;
    }
    let max = match key {
        "variant" => 15,
        "extlang" => 3,
        _ => 15,
    };
    let mut first = true;
    for i in 0..max {
        if let Some(found) = find(a, &format!("{key}{i}")).map(Value::deref) {
            let Some(v) = found.as_str() else { return Appended::NotString };
            if first {
                prefix(out);
                first = false;
            }
            out.push('_');
            out.push_str(&lossy(v.as_bytes()));
        }
    }
    Appended::Done
}

fn f_compose(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    reset_global(ctx);
    let Ok(a) = arg(args, 0).into_array() else { return Ok(Value::FALSE) };
    if a.is_empty() {
        return Ok(Value::FALSE);
    }
    let mut out = String::new();
    let not_string = |ctx: &mut dyn Ctx| -> R {
        set_error(ctx, None, U_ILLEGAL_ARGUMENT_ERROR, "locale_compose: parameter array element is not a string")?;
        Ok(Value::FALSE)
    };
    match append_key(&mut out, &a, "grandfathered") {
        Appended::Done => return Ok(s(out)),
        Appended::NotString => return not_string(ctx),
        Appended::Missing => {}
    }
    match append_key(&mut out, &a, "language") {
        Appended::Missing => {
            let name = crate::called("locale_compose");
            return Err(ctx.throw("ValueError", format!("{name}(): Argument #1 ($subtags) must contain a \"language\" key")));
        }
        Appended::NotString => return not_string(ctx),
        Appended::Done => {}
    }
    for (key, multiple) in [("extlang", true), ("script", false), ("region", false), ("variant", true), ("private", true)] {
        let r = if multiple { append_multiple(&mut out, &a, key) } else { append_key(&mut out, &a, key) };
        if let Appended::NotString = r {
            return not_string(ctx);
        }
    }
    Ok(s(out))
}

/// get_private_subtags.
fn private_subtags(loc: &str) -> Option<String> {
    let mut rest = loc;
    while let Some(pos) = singleton_pos(rest) {
        let c = rest.as_bytes()[pos];
        if c == b'x' || c == b'X' {
            if pos + 2 == rest.len() {
                return None;
            }
            return rest.get(pos + 2..).map(str::to_string);
        }
        if pos + 1 >= rest.len() {
            break;
        }
        rest = &rest[pos + 1..];
    }
    None
}

/// php_strtok_r over "-_": the first token, then the following ones while longer than one character.
fn tokens(v: &str) -> Vec<String> {
    let mut it = v.split(['-', '_']).filter(|t| !t.is_empty());
    let mut out = Vec::new();
    if let Some(first) = it.next() {
        out.push(first.to_string());
        for t in it {
            if t.len() <= 1 {
                break;
            }
            out.push(t.to_string());
        }
    }
    out
}

fn f_parse(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    reset_global(ctx);
    let mut loc = str_arg(args, 0);
    if too_long(ctx, loc.len())? {
        return Ok(Value::NULL);
    }
    if loc.is_empty() {
        loc = default_locale(ctx);
    }
    let mut out = PhpArray::new();
    let put = |out: &mut PhpArray, k: &str, v: String| out.insert(Key::from_string(PhpStr::from(k.as_bytes())), s(v));
    if uloc::grandfathered(&loc).is_some() {
        put(&mut out, "grandfathered", loc);
        return Ok(Value::array(out));
    }
    for (tag, key) in [(Tag::Lang, "language"), (Tag::Script, "script"), (Tag::Region, "region")] {
        if let Ok(Some(v)) = icu_value(&loc, tag, true) {
            put(&mut out, key, v);
        }
    }
    if let Ok(Some(v)) = icu_value(&loc, Tag::Variant, true) {
        for (i, t) in tokens(&v).into_iter().enumerate() {
            put(&mut out, &format!("variant{i}"), t);
        }
    }
    if let Some(v) = private_subtags(&loc) {
        for (i, t) in tokens(&v).into_iter().enumerate() {
            put(&mut out, &format!("private{i}"), t);
        }
    }
    Ok(Value::array(out))
}

fn f_get_all_variants(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    reset_global(ctx);
    let mut loc = str_arg(args, 0);
    if loc.is_empty() {
        loc = default_locale(ctx);
    }
    if too_long(ctx, loc.len())? {
        return Ok(Value::NULL);
    }
    let mut out = PhpArray::new();
    if uloc::grandfathered(&loc).is_none()
        && let Ok(Some(v)) = icu_value(&loc, Tag::Variant, false)
    {
        for t in tokens(&v) {
            let _ = out.push(s(t));
        }
    }
    Ok(Value::array(out))
}

// ---- matching ----

/// strToMatch: lowercase, '-' to '_'.
fn to_match(s: &str) -> String {
    s.chars().map(|c| if c == '-' { '_' } else { c.to_ascii_lowercase() }).collect()
}

fn f_filter_matches(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    reset_global(ctx);
    let tag = str_arg(args, 0);
    let mut range = str_arg(args, 1);
    let canonical = arg(args, 2).to_bool();
    if range.is_empty() {
        range = default_locale(ctx);
    }
    if range == "*" {
        return Ok(Value::TRUE);
    }
    if too_long(ctx, range.len())? || too_long(ctx, tag.len())? {
        return Ok(Value::NULL);
    }
    let (t, r) = if canonical {
        let r = uloc::canonicalize(&range);
        if r.is_empty() {
            set_error(ctx, None, crate::U_ZERO_ERROR, "locale_filter_matches : unable to canonicalize loc_range")?;
            return Ok(Value::FALSE);
        }
        let t = uloc::canonicalize(&tag);
        if t.is_empty() {
            set_error(ctx, None, crate::U_ZERO_ERROR, "locale_filter_matches : unable to canonicalize lang_tag")?;
            return Ok(Value::FALSE);
        }
        (to_match(&t), to_match(&r))
    } else {
        if tag.is_empty() {
            return Ok(Value::FALSE);
        }
        (to_match(&tag), to_match(&range))
    };
    if let Some(rest) = t.strip_prefix(r.as_str()) {
        let next = rest.as_bytes().first();
        let ok = matches!(next, None | Some(b'_') | Some(b'-')) || (canonical && next == Some(&b'@'));
        return Ok(Value::bool(ok));
    }
    Ok(Value::FALSE)
}

/// getStrrtokenPos.
fn strrtoken_pos(s: &[u8], saved: usize) -> Option<usize> {
    let mut result: isize = -1;
    let mut i = saved as isize - 1;
    while i >= 0 {
        let c = s[i as usize];
        if c == b'_' || c == b'-' || c == b'@' {
            result = if i >= 2 && matches!(s[(i - 2) as usize], b'_' | b'-') { i - 2 } else { i };
            break;
        }
        i -= 1;
    }
    if result < 1 { None } else { Some(result as usize) }
}

fn f_lookup(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    reset_global(ctx);
    let Ok(tags) = arg(args, 0).into_array() else { return Ok(Value::NULL) };
    let mut range = str_arg(args, 1);
    let canonical = arg(args, 2).to_bool();
    let fallback = opt_str(args, 3);
    if range.is_empty() {
        range = match &fallback {
            Some(f) => f.clone(),
            None => default_locale(ctx),
        };
    }
    if too_long(ctx, range.len())? {
        return Ok(Value::NULL);
    }
    if tags.is_empty() {
        return Ok(s(""));
    }
    let fail = |ctx: &mut dyn Ctx, what: &str| -> R {
        set_error(ctx, None, U_ILLEGAL_ARGUMENT_ERROR, &format!("lookup_loc_range: unable to canonicalize {what}"))?;
        Ok(Value::NULL)
    };
    let mut list: Vec<(String, String)> = Vec::new();
    for v in tags.values() {
        let v = v.deref();
        let Some(t) = v.as_str() else {
            let name = crate::called("locale_lookup");
            return Err(ctx.throw("TypeError", format!("{name}(): Argument #2 ($languageTag) must only contain string values")));
        };
        let t = lossy(t.as_bytes());
        if t.is_empty() {
            return fail(ctx, "lang_tag");
        }
        list.push((to_match(&t), t));
    }
    if canonical {
        for (m, _) in list.iter_mut() {
            let c = uloc::canonicalize(m);
            if c.is_empty() {
                return fail(ctx, "lang_tag");
            }
            *m = to_match(&c);
        }
        let c = uloc::canonicalize(&range);
        if c.is_empty() {
            return fail(ctx, "loc_range");
        }
        range = c;
    }
    let cur = to_match(&range);
    if cur.is_empty() {
        return fail(ctx, "lang_tag");
    }
    let mut pos = Some(cur.len());
    while let Some(p) = pos.filter(|&p| p > 0) {
        for (m, orig) in &list {
            if m.len() == p && cur.as_bytes()[..p] == *m.as_bytes() {
                return Ok(s(if canonical { m.clone() } else { orig.clone() }));
            }
        }
        pos = strrtoken_pos(cur.as_bytes(), p);
    }
    Ok(match fallback {
        Some(f) => s(f),
        None => s(""),
    })
}

/// uloc_acceptLanguageFromHTTP against ICU's available locales: the
/// languages by quality (stable), each matched exactly, then through its
/// likely script, then by truncation.
fn f_accept_from_http(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let header = str_arg(args, 0);
    if header.len() > FULLNAME_CAPACITY && header.split(',').any(|p| p.len() > FULLNAME_CAPACITY) {
        set_error(ctx, None, U_ILLEGAL_ARGUMENT_ERROR, "locale_accept_from_http: locale string too long")?;
        return Ok(Value::FALSE);
    }
    let mut prefs: Vec<(f64, usize, String)> = Vec::new();
    for (i, item) in header.split(',').enumerate() {
        let mut parts = item.split(';');
        let tag = parts.next().unwrap_or("").trim().to_string();
        let mut q = 1.0;
        for p in parts {
            if let Some(v) = p.trim().strip_prefix("q=") {
                q = v.trim().parse().unwrap_or(0.0);
            }
        }
        if tag.is_empty() || q <= 0.0 {
            continue;
        }
        prefs.push((q, i, tag));
    }
    prefs.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal).then(a.1.cmp(&b.1)));
    let available = crate::uloc_tables::AVAILABLE;
    for (_, _, tag) in &prefs {
        if tag == "*" {
            continue;
        }
        let name = uloc::get_name(&uloc::for_language_tag(tag));
        if name.is_empty() {
            continue;
        }
        if let Some(a) = available.iter().find(|a| a.eq_ignore_ascii_case(&name)) {
            return Ok(s(*a));
        }
        // through the likely script: zh_TW is zh_Hant_TW
        if let Ok(mut l) = icu_locale_core::Locale::try_from_str(&name.replace('_', "-")) {
            icu_locale::LocaleExpander::new_common().maximize(&mut l.id);
            let p = uloc::subtags(&l.id.to_string());
            let with_script = uloc::format_parts(&Parts { variant: String::new(), keywords: Vec::new(), ..p.clone() });
            if let Some(a) = available.iter().find(|a| a.eq_ignore_ascii_case(&with_script)) {
                return Ok(s(*a));
            }
        }
        let mut n = name.clone();
        while let Some(i) = n.rfind('_') {
            n.truncate(i);
            if let Some(a) = available.iter().find(|a| a.eq_ignore_ascii_case(&n)) {
                return Ok(s(*a));
            }
        }
    }
    Ok(Value::FALSE)
}
