//! The nullsafe operator `?->`: a null base short-circuits the rest of the
//! chain, and PHP rejects writing to or referencing a nullsafe chain at
//! compile time.

mod common;
use common::run_code_capture_output;

fn output(code: &str) -> String {
    run_code_capture_output(code).expect("script failed").1
}

#[test]
fn null_base_short_circuits_the_whole_chain() {
    let out = output(
        r#"<?php
        class Foo { public $p = null; function self() { return $this; } function none() { return null; } }
        function side($x) { echo "side effect\n"; return $x; }
        $foo = new Foo; $null = null;
        var_dump($null?->bar(side(1))->baz);
        var_dump($null?->a['b']->c());
        var_dump($foo->none()?->x(side(2)));
        var_dump($foo?->self()?->p);
        var_dump(isset($null?->a->b), $null?->a->b ?? 'default');
        "#,
    );
    assert_eq!(out, "NULL\nNULL\nNULL\nNULL\nbool(false)\nstring(7) \"default\"\n");
}

#[test]
fn non_null_base_continues_the_chain() {
    let out = output(
        r#"<?php
        class Foo { public $p = ['k' => 'v']; static $s = 's'; function self() { return $this; } }
        $foo = new Foo;
        var_dump($foo?->self()->p['k'], $foo?->self()::$s);
        "#,
    );
    assert_eq!(out, "string(1) \"v\"\nstring(1) \"s\"\n");
}

fn php(args: &[&str]) -> (String, Option<i32>) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_php"))
        .args(args)
        .output()
        .expect("run php");
    (String::from_utf8_lossy(&out.stdout).into_owned(), out.status.code())
}

#[test]
fn writing_through_nullsafe_is_a_compile_error() {
    // Compile errors stop the whole file: nothing before them runs either.
    let (out, status) = php(&["-r", "echo 'not run';\n$a?->b = 1;"]);
    assert_eq!(
        out,
        "\nFatal error: Can't use nullsafe operator in write context in Command line code on line 2\n"
    );
    assert_eq!(status, Some(255));
    let (out, _) = php(&["-r", "function f() {\n  $x = &$a?->b;\n}"]);
    assert_eq!(
        out,
        "\nFatal error: Cannot take reference of a nullsafe chain in Command line code on line 2\n"
    );
    let (out, _) = php(&["-r", "$a?->b['c']++;"]);
    assert!(out.contains("Can't use nullsafe operator in write context"), "{out}");
}

#[test]
fn object_without_array_access_cannot_be_used_as_array() {
    let out = output(
        r#"<?php
        class Foo {}
        $f = new Foo;
        foreach ([fn() => $f[0], fn() => isset($f[0]), fn() => $f->missing()] as $fn) {
            try { $fn(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
        }
        "#,
    );
    assert_eq!(
        out,
        "Cannot use object of type Foo as array\nCannot use object of type Foo as array\nCall to undefined method Foo::missing()\n"
    );
}

#[test]
fn eval_parse_error_is_a_parse_error_exception() {
    let out = output(
        r#"<?php
        try { eval("echo 1 +;"); } catch (ParseError $e) { echo get_class($e), " ", $e->getLine(), "\n"; }
        "#,
    );
    assert_eq!(out, "ParseError 1\n");
}

#[test]
fn isset_on_array_access_asks_offset_exists_only() {
    let out = output(
        r#"<?php
        class A implements ArrayAccess {
            function offsetExists($o): bool { echo "exists\n"; return true; }
            function offsetGet($o): mixed { echo "get\n"; return null; }
            function offsetSet($o, $v): void {}
            function offsetUnset($o): void {}
        }
        $a = new A;
        var_dump(isset($a[1]), empty($a[1]));
        "#,
    );
    assert_eq!(out, "exists\nexists\nget\nbool(true)\nbool(true)\n");
}
