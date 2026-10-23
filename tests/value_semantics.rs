//! Values, slots and identity, as PHP 8.4 has them: object ids, ++/--,
//! static initializers, copies on store and return, references in arrays.

mod common;
use common::run_code_capture_output;

fn output(code: &str) -> String {
    run_code_capture_output(code).expect("script failed").1
}

#[test]
fn object_ids_count_from_one_in_creation_order() {
    let out = output(
        r#"<?php
        class A {} class B {}
        $a = new A; $b = new B; $c = clone $a;
        var_dump($a, spl_object_id($b), $c);
        "#,
    );
    assert_eq!(out, "object(A)#1 (0) {\n}\nint(2)\nobject(A)#3 (0) {\n}\n");
}

#[test]
fn increment_and_decrement_follow_php() {
    let out = output(
        r#"<?php
        $s = "a"; $s++; $z = "Az"; $z++; $f = 1.5; $f++; $n = null; $n++; $num = "9"; ++$num;
        var_dump($s, $z, $f, $n, $num);
        $a = ["x" => 1]; $b = $a; $a["x"]++; $a["y"]["z"] = 1; $a["y"]["z"]--;
        var_dump($a["x"], $b["x"], $a["y"]["z"]);
        $o = new stdClass; $o->p = [1]; $o->p[0] += 5; $o->p[0]++;
        var_dump($o->p[0]);
        "#,
    );
    assert_eq!(
        out,
        "string(1) \"b\"\nstring(2) \"Ba\"\nfloat(2.5)\nint(1)\nint(10)\nint(2)\nint(1)\nint(0)\nint(7)\n"
    );
}

#[test]
fn stored_values_are_copies() {
    let out = output(
        r#"<?php
        $v = 1; $arr = [$v]; $o = new stdClass; $o->p = $v; $v++;
        $i = 0; $list = []; for ($j = 0; $j < 3; $j++) { $list[] = $i; $i++; }
        function g() { static $a = ["x" => 1]; $a["x"]++; return $a["x"]; }
        var_dump($arr[0], $o->p, implode(",", $list), g(), g());
        "#,
    );
    assert_eq!(out, "int(1)\nint(1)\nstring(5) \"0,1,2\"\nint(2)\nint(3)\n");
}

#[test]
fn static_initializers_run_once_with_any_expression() {
    let out = output(
        r#"<?php
        function f() { static $a = 1, $b = 10, $o = new stdClass; $a++; $b++; return [$a, $b, spl_object_id($o)]; }
        echo json_encode(f()), json_encode(f()), "\n";
        "#,
    );
    assert_eq!(out, "[2,11,1][3,12,1]\n");
}

#[test]
fn references_in_arrays_and_the_var_dump_marker() {
    let out = output(
        r#"<?php
        $a = 1; $arr = [&$a, 2]; $a = 7; var_dump($arr);
        $b = [1]; $r = &$b[0]; unset($r); var_dump($b, [...$arr]);
        "#,
    );
    assert_eq!(
        out,
        "array(2) {\n  [0]=>\n  &int(7)\n  [1]=>\n  int(2)\n}\narray(1) {\n  [0]=>\n  int(1)\n}\narray(2) {\n  [0]=>\n  &int(7)\n  [1]=>\n  int(2)\n}\n"
    );
}
