//! ICU locale IDs as uloc.cpp reads them: `lang_Script_REGION_VARIANT@k=v;k=v`,
//! lenient about case and `-`/`_`, with BCP 47 tags that carry an extension
//! (`de-DE-u-co-phonebk`) converted first. What PHP's Locale functions call:
//! uloc_getLanguage / getScript / getCountry / getVariant, uloc_getName,
//! uloc_canonicalize, uloc_openKeywords and uloc_forLanguageTag.
//!
//! Reference: icu4c/source/common/uloc.cpp, uloc_tag.cpp (ICU 76)

use crate::uloc_tables::{LANG3, REGION3};

/// A locale ID's parts, as ICU normalizes them.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Parts {
    pub lang: String,
    pub script: String,
    pub region: String,
    pub variant: String,
    /// Sorted by key; keys lowercase.
    pub keywords: Vec<(String, String)>,
}

fn is_sep(c: u8) -> bool {
    c == b'_' || c == b'-'
}

fn is_term(c: Option<&u8>) -> bool {
    matches!(c, None | Some(b'.') | Some(b'@'))
}

/// The subtags of an ICU locale ID (no BCP 47 conversion): uloc_getLanguage
/// and friends. Returns the parts and where the keywords ('@') begin.
pub fn subtags(id: &str) -> Parts {
    let b = id.as_bytes();
    let mut p = Parts::default();
    let mut i = 0;
    // language
    if b.len() >= 2 && matches!(b[0], b'x' | b'X' | b'i' | b'I') && is_sep(b[1]) {
        p.lang.push(b[0].to_ascii_lowercase() as char);
        p.lang.push('-');
        i = 2;
    }
    while i < b.len() && !is_term(b.get(i)) && !is_sep(b[i]) {
        p.lang.push(b[i].to_ascii_lowercase() as char);
        i += 1;
    }
    if p.lang.len() == 3
        && let Some((_, two)) = LANG3.iter().find(|(three, _)| *three == p.lang)
    {
        p.lang = (*two).to_string();
    }
    if p.lang == "root" || p.lang == "und" {
        p.lang.clear();
    }
    let subtag_len = |from: usize| (from..b.len()).take_while(|&j| !is_term(b.get(j)) && !is_sep(b[j])).count();
    let mut had_region = false;
    if i < b.len() && is_sep(b[i]) {
        // script: four letters
        let n = subtag_len(i + 1);
        if n == 4 && b[i + 1..i + 5].iter().all(u8::is_ascii_alphabetic) {
            p.script.push(b[i + 1].to_ascii_uppercase() as char);
            p.script.extend(b[i + 2..i + 5].iter().map(|c| c.to_ascii_lowercase() as char));
            i += 5;
        }
    }
    if i < b.len() && is_sep(b[i]) {
        // region: two or three characters
        let n = subtag_len(i + 1);
        if n == 2 || n == 3 {
            let r: String = b[i + 1..i + 1 + n].iter().map(|c| c.to_ascii_uppercase() as char).collect();
            p.region = match REGION3.iter().find(|(three, _)| *three == r) {
                Some((_, two)) => (*two).to_string(),
                None => r,
            };
            i += 1 + n;
            had_region = true;
        }
    }
    // An extension singleton (-u-, -t-, -x-) ends the variants.
    let extension_at = |j: usize| j + 2 < b.len() && is_sep(b[j]) && matches!(b[j + 1].to_ascii_lowercase(), b'u' | b't' | b'x') && is_sep(b[j + 2]) || (j + 2 == b.len() && is_sep(b[j]) && b[j + 1].eq_ignore_ascii_case(&b'x'));
    if i < b.len() && is_sep(b[i]) && !extension_at(i) {
        i += 1;
        if !had_region && i < b.len() && is_sep(b[i]) {
            i += 1;
        }
        while i < b.len() && !is_term(b.get(i)) && !extension_at(i) {
            let c = b[i];
            p.variant.push(if c == b'-' { '_' } else { c.to_ascii_uppercase() as char });
            i += 1;
        }
        while p.variant.ends_with('_') {
            p.variant.pop();
        }
    }
    if let Some(at) = id.find('@') {
        p.keywords = keywords(&id[at + 1..]);
    }
    p
}

/// `k=v;k2=v2` → sorted (key lowercase, value trimmed) pairs; the first of a
/// repeated key wins.
fn keywords(s: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for kv in s.split(';') {
        let Some((k, v)) = kv.split_once('=') else { continue };
        let k = k.trim().to_ascii_lowercase();
        let v = v.trim();
        if k.is_empty() || v.is_empty() || !k.bytes().all(|c| c.is_ascii_alphanumeric()) {
            continue;
        }
        if out.iter().any(|(have, _)| *have == k) {
            continue;
        }
        out.push((k, v.to_string()));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Whether ICU reads this ID as a BCP 47 tag first (_hasBCP47Extension: no
/// '@' and some subtag is a single character).
pub fn has_bcp47_extension(id: &str) -> bool {
    let head = id.split('.').next().unwrap_or("");
    !id.contains('@') && head.contains(['-', '_']) && head.split(['-', '_']).any(|s| s.len() == 1)
}

/// uloc_forLanguageTag: a BCP 47 tag as an ICU locale ID.
pub fn for_language_tag(tag: &str) -> String {
    if let Some(preferred) = grandfathered_preferred(tag) {
        return for_language_tag(&preferred);
    }
    let subs: Vec<&str> = tag.split(['-', '_']).collect();
    let mut p = Parts::default();
    let mut i = 0;
    let alpha = |s: &str| s.bytes().all(|c| c.is_ascii_alphabetic());
    let digit = |s: &str| s.bytes().all(|c| c.is_ascii_digit());
    let alnum = |s: &str| s.bytes().all(|c| c.is_ascii_alphanumeric());
    let mut variants: Vec<String> = Vec::new();
    if let Some(&first) = subs.first() {
        if (first.len() >= 2 && first.len() <= 8 && alpha(first)) || first.eq_ignore_ascii_case("x") {
            if !first.eq_ignore_ascii_case("x") {
                p.lang = first.to_ascii_lowercase();
                i = 1;
            }
        } else {
            return String::new();
        }
    }
    // extlang (skipped as ICU does: up to three 3-letter subtags after a 2-3 letter language)
    while p.lang.len() <= 3 && i < subs.len() && subs[i].len() == 3 && alpha(subs[i]) && i <= 3 && !p.lang.is_empty() {
        // ICU replaces the language with the extlang only through canonicalization; keep the language
        i += 1;
    }
    if i < subs.len() && subs[i].len() == 4 && alpha(subs[i]) {
        let s = subs[i];
        p.script = s[..1].to_ascii_uppercase() + &s[1..].to_ascii_lowercase();
        i += 1;
    }
    if i < subs.len() && ((subs[i].len() == 2 && alpha(subs[i])) || (subs[i].len() == 3 && digit(subs[i]))) {
        p.region = subs[i].to_ascii_uppercase();
        i += 1;
    }
    while i < subs.len() {
        let s = subs[i];
        if (s.len() >= 5 && s.len() <= 8 && alnum(s)) || (s.len() == 4 && s.as_bytes()[0].is_ascii_digit() && alnum(s)) {
            variants.push(s.to_ascii_uppercase());
            i += 1;
        } else {
            break;
        }
    }
    p.variant = variants.join("_");
    let mut kws: Vec<(String, String)> = Vec::new();
    while i < subs.len() {
        let single = subs[i].to_ascii_lowercase();
        if single.len() != 1 || !alnum(&single) {
            break;
        }
        i += 1;
        let start = i;
        if single == "x" {
            let rest: Vec<&str> = subs[i..].to_vec();
            if !rest.is_empty() && rest.iter().all(|s| !s.is_empty() && s.len() <= 8 && alnum(s)) {
                kws.push(("x".into(), rest.join("-").to_ascii_lowercase()));
            }
            break;
        }
        while i < subs.len() && subs[i].len() >= 2 && subs[i].len() <= 8 && alnum(subs[i]) {
            i += 1;
        }
        let body: Vec<String> = subs[start..i].iter().map(|s| s.to_ascii_lowercase()).collect();
        if body.is_empty() {
            break;
        }
        if single == "u" {
            unicode_keywords(&body, &mut kws);
        } else {
            kws.push((single, body.join("-")));
        }
    }
    let mut seen: Vec<(String, String)> = Vec::new();
    for (k, v) in kws {
        if !seen.iter().any(|(h, _)| *h == k) {
            seen.push((k, v));
        }
    }
    seen.sort_by(|a, b| a.0.cmp(&b.0));
    p.keywords = seen;
    format_parts(&p)
}

/// The keywords a `-u-` extension's key/type pairs stand for, with ICU's
/// legacy key and type names.
fn unicode_keywords(body: &[String], out: &mut Vec<(String, String)>) {
    let mut j = 0;
    // attributes (3-8 chars before the first key) are kept as ICU does: "attribute"
    let mut attrs = Vec::new();
    while j < body.len() && body[j].len() != 2 {
        attrs.push(body[j].clone());
        j += 1;
    }
    if !attrs.is_empty() {
        out.push(("attribute".into(), attrs.join("-")));
    }
    while j < body.len() {
        let key = body[j].clone();
        j += 1;
        let mut types = Vec::new();
        while j < body.len() && body[j].len() != 2 {
            types.push(body[j].clone());
            j += 1;
        }
        let ty = if types.is_empty() { "true".to_string() } else { types.join("-") };
        let (lkey, lty) = legacy_keyword(&key, &ty);
        out.push((lkey, lty));
    }
}

/// uloc_toLegacyKey / uloc_toLegacyType for the keys ICU's data names.
pub fn legacy_keyword(key: &str, ty: &str) -> (String, String) {
    let lkey = match key {
        "ca" => "calendar",
        "co" => "collation",
        "cu" => "currency",
        "nu" => "numbers",
        "tz" => "timezone",
        "kn" => "colnumeric",
        "ks" => "colstrength",
        "kb" => "colbackwards",
        "kc" => "colcaselevel",
        "kf" => "colcasefirst",
        "kk" => "colnormalization",
        "ka" => "colalternate",
        "kr" => "colreorder",
        "kh" => "colhiraganaquaternary",
        "vt" => "variabletop",
        _ => key,
    };
    let lty = match (lkey, ty) {
        ("calendar", "gregory") => "gregorian",
        ("calendar", "ethioaa") => "ethiopic-amete-alem",
        ("calendar", "islamicc") => "islamic-civil",
        ("collation", "phonebk") => "phonebook",
        ("collation", "trad") => "traditional",
        ("collation", "dict") => "dictionary",
        ("collation", "gb2312") => "gb2312han",
        ("colstrength", "level1") => "primary",
        ("colstrength", "level2") => "secondary",
        ("colstrength", "level3") => "tertiary",
        ("colstrength", "level4") => "quaternary",
        ("colstrength", "identic") => "identical",
        ("colalternate", "noignore") => "non-ignorable",
        (_, "true") if lkey.starts_with("col") => "yes",
        (_, "false") if lkey.starts_with("col") => "no",
        _ => ty,
    };
    (lkey.to_string(), lty.to_string())
}

/// The ICU form of parts: `lang_Script_REGION_VARIANT@k=v;k=v`.
pub fn format_parts(p: &Parts) -> String {
    let mut s = p.lang.clone();
    if !p.script.is_empty() {
        s.push('_');
        s.push_str(&p.script);
    }
    if !p.region.is_empty() {
        s.push('_');
        s.push_str(&p.region);
    }
    if !p.variant.is_empty() {
        if p.region.is_empty() {
            s.push('_');
        }
        s.push('_');
        s.push_str(&p.variant);
    }
    if !p.keywords.is_empty() {
        s.push('@');
        let kv: Vec<String> = p.keywords.iter().map(|(k, v)| format!("{k}={v}")).collect();
        s.push_str(&kv.join(";"));
    }
    s
}

/// The parts of an ID, converted from BCP 47 first when ICU would.
pub fn parts(id: &str) -> Parts {
    if has_bcp47_extension(id) {
        let converted = for_language_tag(id);
        subtags(if converted.is_empty() { id } else { &converted })
    } else {
        subtags(id)
    }
}

/// uloc_getName.
pub fn get_name(id: &str) -> String {
    format_parts(&parts(id))
}

/// uloc_canonicalize: uloc_getName plus ICU's few remaining ID mappings.
pub fn canonicalize(id: &str) -> String {
    let mut p = parts(id);
    match (p.lang.as_str(), p.variant.as_str()) {
        ("art", "LOJBAN") => {
            p.lang = "jbo".into();
            p.variant.clear();
        }
        ("hy", "AREVMDA") => {
            p.lang = "hyw".into();
            p.variant.clear();
        }
        ("zh", "GAN" | "HAKKA" | "XIANG" | "WUU" | "MIN_NAN") if p.region.is_empty() => {}
        _ => {}
    }
    if p.lang == "zh" && p.script.is_empty() && p.variant.is_empty() {
        // "zh_GAN" style: the dialect is read as a region of three letters
        let mapped = match p.region.as_str() {
            "GAN" => Some("gan"),
            "WUU" => Some("wuu"),
            _ => None,
        };
        if let Some(l) = mapped {
            p.lang = l.into();
            p.region.clear();
        }
    }
    format_parts(&p)
}

/// The tags RFC 5646 grandfathered (with their preferred values), as PHP lists them.
pub static GRANDFATHERED: &[(&str, Option<&str>)] = &[
    ("art-lojban", Some("jbo")),
    ("cel-gaulish", None),
    ("en-GB-oed", Some("en-GB-oxendict")),
    ("i-ami", Some("ami")),
    ("i-bnn", Some("bnn")),
    ("i-default", None),
    ("i-enochian", None),
    ("i-hak", Some("hak")),
    ("i-klingon", Some("tlh")),
    ("i-lux", Some("lb")),
    ("i-mingo", None),
    ("i-navajo", Some("nv")),
    ("i-pwn", Some("pwn")),
    ("i-tao", Some("tao")),
    ("i-tay", Some("tay")),
    ("i-tsu", Some("tsu")),
    ("no-bok", Some("nb")),
    ("no-nyn", Some("nn")),
    ("sgn-BE-FR", Some("sfb")),
    ("sgn-BE-NL", Some("vgt")),
    ("sgn-BR", Some("bzs")),
    ("sgn-CH-DE", Some("sgg")),
    ("sgn-CO", Some("csn")),
    ("sgn-DE", Some("gsg")),
    ("sgn-DK", Some("dsl")),
    ("sgn-ES", Some("ssp")),
    ("sgn-FR", Some("fsl")),
    ("sgn-GB", Some("bfi")),
    ("sgn-GR", Some("gss")),
    ("sgn-IE", Some("isg")),
    ("sgn-IT", Some("ise")),
    ("sgn-JP", Some("jsl")),
    ("sgn-MX", Some("mfs")),
    ("sgn-NI", Some("ncs")),
    ("sgn-NL", Some("dse")),
    ("sgn-NO", Some("nsl")),
    ("sgn-PT", Some("psr")),
    ("sgn-SE", Some("swl")),
    ("sgn-US", Some("ase")),
    ("sgn-ZA", Some("sfs")),
    ("zh-cmn", Some("cmn")),
    ("zh-cmn-Hans", Some("cmn-Hans")),
    ("zh-cmn-Hant", Some("cmn-Hant")),
    ("zh-gan", Some("gan")),
    ("zh-guoyu", Some("cmn")),
    ("zh-hakka", Some("hak")),
    ("zh-min", None),
    ("zh-min-nan", Some("nan")),
    ("zh-wuu", Some("wuu")),
    ("zh-xiang", Some("hsn")),
];

/// The grandfathered tag `tag` is (case-insensitively), if any.
pub fn grandfathered(tag: &str) -> Option<&'static (&'static str, Option<&'static str>)> {
    GRANDFATHERED.iter().find(|(t, _)| t.eq_ignore_ascii_case(tag))
}

fn grandfathered_preferred(tag: &str) -> Option<String> {
    // ICU's own table of irregular tags (uloc_tag.cpp GRANDFATHERED)
    let lower = tag.to_ascii_lowercase();
    let mapped = match lower.as_str() {
        "art-lojban" => "jbo",
        "en-gb-oed" => "en-gb-oxendict",
        "i-ami" => "ami",
        "i-bnn" => "bnn",
        "i-hak" => "hak",
        "i-klingon" => "tlh",
        "i-lux" => "lb",
        "i-navajo" => "nv",
        "i-pwn" => "pwn",
        "i-tao" => "tao",
        "i-tay" => "tay",
        "i-tsu" => "tsu",
        "no-bok" => "nb",
        "no-nyn" => "nn",
        "zh-guoyu" => "zh",
        "zh-hakka" => "hak",
        "zh-xiang" => "hsn",
        "zh-min-nan" => "nan",
        "zh-gan" => "gan",
        "zh-wuu" => "wuu",
        "i-default" => "en-x-i-default",
        "i-enochian" => "und-x-i-enochian",
        "i-mingo" => "see-x-i-mingo",
        "cel-gaulish" => "xtg",
        "zh-min" => "nan-x-zh-min",
        _ => return None,
    };
    Some(mapped.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_icu() {
        assert_eq!(canonicalize("en_us"), "en_US");
        assert_eq!(canonicalize("de-DE-u-co-phonebk"), "de_DE@collation=phonebook");
        assert_eq!(canonicalize("x-klingon"), "@x=klingon");
        assert_eq!(canonicalize("i-klingon"), "tlh");
        assert_eq!(canonicalize("de-DEVA"), "de_Deva");
        assert_eq!(canonicalize("en__POSIX"), "en__POSIX");
        assert_eq!(canonicalize("en_US.UTF-8"), "en_US");
        assert_eq!(canonicalize("eng_US"), "en_US");
        assert_eq!(canonicalize("und_US"), "_US");
        assert_eq!(canonicalize("en_US_"), "en_US");
        assert_eq!(canonicalize("DE_de@CURRENCY=eur"), "de_DE@currency=eur");
        assert_eq!(canonicalize("en-a-bbb-x-ccc"), "en@a=bbb;x=ccc");
        assert_eq!(canonicalize("de-1996-1901"), "de__1996_1901");
        assert_eq!(canonicalize("en-Latn-US-boont-x-y"), "en_Latn_US_BOONT@x=y");
        assert_eq!(canonicalize("en-US-u-ca-gregory-co-phonebk"), "en_US@calendar=gregorian;collation=phonebook");
        assert_eq!(canonicalize("art__LOJBAN"), "jbo");
        assert_eq!(subtags("en-a-bbb-x-ccc").variant, "A_BBB");
        assert_eq!(subtags("zh-Hans-CN-u-nu-hanidec").variant, "");
        assert_eq!(canonicalize("C"), "c");
    }
}
