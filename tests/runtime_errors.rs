//! Runtime errors and diagnostics as PHP 8.4 reports them: undefined
//! variables and properties, quiet fetches (??, isset), Errors scripts can
//! catch, and builtin argument checks from PHP's own signatures.

mod common;
use common::run_code_capture_output;

fn output(code: &str) -> String {
    let (_, out) = run_code_capture_output(code).expect("script failed");
    // Locations depend on how the test harness names the script.
    out.lines()
        .map(|l| match l.find(" in ") {
            Some(i) if l.starts_with("Warning:") || l.starts_with("Deprecated:") => &l[..i],
            _ => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn coalesce_and_isset_read_quietly() {
    let out = output(
        r#"<?php
        $a = [];
        $o = new stdClass;
        echo $u ?? "d", $u['k'] ?? "d", $a['k']['j'] ?? "d", $o->p->q ?? "d", $n->p ?? "d", "\n";
        var_dump(isset($u['k']['j']), empty($o->p->q));
        "#,
    );
    assert_eq!(out, "ddddd\nbool(false)\nbool(true)");
}

#[test]
fn undefined_variable_warns_where_it_is_read() {
    let out = output(
        r#"<?php
        function f(&$r) { $r = 1; }
        f($created);
        echo $created, "\n";
        $z .= "x";
        echo strlen($missing), "\n";
        "#,
    );
    assert_eq!(
        out,
        "1\n\nWarning: Undefined variable $z\n\nWarning: Undefined variable $missing\n\n\
         Deprecated: strlen(): Passing null to parameter #1 ($string) of type string is deprecated\n0"
    );
}

#[test]
fn property_reads_warn_or_throw_as_php_does() {
    let out = output(
        r#"<?php
        class A { private $priv = 1; public int $typed; }
        $a = new A;
        try { echo $a->priv; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
        try { echo $a->typed; } catch (Error $e) { echo $e->getMessage(), "\n"; }
        echo $a->missing, "|\n";
        $n = null; echo $n->x, "|\n";
        try { $n->m(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
        "#,
    );
    assert_eq!(
        out,
        "Error: Cannot access private property A::$priv\n\
         Typed property A::$typed must not be accessed before initialization\n\
         \nWarning: Undefined property: A::$missing\n|\n\
         \nWarning: Attempt to read property \"x\" on null\n|\n\
         Call to a member function m() on null"
    );
}

#[test]
fn engine_errors_are_catchable() {
    let out = output(
        r#"<?php
        abstract class Ab {}
        foreach ([
            function() { return 1 % 0; },
            function() { return new Ab; },
            function() { return new DoesNotExist; },
        ] as $f) {
            try { $f(); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
        }
        "#,
    );
    assert_eq!(
        out,
        "DivisionByZeroError: Modulo by zero\n\
         Error: Cannot instantiate abstract class Ab\n\
         Error: Class \"DoesNotExist\" not found\n"
            .trim_end()
    );
}

#[test]
fn builtins_check_arguments_against_php_signatures() {
    let out = output(
        r#"<?php
        foreach ([
            function() { return strlen(); },
            function() { return strlen([]); },
            function() { return str_repeat("x", "abc"); },
            function() { return array_merge([1], true); },
            function() { return in_array(1); },
            function() { return count(5); },
        ] as $f) {
            try { $f(); } catch (TypeError $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
        }
        echo str_repeat("x", "3"), "\n";
        echo str_repeat("x", 2.5), "\n";
        "#,
    );
    assert_eq!(
        out,
        "ArgumentCountError: strlen() expects exactly 1 argument, 0 given\n\
         TypeError: strlen(): Argument #1 ($string) must be of type string, array given\n\
         TypeError: str_repeat(): Argument #2 ($times) must be of type int, string given\n\
         TypeError: array_merge(): Argument #2 must be of type array, true given\n\
         ArgumentCountError: in_array() expects at least 2 arguments, 1 given\n\
         TypeError: count(): Argument #1 ($value) must be of type Countable|array, int given\n\
         xxx\n\
         \nDeprecated: Implicit conversion from float 2.5 to int loses precision\nxx"
    );
}

#[test]
fn standard_exception_classes_exist() {
    let out = output(
        r#"<?php
        foreach (["InvalidArgumentException" => "LogicException", "UnexpectedValueException" => "RuntimeException",
                  "ArgumentCountError" => "TypeError", "JsonException" => "Exception"] as $c => $p) {
            echo $c, " ", is_subclass_of($c, $p) ? "ok" : "missing", "\n";
        }
        "#,
    );
    assert_eq!(
        out,
        "InvalidArgumentException ok\nUnexpectedValueException ok\nArgumentCountError ok\nJsonException ok"
    );
}

#[test]
fn by_reference_methods_and_closing_tag_newline() {
    let out = output("<?php\nclass E { public function &get() { static $v = 3; return $v; } }\necho (new E)->get();\n?>\nend");
    assert_eq!(out, "3end");
}
