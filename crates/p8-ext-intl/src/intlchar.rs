//! IntlChar (uchar/uchar.c) over icu_properties and icu_casemap: code
//! points, the general category and the classification functions as
//! uchar.cpp defines them, binary and enumerated properties by ICU's
//! UProperty numbers, simple case mappings, bidi mirroring, digits.
//!
//! Not available (null with U_UNSUPPORTED_ERROR): character names
//! (charName, charFromName, enumCharNames), charAge, property (value)
//! name lookups, getFC_NFKC_Closure, and the numeric values of characters
//! that are not decimal digits.

use crate::{R, U_ILLEGAL_ARGUMENT_ERROR, U_UNSUPPORTED_ERROR, arg};
use icu_collections::codepointtrie::TrieValue;
use icu_properties::props::{self, GeneralCategory as G};
use icu_properties::{CodePointMapData, CodePointSetData};
use p8_runtime::ext::{ClassSpec, Ctx, Throw};
use p8_runtime::object::Native;
use p8_runtime::value::Value;

macro_rules! statics {
    ($($name:literal => $func:ident;)*) => {
        &[$(($name, Some(Native::Static({
            fn m(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
                crate::prep(ctx, concat!("IntlChar::", $name), args)?;
                $func(ctx, args)
            }
            m
        }))),)*]
    };
}

pub static CLASSES: &[ClassSpec] = &[ClassSpec {
    name: "IntlChar",
    parent: None,
    interfaces: &[],
    flags: 0,
    properties: &[],
    methods: statics! {
        "chr" => chr;
        "ord" => ord;
        "hasBinaryProperty" => has_binary_property;
        "getIntPropertyValue" => get_int_property_value;
        "getIntPropertyMinValue" => get_int_property_min_value;
        "getIntPropertyMaxValue" => get_int_property_max_value;
        "getNumericValue" => get_numeric_value;
        "enumCharTypes" => enum_char_types;
        "getBlockCode" => get_block_code;
        "charName" => unsupported_null;
        "charFromName" => unsupported_null;
        "enumCharNames" => unsupported_false;
        "getPropertyName" => unsupported_false;
        "getPropertyEnum" => unsupported_int;
        "getPropertyValueName" => unsupported_false;
        "getPropertyValueEnum" => unsupported_int;
        "foldCase" => fold_case;
        "digit" => digit;
        "forDigit" => for_digit;
        "charAge" => unsupported_null;
        "getUnicodeVersion" => get_unicode_version;
        "getFC_NFKC_Closure" => unsupported_null;
        "isUAlphabetic" => is_ualphabetic;
        "isULowercase" => is_ulowercase;
        "isUUppercase" => is_uuppercase;
        "isUWhiteSpace" => is_uwhitespace;
        "islower" => islower;
        "isupper" => isupper;
        "istitle" => istitle;
        "isdigit" => isdigit;
        "isalpha" => isalpha;
        "isalnum" => isalnum;
        "isxdigit" => isxdigit;
        "ispunct" => ispunct;
        "isgraph" => isgraph;
        "isblank" => isblank;
        "isdefined" => isdefined;
        "isspace" => isspace;
        "isJavaSpaceChar" => is_java_space_char;
        "isWhitespace" => is_whitespace;
        "iscntrl" => iscntrl;
        "isISOControl" => is_iso_control;
        "isprint" => isprint;
        "isbase" => isbase;
        "isMirrored" => is_mirrored;
        "isIDStart" => is_id_start;
        "isIDPart" => is_id_part;
        "isIDIgnorable" => is_id_ignorable;
        "isJavaIDStart" => is_java_id_start;
        "isJavaIDPart" => is_java_id_part;
        "charDirection" => char_direction;
        "charType" => char_type;
        "getCombiningClass" => get_combining_class;
        "charDigitValue" => char_digit_value;
        "charMirror" => char_mirror;
        "tolower" => tolower;
        "toupper" => toupper;
        "totitle" => totitle;
        "getBidiPairedBracket" => get_bidi_paired_bracket;
    },
    constants: crate::consts::INTL_CHAR,
    debug_info: None,
}];

/// convert_cp: an int code point or a one-code-point UTF-8 string; None
/// (the global error set) otherwise. The flag says whether it was a string.
fn cp(ctx: &mut dyn Ctx, v: &Value) -> Result<Option<(u32, bool)>, Throw> {
    crate::reset_global(ctx);
    let (n, is_str) = match v {
        Value::Str(st) => {
            let b = st.as_bytes();
            let text = std::str::from_utf8(b).ok();
            let mut chars = text.map(|t| t.chars());
            match chars.as_mut().map(|c| (c.next(), c.next())) {
                Some((Some(c), None)) => (c as i64, true),
                _ => {
                    crate::set_global_code(ctx, U_ILLEGAL_ARGUMENT_ERROR);
                    crate::set_message(ctx, None, "Passing a UTF-8 character for codepoint requires a string which is exactly one UTF-8 codepoint long.")?;
                    return Ok(None);
                }
            }
        }
        other => (other.to_int(), false),
    };
    if !(0..=0x10FFFF).contains(&n) {
        crate::set_global_code(ctx, U_ILLEGAL_ARGUMENT_ERROR);
        crate::set_message(ctx, None, "Codepoint out of range")?;
        return Ok(None);
    }
    Ok(Some((n as u32, is_str)))
}

fn gc(c: u32) -> G {
    CodePointMapData::<G>::new().get32(c)
}

fn is_letter(g: G) -> bool {
    matches!(g, G::UppercaseLetter | G::LowercaseLetter | G::TitlecaseLetter | G::ModifierLetter | G::OtherLetter)
}

fn is_punct(g: G) -> bool {
    matches!(g, G::DashPunctuation | G::OpenPunctuation | G::ClosePunctuation | G::ConnectorPunctuation | G::OtherPunctuation | G::InitialPunctuation | G::FinalPunctuation)
}

fn is_sep(g: G) -> bool {
    matches!(g, G::SpaceSeparator | G::LineSeparator | G::ParagraphSeparator)
}

fn set<P: icu_properties::props::BinaryProperty>(c: u32) -> bool {
    CodePointSetData::new::<P>().contains32(c)
}

fn is_iso_control_cp(c: u32) -> bool {
    c <= 0x1F || (0x7F..=0x9F).contains(&c)
}

fn is_id_ignorable_cp(c: u32) -> bool {
    if c <= 0x9F { is_iso_control_cp(c) && !((9..=0xD).contains(&c) || (0x1C..=0x1F).contains(&c)) } else { gc(c) == G::Format }
}

macro_rules! bool_methods {
    ($($name:ident => |$c:ident| $body:expr;)*) => {
        $(fn $name(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
            let Some(($c, _)) = cp(ctx, &arg(args, 0))? else { return Ok(Value::Null) };
            Ok(Value::Bool($body))
        })*
    };
}

bool_methods! {
    is_ualphabetic => |c| set::<props::Alphabetic>(c);
    is_ulowercase => |c| set::<props::Lowercase>(c);
    is_uuppercase => |c| set::<props::Uppercase>(c);
    is_uwhitespace => |c| set::<props::WhiteSpace>(c);
    islower => |c| gc(c) == G::LowercaseLetter;
    isupper => |c| gc(c) == G::UppercaseLetter;
    istitle => |c| gc(c) == G::TitlecaseLetter;
    isdigit => |c| gc(c) == G::DecimalNumber;
    isalpha => |c| is_letter(gc(c));
    isalnum => |c| is_letter(gc(c)) || gc(c) == G::DecimalNumber;
    isxdigit => |c| matches!(c, 0x30..=0x39 | 0x41..=0x46 | 0x61..=0x66 | 0xFF21..=0xFF26 | 0xFF41..=0xFF46) || gc(c) == G::DecimalNumber;
    ispunct => |c| is_punct(gc(c));
    isgraph => |c| !matches!(gc(c), G::Control | G::Format | G::Surrogate | G::Unassigned) && !is_sep(gc(c));
    isblank => |c| if c <= 0x9F { c == 9 || c == 0x20 } else { gc(c) == G::SpaceSeparator };
    isdefined => |c| gc(c) != G::Unassigned;
    isspace => |c| if c <= 0x9F { c == 0x20 || (9..=0xD).contains(&c) || (0x1C..=0x1F).contains(&c) } else { is_sep(gc(c)) };
    is_java_space_char => |c| is_sep(gc(c));
    is_whitespace => |c| if c <= 0x9F { c == 0x20 || (9..=0xD).contains(&c) || (0x1C..=0x1F).contains(&c) } else { is_sep(gc(c)) && c != 0xA0 && c != 0x2007 && c != 0x202F };
    iscntrl => |c| matches!(gc(c), G::Control | G::Format | G::LineSeparator | G::ParagraphSeparator);
    is_iso_control => |c| is_iso_control_cp(c);
    isprint => |c| !matches!(gc(c), G::Control | G::Format | G::Surrogate | G::PrivateUse | G::Unassigned);
    isbase => |c| is_letter(gc(c)) || matches!(gc(c), G::DecimalNumber | G::LetterNumber | G::OtherNumber | G::SpacingMark | G::EnclosingMark);
    is_mirrored => |c| set::<props::BidiMirrored>(c);
    is_id_start => |c| is_letter(gc(c)) || gc(c) == G::LetterNumber;
    is_id_part => |c| is_letter(gc(c)) || matches!(gc(c), G::LetterNumber | G::SpacingMark | G::NonspacingMark | G::DecimalNumber | G::ConnectorPunctuation) || is_id_ignorable_cp(c);
    is_id_ignorable => |c| is_id_ignorable_cp(c);
    is_java_id_start => |c| is_letter(gc(c)) || matches!(gc(c), G::CurrencySymbol | G::ConnectorPunctuation);
    is_java_id_part => |c| is_letter(gc(c)) || matches!(gc(c), G::CurrencySymbol | G::ConnectorPunctuation | G::DecimalNumber | G::LetterNumber | G::SpacingMark | G::NonspacingMark) || is_id_ignorable_cp(c);
}

fn chr(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Some((c, _)) = cp(ctx, &arg(args, 0))? else { return Ok(Value::Null) };
    Ok(Value::Str(p8_runtime::string::PhpStr::new(encode(c))))
}

/// UTF-8 for any code point (surrogates included, as U8_APPEND_UNSAFE writes them).
fn encode(c: u32) -> Vec<u8> {
    match char::from_u32(c) {
        Some(ch) => ch.to_string().into_bytes(),
        None => vec![0xE0 | (c >> 12) as u8, 0x80 | ((c >> 6) & 0x3F) as u8, 0x80 | (c & 0x3F) as u8],
    }
}

fn ord(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Some((c, _)) = cp(ctx, &arg(args, 0))? else { return Ok(Value::Null) };
    Ok(Value::Int(c as i64))
}

/// A mapped code point: a string when the input was one.
fn char_result(c: u32, as_str: bool) -> Value {
    if as_str { Value::Str(p8_runtime::string::PhpStr::new(encode(c))) } else { Value::Int(c as i64) }
}

fn map_char(ctx: &mut dyn Ctx, args: &[Value], f: impl Fn(char) -> char) -> R {
    let Some((c, as_str)) = cp(ctx, &arg(args, 0))? else { return Ok(Value::Null) };
    let out = char::from_u32(c).map_or(c, |ch| f(ch) as u32);
    Ok(char_result(out, as_str))
}

fn tolower(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    map_char(ctx, args, |c| icu_casemap::CaseMapperBorrowed::new().simple_lowercase(c))
}

fn toupper(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    map_char(ctx, args, |c| icu_casemap::CaseMapperBorrowed::new().simple_uppercase(c))
}

fn totitle(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    map_char(ctx, args, |c| icu_casemap::CaseMapperBorrowed::new().simple_titlecase(c))
}

fn fold_case(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    // U_FOLD_CASE_EXCLUDE_SPECIAL_I = 1: the Turkic folding of I and İ
    let turkic = args.len() > 1 && arg(args, 1).to_int() & 1 != 0;
    map_char(ctx, args, |c| {
        let m = icu_casemap::CaseMapperBorrowed::new();
        if turkic { m.simple_fold_turkic(c) } else { m.simple_fold(c) }
    })
}

fn char_mirror(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    map_char(ctx, args, |c| CodePointMapData::<icu_properties::props::BidiMirroringGlyph>::new().get(c).mirroring_glyph.unwrap_or(c))
}

fn get_bidi_paired_bracket(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    map_char(ctx, args, |c| {
        let g = CodePointMapData::<icu_properties::props::BidiMirroringGlyph>::new().get(c);
        if g.paired_bracket_type != icu_properties::props::BidiPairedBracketType::None { g.mirroring_glyph.unwrap_or(c) } else { c }
    })
}

fn char_type(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Some((c, _)) = cp(ctx, &arg(args, 0))? else { return Ok(Value::Null) };
    Ok(Value::Int(gc(c).to_u32() as i64))
}

fn char_direction(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Some((c, _)) = cp(ctx, &arg(args, 0))? else { return Ok(Value::Null) };
    Ok(Value::Int(CodePointMapData::<props::BidiClass>::new().get32(c).to_u32() as i64))
}

fn get_combining_class(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Some((c, _)) = cp(ctx, &arg(args, 0))? else { return Ok(Value::Null) };
    Ok(Value::Int(CodePointMapData::<props::CanonicalCombiningClass>::new().get32(c).to_u32() as i64))
}

fn get_block_code(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Some((c, _)) = cp(ctx, &arg(args, 0))? else { return Ok(Value::Null) };
    Ok(Value::Int(CodePointMapData::<props::Block>::new().get32(c).to_u32() as i64))
}

/// A decimal digit's value: its place in its run of Nd characters.
fn decimal_value(c: u32) -> Option<u32> {
    if gc(c) != G::DecimalNumber {
        return None;
    }
    let mut start = c;
    while start > 0 && gc(start - 1) == G::DecimalNumber && c - start < 9 + 50 {
        start -= 1;
    }
    Some((c - start) % 10)
}

fn char_digit_value(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Some((c, _)) = cp(ctx, &arg(args, 0))? else { return Ok(Value::Null) };
    Ok(Value::Int(decimal_value(c).map_or(-1, |d| d as i64)))
}

fn get_numeric_value(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Some((c, _)) = cp(ctx, &arg(args, 0))? else { return Ok(Value::Null) };
    if let Some(d) = decimal_value(c) {
        return Ok(Value::Float(d as f64));
    }
    if CodePointMapData::<props::NumericType>::new().get32(c) == props::NumericType::None {
        // U_NO_NUMERIC_VALUE
        return Ok(Value::Float(-123456789.0));
    }
    crate::set_error(ctx, None, U_UNSUPPORTED_ERROR, "IntlChar::getNumericValue: the numeric value of this character is not available")?;
    Ok(Value::Null)
}

/// u_digit: the value in `radix`, decimal digits and Latin letters (ASCII and fullwidth).
fn digit_value(c: u32, radix: i64) -> Option<i64> {
    if !(2..=36).contains(&radix) {
        return None;
    }
    let v = if let Some(d) = decimal_value(c) {
        d as i64
    } else {
        match c {
            0x41..=0x5A => (c - 0x41 + 10) as i64,
            0x61..=0x7A => (c - 0x61 + 10) as i64,
            0xFF21..=0xFF3A => (c - 0xFF21 + 10) as i64,
            0xFF41..=0xFF5A => (c - 0xFF41 + 10) as i64,
            _ => return None,
        }
    };
    (v < radix).then_some(v)
}

fn digit(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Some((c, _)) = cp(ctx, &arg(args, 0))? else { return Ok(Value::Null) };
    let radix = if args.len() > 1 { arg(args, 1).to_int() } else { 10 };
    match digit_value(c, radix) {
        Some(v) => Ok(Value::Int(v)),
        None => {
            crate::set_global_code(ctx, U_ILLEGAL_ARGUMENT_ERROR);
            crate::set_message(ctx, None, "Invalid digit")?;
            Ok(Value::Bool(false))
        }
    }
}

fn for_digit(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    crate::reset_global(ctx);
    let d = arg(args, 0).to_int();
    let radix = if args.len() > 1 { arg(args, 1).to_int() } else { 10 };
    if !(2..=36).contains(&radix) || d < 0 || d >= radix {
        return Ok(Value::Int(0));
    }
    Ok(Value::Int(if d < 10 { 0x30 + d } else { 0x61 + d - 10 }))
}

fn enum_char_types(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let callback = arg(args, 0);
    let map = CodePointMapData::<G>::new();
    for r in map.iter_ranges() {
        let mut a = [Value::Int(*r.range.start() as i64), Value::Int(*r.range.end() as i64 + 1), Value::Int(r.value.to_u32() as i64)];
        ctx.call(&callback, &mut a)?;
    }
    Ok(Value::Null)
}

/// u_hasBinaryProperty by ICU's UProperty number.
#[allow(deprecated)] // ICU still answers for Case_Sensitive, the *_Inert and Segment_Starter properties ICU4X deprecates
fn binary(prop: i64, c: u32) -> Option<bool> {
    use props::*;
    Some(match prop {
        0 => set::<Alphabetic>(c),
        1 => set::<AsciiHexDigit>(c),
        2 => set::<BidiControl>(c),
        3 => set::<BidiMirrored>(c),
        4 => set::<Dash>(c),
        5 => set::<DefaultIgnorableCodePoint>(c),
        6 => set::<Deprecated>(c),
        7 => set::<Diacritic>(c),
        8 => set::<Extender>(c),
        9 => set::<FullCompositionExclusion>(c),
        10 => set::<GraphemeBase>(c),
        11 => set::<GraphemeExtend>(c),
        12 => set::<GraphemeLink>(c),
        13 => set::<HexDigit>(c),
        14 => set::<Hyphen>(c),
        15 => set::<IdContinue>(c),
        16 => set::<IdStart>(c),
        17 => set::<Ideographic>(c),
        18 => set::<IdsBinaryOperator>(c),
        19 => set::<IdsTrinaryOperator>(c),
        20 => set::<JoinControl>(c),
        21 => set::<LogicalOrderException>(c),
        22 => set::<Lowercase>(c),
        23 => set::<Math>(c),
        24 => set::<NoncharacterCodePoint>(c),
        25 => set::<QuotationMark>(c),
        26 => set::<Radical>(c),
        27 => set::<SoftDotted>(c),
        28 => set::<TerminalPunctuation>(c),
        29 => set::<UnifiedIdeograph>(c),
        30 => set::<Uppercase>(c),
        31 => set::<WhiteSpace>(c),
        32 => set::<XidContinue>(c),
        33 => set::<XidStart>(c),
        34 => set::<CaseSensitive>(c),
        35 => set::<SentenceTerminal>(c),
        36 => set::<VariationSelector>(c),
        37 => set::<NfdInert>(c),
        38 => set::<NfkdInert>(c),
        39 => set::<NfcInert>(c),
        40 => set::<NfkcInert>(c),
        41 => set::<SegmentStarter>(c),
        42 => set::<PatternSyntax>(c),
        43 => set::<PatternWhiteSpace>(c),
        44 => set::<Alnum>(c),
        45 => set::<Blank>(c),
        46 => set::<Graph>(c),
        47 => set::<Print>(c),
        48 => set::<Xdigit>(c),
        49 => set::<Cased>(c),
        50 => set::<CaseIgnorable>(c),
        51 => set::<ChangesWhenLowercased>(c),
        52 => set::<ChangesWhenUppercased>(c),
        53 => set::<ChangesWhenTitlecased>(c),
        54 => set::<ChangesWhenCasefolded>(c),
        55 => set::<ChangesWhenCasemapped>(c),
        56 => set::<ChangesWhenNfkcCasefolded>(c),
        57 => set::<Emoji>(c),
        58 => set::<EmojiPresentation>(c),
        59 => set::<EmojiModifier>(c),
        60 => set::<EmojiModifierBase>(c),
        61 => set::<EmojiComponent>(c),
        62 => set::<RegionalIndicator>(c),
        63 => set::<PrependedConcatenationMark>(c),
        64 => set::<ExtendedPictographic>(c),
        72 => set::<IdsUnaryOperator>(c),
        73 => set::<IdCompatMathStart>(c),
        74 => set::<IdCompatMathContinue>(c),
        75 => set::<ModifierCombiningMark>(c),
        _ => return None,
    })
}

fn has_binary_property(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Some((c, _)) = cp(ctx, &arg(args, 0))? else { return Ok(Value::Null) };
    Ok(Value::Bool(binary(arg(args, 1).to_int(), c).unwrap_or(false)))
}

/// u_getIntPropertyValue by UProperty number; None for a property not available here.
fn int_value(prop: i64, c: u32) -> Option<i64> {
    if let Some(b) = binary(prop, c) {
        return Some(b as i64);
    }
    let v = match prop {
        0x1000 => CodePointMapData::<props::BidiClass>::new().get32(c).to_u32(),
        0x1001 => CodePointMapData::<props::Block>::new().get32(c).to_u32(),
        0x1002 => CodePointMapData::<props::CanonicalCombiningClass>::new().get32(c).to_u32(),
        0x1004 => CodePointMapData::<props::EastAsianWidth>::new().get32(c).to_u32(),
        0x1005 => gc(c).to_u32(),
        0x1006 => CodePointMapData::<props::JoiningGroup>::new().get32(c).to_u32(),
        0x1007 => CodePointMapData::<props::JoiningType>::new().get32(c).to_u32(),
        0x1008 => CodePointMapData::<props::LineBreak>::new().get32(c).to_u32(),
        0x1009 => CodePointMapData::<props::NumericType>::new().get32(c).to_u32(),
        0x100A => CodePointMapData::<props::Script>::new().get32(c).to_u32(),
        0x100B => CodePointMapData::<props::HangulSyllableType>::new().get32(c).to_u32(),
        0x1012 => CodePointMapData::<props::GraphemeClusterBreak>::new().get32(c).to_u32(),
        0x1013 => CodePointMapData::<props::SentenceBreak>::new().get32(c).to_u32(),
        0x1014 => CodePointMapData::<props::WordBreak>::new().get32(c).to_u32(),
        0x1015 => match CodePointMapData::<props::BidiMirroringGlyph>::new().get32(c).paired_bracket_type {
            props::BidiPairedBracketType::Open => 1,
            props::BidiPairedBracketType::Close => 2,
            _ => 0,
        },
        0x1016 => CodePointMapData::<props::IndicSyllabicCategory>::new().get32(c).to_u32(),
        0x1018 => CodePointMapData::<props::VerticalOrientation>::new().get32(c).to_u32(),
        0x2000 => 1 << gc(c).to_u32(),
        _ => return None,
    };
    Some(v as i64)
}

fn get_int_property_value(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Some((c, _)) = cp(ctx, &arg(args, 0))? else { return Ok(Value::Null) };
    match int_value(arg(args, 1).to_int(), c) {
        Some(v) => Ok(Value::Int(v)),
        None => {
            crate::set_error(ctx, None, U_UNSUPPORTED_ERROR, "IntlChar::getIntPropertyValue: this property is not available")?;
            Ok(Value::Null)
        }
    }
}

fn get_int_property_min_value(_: &mut dyn Ctx, _: &mut [Value]) -> R {
    Ok(Value::Int(0))
}

fn get_int_property_max_value(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let prop = arg(args, 0).to_int();
    let max = match prop {
        0..=75 => 1,
        0x1000 => 22,
        0x1001 => 338,
        0x1002 => 254,
        0x1004 => 5,
        0x1005 => 29,
        0x1007 => 5,
        0x1009 => 3,
        0x100B => 5,
        0x1015 => 2,
        _ => {
            crate::set_error(ctx, None, U_UNSUPPORTED_ERROR, "IntlChar::getIntPropertyMaxValue: this property is not available")?;
            return Ok(Value::Null);
        }
    };
    Ok(Value::Int(max))
}

/// IntlChar::getUnicodeVersion() (u_getUnicodeVersion): the version of the
/// Unicode data the properties come from, IntlChar::UNICODE_VERSION's, as
/// its four numbers.
fn get_unicode_version(_: &mut dyn Ctx, _: &mut [Value]) -> R {
    let version = crate::consts::INTL_CHAR.iter().find_map(|(name, c)| match (*name, c) {
        ("UNICODE_VERSION", p8_runtime::ext::Constant::Str(v)) => Some(*v),
        _ => None,
    });
    let mut parts: Vec<Value> = version.unwrap_or_default().split(|&b| b == b'.').map(|n| Value::Int(String::from_utf8_lossy(n).parse().unwrap_or(0))).collect();
    parts.resize(4, Value::Int(0));
    Ok(Value::Array(std::rc::Rc::new(p8_runtime::array::PhpArray::from_list(parts))))
}

fn unsupported(ctx: &mut dyn Ctx, ret: Value) -> R {
    crate::reset_global(ctx);
    crate::set_error(ctx, None, U_UNSUPPORTED_ERROR, "IntlChar: character names, ages and property names are not available")?;
    Ok(ret)
}

fn unsupported_null(ctx: &mut dyn Ctx, _: &mut [Value]) -> R {
    unsupported(ctx, Value::Null)
}

fn unsupported_false(ctx: &mut dyn Ctx, _: &mut [Value]) -> R {
    unsupported(ctx, Value::Bool(false))
}

fn unsupported_int(ctx: &mut dyn Ctx, _: &mut [Value]) -> R {
    // UCHAR_INVALID_CODE / UCHAR_INVALID_ENUM
    unsupported(ctx, Value::Int(-1))
}
