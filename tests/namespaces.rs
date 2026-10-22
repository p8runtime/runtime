//! Names resolved at compile time against the namespace and `use` imports
//! (zend_resolve_class_name and friends), and top-level declarations that
//! exist before the first statement runs.

mod common;
use common::run_code_capture_output;

fn output(code: &str) -> String {
    run_code_capture_output(code).expect("script failed").1
}

#[test]
fn declarations_and_references_take_the_namespace() {
    let out = output(
        r#"<?php
        namespace App\Models;
        const VERSION = 2;
        interface HasName { function name(): string; }
        class User implements HasName { function name(): string { return __METHOD__; } }
        function helper() { return __FUNCTION__; }
        var_dump(User::class, get_class(new User), (new User)->name(), helper(), __NAMESPACE__);
        var_dump(VERSION, \App\Models\VERSION, namespace\VERSION, new User instanceof HasName);
        "#,
    );
    assert_eq!(
        out,
        "string(15) \"App\\Models\\User\"\nstring(15) \"App\\Models\\User\"\n\
         string(21) \"App\\Models\\User::name\"\nstring(17) \"App\\Models\\helper\"\n\
         string(10) \"App\\Models\"\nint(2)\nint(2)\nint(2)\nbool(true)\n"
    );
}

#[test]
fn unqualified_functions_and_constants_fall_back_to_global() {
    let out = output(
        r#"<?php
        namespace App;
        function strlen($s) { return -1; }
        var_dump(strlen("abc"), \strlen("abc"), strtoupper("x"), PHP_INT_SIZE, E_ALL === \E_ALL);
        "#,
    );
    assert_eq!(out, "int(-1)\nint(3)\nstring(1) \"X\"\nint(8)\nbool(true)\n");
}

#[test]
fn use_imports_classes_functions_and_constants() {
    let out = output(
        r#"<?php
        namespace Lib { class Thing {} function make() { return new Thing; } const LIMIT = 5; }
        namespace App {
            use Lib\Thing as T;
            use Lib as L;
            use function Lib\make;
            use const Lib\LIMIT;
            use InvalidArgumentException as IAE;
            var_dump(get_class(make()), T::class, L\Thing::class, LIMIT, new T instanceof L\Thing);
            try { throw new IAE("x"); } catch (\InvalidArgumentException $e) { echo get_class($e), "\n"; }
        }
        "#,
    );
    assert_eq!(
        out,
        "string(9) \"Lib\\Thing\"\nstring(9) \"Lib\\Thing\"\nstring(9) \"Lib\\Thing\"\nint(5)\nbool(true)\nInvalidArgumentException\n"
    );
}

#[test]
fn braced_namespace_bodies_run() {
    let out = output(
        r#"<?php
        namespace A { function f() { return __FUNCTION__; } echo f(), "\n"; }
        namespace { echo A\f(), " ", \A\f(), "\n"; }
        "#,
    );
    assert_eq!(out, "A\\f\nA\\f A\\f\n");
}

#[test]
fn top_level_functions_and_classes_exist_before_their_statement() {
    let out = output(
        r#"<?php
        echo early(), " ", get_class(new Early), "\n";
        function early() { return "function"; }
        class Early {}
        function outer() { return function_exists('inner'); function inner() {} }
        var_dump(outer(), function_exists('cond'));
        if (true) { function cond() {} }
        "#,
    );
    assert_eq!(out, "function Early\nbool(false)\nbool(false)\n");
}

#[test]
fn instanceof_a_variable_class_name() {
    let out = output(r#"<?php $c = 'stdClass'; var_dump(new stdClass instanceof $c);"#);
    assert_eq!(out, "bool(true)\n");
}
