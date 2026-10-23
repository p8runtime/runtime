//! Arrow functions, variadic parameters and generators, as PHP 8.4 runs them.

mod common;
use common::run_code_capture_output;

fn output(code: &str) -> String {
    run_code_capture_output(code).expect("script failed").1
}

#[test]
fn arrow_functions_capture_by_value() {
    let out = output(
        r#"<?php
        $f = fn() => 1; echo $f(), "\n";
        echo implode(",", array_map(fn($x) => $x * 2, [1, 2, 3])), "\n";
        $m = 3; $g = fn($x) => $x * $m; $m = 10; echo $g(2), "\n";
        $n = fn($x) => fn($y) => $x + $y + $m; echo $n(1)(2), "\n";
        class C { public $v = 5; function get() { return fn() => $this->v; } }
        echo (new C)->get()(), "\n";
        foreach ([fn() => 1 % 0, fn() => 2] as $f) {
            try { echo $f(), "\n"; } catch (DivisionByZeroError $e) { echo "caught\n"; }
        }
        "#,
    );
    assert_eq!(out, "1\n2,4,6\n6\n13\n5\ncaught\n2\n");
}

#[test]
fn variadic_parameters_collect_the_rest() {
    let out = output(
        r#"<?php
        $r = fn(int ...$xs): int => array_sum($xs); echo $r(1, 2, 3), "\n";
        $c = function($first, ...$rest) { return $first . ":" . implode(",", $rest); };
        echo $c("a", "b", "c"), "|", $c("x"), "\n";
        "#,
    );
    assert_eq!(out, "6\na:b,c|x:\n");
}

#[test]
fn generators_from_methods_closures_and_arrow_functions() {
    let out = output(
        r#"<?php
        class A { function g() { yield 1; } static function s() { yield 2; } }
        foreach ((new A)->g() as $v) echo $v;
        foreach (A::s() as $v) echo $v;
        foreach ((function() { yield 3; })() as $v) echo $v;
        foreach ((fn() => yield 4)() as $v) echo $v;
        function loop() { foreach ([5, 6] as $v) { yield $v; } }
        foreach (loop() as $v) echo $v;
        echo "\n";
        "#,
    );
    assert_eq!(out, "123456\n");
}

#[test]
fn generator_methods_drive_the_generator() {
    let out = output(
        r#"<?php
        function g() { $x = yield 1; echo "got $x\n"; yield 2; return 9; }
        $gen = g();
        echo $gen->current(), "\n";
        echo $gen->send("A"), "\n";
        $gen->next();
        var_dump($gen->valid(), $gen->getReturn());
        function t() { try { yield 1; } catch (Exception $e) { echo "caught ", $e->getMessage(), "\n"; yield 99; } }
        $gen = t(); $gen->current();
        echo $gen->throw(new Exception("boom")), "\n";
        function k() { yield 'a' => 1; yield 2; }
        $gen = k(); $gen->next();
        try { $gen->rewind(); } catch (Exception $e) { echo $e->getMessage(), "\n"; }
        foreach (k() as $key => $v) echo "$key=$v ";
        echo "\n";
        "#,
    );
    assert_eq!(
        out,
        "1\ngot A\n2\nbool(false)\nint(9)\ncaught boom\n99\n\
         Cannot rewind a generator that was already run\na=1 0=2 \n"
    );
}

#[test]
fn yield_from_delegates_and_returns_the_inner_result() {
    let out = output(
        r#"<?php
        function inner() { yield 1; yield 2; return "r"; }
        function outer() { $r = yield from inner(); echo "[$r]"; yield from ['x' => 3]; }
        foreach (outer() as $k => $v) echo "$k:$v ";
        echo "\n", implode(",", iterator_to_array(outer(), false)), "\n";
        "#,
    );
    assert_eq!(out, "0:1 1:2 [r]x:3 \n[r]1,2,3\n");
}
