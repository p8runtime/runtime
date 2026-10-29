#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! p8's mbstring extension: multibyte string functions over UTF-8, the
//! Unicode encodings and the single-byte code pages, with php-src's own
//! case-mapping, width and detection tables so results are PHP's.
//!
//! Not here: the CJK multibyte encodings (SJIS, EUC-JP, ISO-2022-JP, GB18030,
//! BIG5, EUC-KR ...), mb_convert_kana, mb_send_mail and the mb_ereg family.
//!
//! Reference: $PHP_SRC_PATH/ext/mbstring/mbstring.c,
//! $PHP_SRC_PATH/ext/mbstring/php_unicode.c, $PHP_SRC_PATH/ext/mbstring/libmbfl/

mod enc;
mod funcs;
mod gendata;
mod mime;
mod sbtables;
mod transfer;
mod unicode;
mod unidata;
mod utf7;

use p8_runtime::ext::{Constant, Extension, Registry};

pub const EXTENSION: Extension = Extension { name: "mbstring", register };

fn register(registry: &mut Registry) {
    registry.add_functions(funcs::BUILTINS);
    // The engine only tells a builtin its own name (for messages) when it
    // may reenter; mb_parse_str() also runs parse_str() for the caller.
    let names: Vec<&str> = funcs::BUILTINS.iter().map(|f| f.name).collect();
    registry.mark_reenters(&names);
    for (name, v) in [
        (&b"MB_CASE_UPPER"[..], 0),
        (b"MB_CASE_LOWER", 1),
        (b"MB_CASE_TITLE", 2),
        (b"MB_CASE_FOLD", 3),
        (b"MB_CASE_UPPER_SIMPLE", 4),
        (b"MB_CASE_LOWER_SIMPLE", 5),
        (b"MB_CASE_TITLE_SIMPLE", 6),
        (b"MB_CASE_FOLD_SIMPLE", 7),
    ] {
        registry.add_constant(name, Constant::Int(v));
    }
    for (name, default) in [
        ("mbstring.language", Some(&b"neutral"[..])),
        ("mbstring.detect_order", None),
        ("mbstring.http_input", None),
        ("mbstring.http_output", None),
        ("mbstring.internal_encoding", None),
        ("mbstring.substitute_character", None),
        ("mbstring.encoding_translation", Some(&b"0"[..])),
        ("mbstring.http_output_conv_mimetypes", Some(&b"^(text/|application/xhtml\\+xml)"[..])),
        ("mbstring.strict_detection", Some(&b"0"[..])),
        ("mbstring.regex_retry_limit", Some(&b"1000000"[..])),
        ("mbstring.regex_stack_limit", Some(&b"100000"[..])),
    ] {
        registry.add_ini(name, default);
    }
    registry.add_module("mbstring");
}
