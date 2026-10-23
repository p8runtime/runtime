//! Constant expressions in initializers (constants, parameter defaults,
//! property defaults): evaluated as PHP does, lazily, in the declaring
//! class's scope. Expected output is PHP 8.4's.

mod common;
use common::run_code_capture_output;

fn output(code: &str) -> String {
    run_code_capture_output(code).expect("script failed").1
}

#[test]
fn initializers_evaluate_expressions() {
    let out = output(
        r#"<?php
        const A = -1;
        const G = A * 10 . "x";
        class K {
            const B = -2;
            const C = "a" . "b";
            const D = self::B * 2;
            const L = [self::B, 'k' => PHP_INT_SIZE];
            public $p = [1, -1, self::C];
            public static $s = self::D + 1;
        }
        function f($x = -3, $y = PHP_INT_MAX, $z = K::B) { var_dump($x, $y, $z); }
        var_dump(A, G, K::B, K::C, K::D, K::L, (new K)->p, K::$s);
        f();
        "#,
    );
    assert_eq!(
        out,
        "int(-1)\nstring(4) \"-10x\"\nint(-2)\nstring(2) \"ab\"\nint(-4)\narray(2) {\n  [0]=>\n  int(-2)\n  [\"k\"]=>\n  int(8)\n}\n\
         array(3) {\n  [0]=>\n  int(1)\n  [1]=>\n  int(-1)\n  [2]=>\n  string(2) \"ab\"\n}\nint(-3)\n\
         int(-3)\nint(9223372036854775807)\nint(-2)\n"
    );
}

#[test]
fn defaults_are_evaluated_when_the_class_is_first_used() {
    // B does not exist when A is declared; A's static default is only
    // evaluated on first access.
    let out = output(
        r#"<?php
        class A { static $foo = B::foo; }
        class B { const foo = 7; }
        var_dump(A::$foo);
        "#,
    );
    assert_eq!(out, "int(7)\n");
}

#[test]
fn self_referencing_constants_are_an_error() {
    let out = output(
        r#"<?php
        class A { const X = self::Y; const Y = self::X; }
        try { echo A::X; } catch (Error $e) { echo get_class($e), ": ", substr($e->getMessage(), 0, 40), "\n"; }
        "#,
    );
    assert_eq!(out, "Error: Cannot declare self-referencing constant\n");
}
