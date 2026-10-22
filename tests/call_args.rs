//! Spread and named arguments, as PHP 8.4 binds them.

mod common;
use common::run_code_capture_output;

fn output(code: &str) -> String {
    run_code_capture_output(code).expect("script failed").1
}

#[test]
fn spread_and_named_arguments_bind_to_parameters() {
    let out = output(
        r#"<?php
        function f($a, $b = 2, ...$rest) { return "$a|$b|" . json_encode($rest); }
        echo f(...[1, 2, 3]), "\n", f(b: 5, a: 4), "\n", f(1, 2, 3, x: 9), "\n";
        echo f(...['a' => 7, 'b' => 8]), "\n";
        function g($x, $y = 10, $z = 20) { return "$x $y $z"; }
        echo g(1, z: 3), "\n";
        foreach (['g(y: 1);', 'g(1, x: 2);', 'g(1, nope: 2);', 'g();'] as $code) {
            try { eval($code); } catch (Error $e) { echo get_class($e), ": ", preg_replace('/ in .* on line \d+/', '', $e->getMessage()), "\n"; }
        }
        class P { function __construct(public int $x = 1, public int $y = 2) {} }
        $p = new P(y: 5); echo $p->x, $p->y, "\n";
        echo call_user_func_array('g', ['x' => 'X', 'z' => 'Z']), "\n";
        "#,
    );
    assert_eq!(
        out,
        "1|2|[3]\n4|5|[]\n1|2|{\"0\":3,\"x\":9}\n7|8|[]\n1 10 3\n\
         ArgumentCountError: g(): Argument #1 ($x) not passed\n\
         Error: Named parameter $x overwrites previous argument\n\
         Error: Unknown named parameter $nope\n\
         ArgumentCountError: Too few arguments to function g(), 0 passed and at least 1 expected\n\
         15\nX 10 Z\n"
    );
}

#[test]
fn builtins_take_named_arguments_by_their_php_names() {
    let out = output(
        r#"<?php
        echo str_pad("x", 5, pad_type: STR_PAD_LEFT, pad_string: "-"), "\n";
        echo json_encode(array_slice([1, 2, 3, 4], 1, preserve_keys: true)), "\n";
        echo max(...[3, 9, 2]), "\n";
        "#,
    );
    assert_eq!(out, "----x\n{\"1\":2,\"2\":3,\"3\":4}\n9\n");
}
