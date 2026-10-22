//! Request and engine information functions, casts, and switch/loop stack
//! handling. Expected output is PHP 8.4's for the same code.

mod common;
use common::run_code_capture_output;

fn output(code: &str) -> String {
    run_code_capture_output(code).expect("script failed").1
}

#[test]
fn version_and_type_information() {
    let out = output(
        r#"<?php
        var_dump(phpversion(), phpversion('json'), phpversion('nope'), php_sapi_name(), zend_version(),
            get_debug_type(1.5), get_debug_type(new stdClass), get_debug_type(new class {}), get_debug_type(null));
        "#,
    );
    assert_eq!(
        out,
        "string(6) \"8.4.27\"\nstring(6) \"8.4.27\"\nbool(false)\nstring(3) \"cli\"\nstring(6) \"4.4.27\"\n\
         string(5) \"float\"\nstring(8) \"stdClass\"\nstring(15) \"class@anonymous\"\nstring(4) \"null\"\n"
    );
}

#[test]
fn backtraces_and_defined_vars() {
    let out = output(
        r#"<?php
        function f($q) { $a = 1; var_dump(array_keys(get_defined_vars())); $t = debug_backtrace();
            echo count($t), " ", $t[1]['class'], $t[1]['type'], $t[1]['function'], " ", get_class($t[1]['object']), "\n";
            debug_print_backtrace(); }
        class C { function m() { f(1); } static function s() { (new C)->m(); } }
        C::s();
        "#,
    );
    assert_eq!(
        out,
        "array(2) {\n  [0]=>\n  string(1) \"q\"\n  [1]=>\n  string(1) \"a\"\n}\n3 C->m C\n\
         #0 (5): f(1)\n#1 (5): C->m()\n#2 (6): C::s()\n"
    );
}

#[test]
fn settype_and_include_path() {
    let out = output(
        r#"<?php
        $x = "12abc"; settype($x, "integer"); var_dump($x);
        settype($x, "array"); var_dump($x);
        try { settype($x, "nope"); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
        set_include_path("/x"); var_dump(get_include_path());
        "#,
    );
    assert_eq!(
        out,
        "int(12)\narray(1) {\n  [0]=>\n  int(12)\n}\nsettype(): Argument #2 ($type) must be a valid type\nstring(2) \"/x\"\n"
    );
}

#[test]
fn casts_follow_php() {
    let out = output(
        r#"<?php
        class A { public $a = 1; protected $b = 2; private $c = 3; }
        var_dump(array_map('bin2hex', array_keys((array)new A)));
        var_dump((int)"12abc", (float)"1.5e3x", (int)[1], (int)1e20, (int)NAN, (int)-9223372036854775809);
        "#,
    );
    assert_eq!(
        out,
        "array(3) {\n  [0]=>\n  string(2) \"61\"\n  [1]=>\n  string(8) \"002a0062\"\n  [2]=>\n  string(8) \"00410063\"\n}\n\
         int(12)\nfloat(1500)\nint(1)\nint(7766279631452241920)\nint(0)\nint(-9223372036854775808)\n"
    );
}

#[test]
fn switch_and_loop_exits_keep_the_stack_straight() {
    let out = output(
        r#"<?php
        function ops() { throw new Exception(); }
        foreach ([[1], [2]] as $b) {
            switch ($b[0]) { default: try { $r = [1] + ops(); } catch (Exception $e) { echo "caught "; } }
        }
        foreach ([1, 2, 3] as $a) { foreach ([1, 2] as $c) { if ($c == 2) continue 2; if ($a == 3) break 2; echo "$a$c "; } }
        while (true) { foreach ([1] as $q) { switch ($q) { case 1: break 3; } } }
        echo "out\n";
        "#,
    );
    assert_eq!(out, "caught caught 11 21 out\n");
}
