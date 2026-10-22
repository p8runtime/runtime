//! User function parameter and return types: an exact match passes,
//! scalars coerce as zend_verify_scalar_type_hint allows in weak mode
//! (int, then float, string, bool), strict mode only widens int to float,
//! and anything else is PHP's TypeError. Expected output is PHP 8.4's.

mod common;
use common::run_code_capture_output;

fn output(code: &str) -> String {
    run_code_capture_output(code).expect("script failed").1
}

#[test]
fn weak_mode_coerces_scalars_in_php_order() {
    let out = output(
        r#"<?php
        function i(int $x) { var_dump($x); }
        function f(float $x) { var_dump($x); }
        function s(string $x) { var_dump($x); }
        function b(bool $x) { var_dump($x); }
        function u(int|string $x) { var_dump($x); }
        function n(?int $x) { var_dump($x); }
        function r(): int { return "12"; }
        i("5"); i(5.0); i(true); f(3); f("1e2"); s(12); s(1.5); b(0); @u(1.5); u("1.5"); n(null); var_dump(r());
        "#,
    );
    assert_eq!(
        out,
        "int(5)\nint(5)\nint(1)\nfloat(3)\nfloat(100)\nstring(2) \"12\"\nstring(3) \"1.5\"\nbool(false)\n\
         int(1)\nstring(3) \"1.5\"\nNULL\nint(12)\n"
    );
}

#[test]
fn mismatches_are_type_errors() {
    let out = output(
        r#"<?php
        function i(int $x) {}
        function s(string $x) {}
        function o(stdClass $o) {}
        function r(): int { return "x"; }
        foreach ([fn() => i("abc"), fn() => i(null), fn() => i([]), fn() => o(new Exception), fn() => s(new stdClass), fn() => r()] as $c) {
            try { $c(); } catch (TypeError $e) { echo preg_replace('/, called in .*/', ', called in F', $e->getMessage()), "\n"; }
        }
        "#,
    );
    assert_eq!(
        out,
        "i(): Argument #1 ($x) must be of type int, string given, called in F\n\
         i(): Argument #1 ($x) must be of type int, null given, called in F\n\
         i(): Argument #1 ($x) must be of type int, array given, called in F\n\
         o(): Argument #1 ($o) must be of type stdClass, Exception given, called in F\n\
         s(): Argument #1 ($x) must be of type string, stdClass given, called in F\n\
         r(): Return value must be of type int, string returned\n"
    );
}

#[test]
fn strict_mode_only_widens_int_to_float() {
    let out = output(
        r#"<?php
        declare(strict_types=1);
        function f(float $x) { var_dump($x); }
        function i(int $x) {}
        f(2);
        try { i("1"); } catch (TypeError $e) { echo substr($e->getMessage(), 0, 55), "\n"; }
        "#,
    );
    assert_eq!(out, "float(2)\ni(): Argument #1 ($x) must be of type int, string given\n");
}

#[test]
fn fractional_floats_to_int_are_deprecated() {
    let out = output(r#"<?php function i(int $x) { var_dump($x); } i(1.5);"#);
    assert!(out.contains("Deprecated: Implicit conversion from float 1.5 to int loses precision"), "{out}");
    assert!(out.ends_with("int(1)\n"), "{out}");
}
