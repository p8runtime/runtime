//! Normalizer and normalizer_* (normalizer/normalizer_normalize.c) over
//! icu_normalizer: NFC, NFD, NFKC, NFKD, and NFKC_Casefold as Unicode
//! defines it (NFKC of the full case folding of NFKC, without
//! Default_Ignorable_Code_Point characters).

use crate::{R, arg, b, reset_global, s, set_error};
use icu_normalizer::properties::{CanonicalDecompositionBorrowed, Decomposed};
use icu_normalizer::{ComposingNormalizerBorrowed, DecomposingNormalizerBorrowed};
use p8_runtime::ext::{Builtin, ClassSpec, Ctx};
use p8_runtime::value::Value;

pub const FORM_D: i64 = 4;
pub const FORM_KD: i64 = 8;
pub const FORM_C: i64 = 16;
pub const FORM_KC: i64 = 32;
pub const FORM_KC_CF: i64 = 48;

pub static BUILTINS: &[Builtin] = &[
    b!("normalizer_normalize", f_normalize),
    b!("normalizer_is_normalized", f_is_normalized),
    b!("normalizer_get_raw_decomposition", f_get_raw_decomposition),
];

pub static CLASSES: &[ClassSpec] = &[ClassSpec {
    name: "Normalizer",
    parent: None,
    interfaces: &[],
    flags: 0,
    properties: &[],
    methods: crate::statics! {
        "normalize" => f_normalize, "Normalizer::normalize";
        "isNormalized" => f_is_normalized, "Normalizer::isNormalized";
        "getRawDecomposition" => f_get_raw_decomposition, "Normalizer::getRawDecomposition";
    },
    constants: crate::consts::NORMALIZER,
    debug_info: None,
}];

/// `text` in normalization form `form` (a valid form).
pub fn normalize(text: &str, form: i64) -> String {
    match form {
        FORM_D => DecomposingNormalizerBorrowed::new_nfd().normalize(text).into_owned(),
        FORM_KD => DecomposingNormalizerBorrowed::new_nfkd().normalize(text).into_owned(),
        FORM_KC => ComposingNormalizerBorrowed::new_nfkc().normalize(text).into_owned(),
        FORM_KC_CF => nfkc_casefold(text),
        _ => ComposingNormalizerBorrowed::new_nfc().normalize(text).into_owned(),
    }
}

fn nfkc_casefold(text: &str) -> String {
    let nfkc = ComposingNormalizerBorrowed::new_nfkc();
    let folded = icu_casemap::CaseMapperBorrowed::new().fold_string(&nfkc.normalize(text)).into_owned();
    let ignorable = icu_properties::CodePointSetData::new::<icu_properties::props::DefaultIgnorableCodePoint>();
    let kept: String = folded.chars().filter(|&c| !ignorable.contains(c)).collect();
    // the folding can make a new NFKC sequence; repeat until stable
    let once = nfkc.normalize(&kept).into_owned();
    let again = icu_casemap::CaseMapperBorrowed::new().fold_string(&once).into_owned();
    if again == once { once } else { nfkc.normalize(&again).into_owned() }
}

fn form_arg(ctx: &mut dyn Ctx, args: &[Value]) -> Result<i64, p8_runtime::ext::Throw> {
    let form = if args.len() > 1 { arg(args, 1).to_int() } else { FORM_C };
    match form {
        FORM_D | FORM_KD | FORM_C | FORM_KC | FORM_KC_CF => Ok(form),
        _ => {
            let name = crate::called(ctx.builtin_name());
            Err(ctx.throw("ValueError", format!("{name}(): Argument #2 ($form) must be a a valid normalization form")))
        }
    }
}

fn f_normalize(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    reset_global(ctx);
    let form = form_arg(ctx, args)?;
    let bytes = crate::bytes_arg(args, 0);
    let Ok(text) = std::str::from_utf8(&bytes) else {
        crate::set_global_code(ctx, 10);
        crate::set_message(ctx, None, "Error converting input string to UTF-16")?;
        return Ok(Value::Bool(false));
    };
    Ok(s(normalize(text, form)))
}

fn f_is_normalized(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    reset_global(ctx);
    let form = form_arg(ctx, args)?;
    let bytes = crate::bytes_arg(args, 0);
    let Ok(text) = std::str::from_utf8(&bytes) else {
        crate::set_global_code(ctx, 10);
        crate::set_message(ctx, None, "Error converting string to UTF-16.")?;
        return Ok(Value::Bool(false));
    };
    let ok = match form {
        FORM_D => DecomposingNormalizerBorrowed::new_nfd().is_normalized(text),
        FORM_KD => DecomposingNormalizerBorrowed::new_nfkd().is_normalized(text),
        FORM_KC => ComposingNormalizerBorrowed::new_nfkc().is_normalized(text),
        FORM_KC_CF => nfkc_casefold(text) == text,
        _ => ComposingNormalizerBorrowed::new_nfc().is_normalized(text),
    };
    Ok(Value::Bool(ok))
}

fn f_get_raw_decomposition(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    reset_global(ctx);
    let form = if args.len() > 1 { arg(args, 1).to_int() } else { FORM_C };
    let bytes = crate::bytes_arg(args, 0);
    if std::str::from_utf8(&bytes).is_err() && one_invalid_sequence(&bytes) {
        crate::set_global_code(ctx, crate::U_ILLEGAL_ARGUMENT_ERROR);
        crate::set_message(ctx, None, "Code point out of range")?;
        return Ok(Value::Null);
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut chars = text.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        crate::set_global_code(ctx, crate::U_ILLEGAL_ARGUMENT_ERROR);
        crate::set_message(ctx, None, "Input string must be exactly one UTF-8 encoded code point long.")?;
        return Ok(Value::Null);
    };
    if std::str::from_utf8(&bytes).is_err() {
        crate::set_global_code(ctx, crate::U_ILLEGAL_ARGUMENT_ERROR);
        crate::set_message(ctx, None, "Input string must be exactly one UTF-8 encoded code point long.")?;
        return Ok(Value::Null);
    }
    let canonical = match CanonicalDecompositionBorrowed::new().decompose(c) {
        Decomposed::Default => None,
        Decomposed::Singleton(a) => Some(a.to_string()),
        Decomposed::Expansion(a, b) => Some(format!("{a}{b}")),
    };
    match form {
        FORM_D | FORM_C => Ok(canonical.map_or(Value::Null, s)),
        _ => {
            // compatibility forms: the raw mapping is the canonical one unless the
            // character has a compatibility decomposition of its own
            let nfd = DecomposingNormalizerBorrowed::new_nfd().normalize(&text).into_owned();
            let nfkd = DecomposingNormalizerBorrowed::new_nfkd().normalize(&text).into_owned();
            if nfd == nfkd || canonical.is_some() {
                return Ok(canonical.map_or(Value::Null, s));
            }
            // ICU4X does not expose the one-level compatibility mapping
            set_error(ctx, None, crate::U_UNSUPPORTED_ERROR, "normalizer_get_raw_decomposition: the raw compatibility decomposition is not available")?;
            Ok(Value::Null)
        }
    }
}

/// Whether U8_NEXT reads the whole of `b` as one (invalid) code point: a
/// single bad byte, or a truncated sequence.
fn one_invalid_sequence(b: &[u8]) -> bool {
    if b.len() == 1 {
        return true;
    }
    let need = match b[0] {
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => return false,
    };
    b.len() < need && b[1..].iter().all(|&c| c & 0xC0 == 0x80)
}
