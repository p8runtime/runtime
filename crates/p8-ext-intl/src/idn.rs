//! idn_to_ascii / idn_to_utf8 (idn/idn.c): UTS 46 processing with the
//! idna crate (its mapping and normalization are ICU4X's), ICU's error bits
//! worked out label by label, transitional processing by mapping the four
//! deviation characters first.
//!
//! The idna crate always checks the Bidi and ContextJ rules; ICU does only
//! when IDNA_CHECK_BIDI / IDNA_CHECK_CONTEXTJ are given.

use crate::{R, arg, s};
use idna::uts46::{AsciiDenyList, Hyphens, Uts46};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "idn_to_ascii", run: idn_to_ascii, by_ref: &[3] },
    Builtin { name: "idn_to_utf8", run: idn_to_utf8, by_ref: &[3] },
];

const USE_STD3_RULES: i64 = 2;
const CHECK_BIDI: i64 = 4;
const CHECK_CONTEXTJ: i64 = 8;
const NONTRANSITIONAL_TO_ASCII: i64 = 16;
const NONTRANSITIONAL_TO_UNICODE: i64 = 32;

const E_EMPTY_LABEL: i64 = 1;
const E_LABEL_TOO_LONG: i64 = 2;
const E_DOMAIN_NAME_TOO_LONG: i64 = 4;
const E_LEADING_HYPHEN: i64 = 8;
const E_TRAILING_HYPHEN: i64 = 16;
const E_HYPHEN_3_4: i64 = 32;
const E_LEADING_COMBINING_MARK: i64 = 64;
const E_DISALLOWED: i64 = 128;
const E_PUNYCODE: i64 = 256;
const E_BIDI: i64 = 2048;
const E_CONTEXTJ: i64 = 4096;

fn idn_to_ascii(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    convert(ctx, args, true)
}

fn idn_to_utf8(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    convert(ctx, args, false)
}

fn is_deviation(c: char) -> bool {
    matches!(c, '\u{DF}' | '\u{3C2}' | '\u{200C}' | '\u{200D}')
}

/// The UTS 46 processing: the output (U+FFFD where a label failed) and ICU's error bits.
pub fn process(domain: &str, flags: i64, to_ascii: bool) -> (String, i64, bool) {
    let transitional_different = domain.chars().any(is_deviation);
    let nontransitional = flags & if to_ascii { NONTRANSITIONAL_TO_ASCII } else { NONTRANSITIONAL_TO_UNICODE } != 0;
    let input: String = if nontransitional {
        domain.to_string()
    } else {
        domain
            .chars()
            .filter(|&c| c != '\u{200C}' && c != '\u{200D}')
            .map(|c| match c {
                '\u{DF}' => "ss".to_string(),
                '\u{3C2}' => "\u{3C3}".to_string(),
                c => c.to_string(),
            })
            .collect()
    };
    let deny = if flags & USE_STD3_RULES != 0 { AsciiDenyList::STD3 } else { AsciiDenyList::EMPTY };
    let (unicode, _) = Uts46::new().to_unicode(input.as_bytes(), deny, Hyphens::Allow);
    let unicode = unicode.into_owned();
    let mut errors = 0;
    let originals: Vec<&str> = input.split(['.', '\u{3002}', '\u{FF0E}', '\u{FF61}']).collect();
    let labels: Vec<&str> = unicode.split('.').collect();
    let mut out_labels: Vec<String> = Vec::new();
    for (i, label) in labels.iter().enumerate() {
        let last = i + 1 == labels.len();
        if label.is_empty() {
            if !last || labels.len() == 1 {
                errors |= E_EMPTY_LABEL;
            }
            out_labels.push(String::new());
            continue;
        }
        let mut label = *label;
        let joined: String;
        if label.contains('\u{FFFD}') {
            let orig = originals.get(i).copied().unwrap_or("");
            if orig.len() >= 4 && orig[..4].eq_ignore_ascii_case("xn--") {
                errors |= E_PUNYCODE;
            } else if orig.chars().any(|c| matches!(c, '\u{200C}' | '\u{200D}')) && flags & CHECK_CONTEXTJ != 0 {
                // a joiner out of context: ICU still converts the label
                errors |= E_CONTEXTJ;
                joined = orig.to_lowercase();
                label = &joined;
            } else if flags & CHECK_BIDI != 0 && label.chars().any(|c| matches!(icu_properties::CodePointMapData::<icu_properties::props::BidiClass>::new().get(c), icu_properties::props::BidiClass::RightToLeft | icu_properties::props::BidiClass::ArabicLetter)) {
                errors |= E_BIDI;
            } else {
                errors |= E_DISALLOWED;
            }
        }
        if label.starts_with('-') {
            errors |= E_LEADING_HYPHEN;
        }
        if label.ends_with('-') {
            errors |= E_TRAILING_HYPHEN;
        }
        if label.len() >= 4 && label.as_bytes()[2..4] == *b"--" && !label.starts_with("xn--") {
            errors |= E_HYPHEN_3_4;
        }
        if let Some(c) = label.chars().next()
            && matches!(
                icu_properties::CodePointMapData::<icu_properties::props::GeneralCategory>::new().get(c),
                icu_properties::props::GeneralCategory::NonspacingMark | icu_properties::props::GeneralCategory::SpacingMark | icu_properties::props::GeneralCategory::EnclosingMark
            )
        {
            errors |= E_LEADING_COMBINING_MARK;
        }
        let ascii_label = if to_ascii && !label.is_ascii() && !label.contains('\u{FFFD}') {
            match idna::punycode::encode_str(label) {
                Some(p) => format!("xn--{p}"),
                None => label.to_string(),
            }
        } else {
            label.to_string()
        };
        if to_ascii && ascii_label.len() > 63 {
            errors |= E_LABEL_TOO_LONG;
        }
        out_labels.push(if to_ascii { ascii_label } else { label.to_string() });
    }
    let out = out_labels.join(".");
    if to_ascii {
        let len = if out.ends_with('.') { out.len() - 1 } else { out.len() };
        if len > 253 {
            errors |= E_DOMAIN_NAME_TOO_LONG;
        }
    }
    (out, errors, transitional_different)
}

fn convert(ctx: &mut dyn Ctx, args: &mut [Value], to_ascii: bool) -> R {
    crate::reset_global(ctx);
    let domain = crate::bytes_arg(args, 0);
    let flags = if args.len() > 1 { arg(args, 1).to_int() } else { 48 };
    let variant = if args.len() > 2 { arg(args, 2).to_int() } else { 1 };
    let name = if to_ascii { "idn_to_ascii" } else { "idn_to_utf8" };
    if domain.is_empty() {
        return Err(ctx.throw("ValueError", format!("{name}(): Argument #1 ($domain) must not be empty")));
    }
    if variant != 1 {
        return Err(ctx.throw("ValueError", format!("{name}(): Argument #2 ($flags) must be INTL_IDNA_VARIANT_UTS46")));
    }
    let text = String::from_utf8_lossy(&domain).into_owned();
    let (out, errors, different) = process(&text, flags, to_ascii);
    let too_long = if to_ascii { out.len() >= 255 } else { out.len() >= 252 * 4 };
    if too_long {
        crate::set_global_code(ctx, 15);
        crate::set_message(ctx, None, &format!("{name}: failed to convert name"))?;
        return Ok(Value::Bool(false));
    }
    if let Some(Value::Ref(cell)) = args.get(3) {
        let mut info = PhpArray::new();
        let put = |a: &mut PhpArray, k: &str, v: Value| a.insert(Key::from_string(PhpStr::from(k.as_bytes())), v);
        put(&mut info, "result", s(out.clone()));
        put(&mut info, "isTransitionalDifferent", Value::Bool(different));
        put(&mut info, "errors", Value::Int(errors));
        *cell.borrow_mut() = Value::Array(Rc::new(info));
    }
    Ok(if errors == 0 { s(out) } else { Value::Bool(false) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uts46_like_icu() {
        assert_eq!(process("bücher.de", 48, true).0, "xn--bcher-kva.de");
        assert_eq!(process("xn--bcher-kva.de", 48, false).0, "bücher.de");
        assert_eq!(process("faß.de", 0, true).0, "fass.de");
        assert_eq!(process("faß.de", 48, true).0, "xn--fa-hia.de");
        assert_eq!(process("-a.de", 48, true).1, E_LEADING_HYPHEN);
        assert_eq!(process("a..de", 48, true).1, E_EMPTY_LABEL);
    }
}
