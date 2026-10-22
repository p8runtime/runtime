//! ext/random's Mt19937 and global functions, and ext/ctype. Seeded
//! sequences are real PHP 8.4's output for the same seeds.

mod common;
use common::run_code_capture_output;

fn output(code: &str) -> String {
    run_code_capture_output(code).expect("script failed").1
}

#[test]
fn seeded_sequences_match_php() {
    let out = output(
        r#"<?php
        mt_srand(42); echo mt_rand(), " ", mt_rand(1, 100), " ", mt_rand(PHP_INT_MIN, PHP_INT_MAX), " ", rand(10, 1), "\n";
        mt_srand(3); $a = range(1, 10); shuffle($a);
        echo implode(",", $a), " ", str_shuffle("abcdefgh"), " ", json_encode(array_rand(array_flip(range(1, 20)), 5)), " ", array_rand(['a' => 1, 'b' => 2, 'c' => 3]), "\n";
        srand(42); echo rand(), "\n";
        "#,
    );
    assert_eq!(
        out,
        "804318771 68 -5839597450370612388 7\n4,8,3,9,1,10,5,2,6,7 cfebahgd [1,5,15,16,17] c\n804318771\n"
    );
}

#[test]
fn mt_rand_php_mode_and_deprecated_constants() {
    let out = output(r#"<?php mt_srand(1, MT_RAND_PHP); echo mt_rand(), " ", mt_rand(1, 1000), "\n";"#);
    assert!(out.contains("Constant MT_RAND_PHP is deprecated"), "{out}");
    assert!(out.contains("The MT_RAND_PHP variant of Mt19937 is deprecated"), "{out}");
    assert!(out.ends_with("1244335972 8\n"), "{out}");
}

#[test]
fn range_errors_and_csprng_functions() {
    let out = output(
        r#"<?php
        foreach ([fn() => mt_rand(5, 1), fn() => random_int(5, 1), fn() => random_bytes(0), fn() => mt_rand(1)] as $f) {
            try { $f(); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
        }
        var_dump(mt_getrandmax(), getrandmax(), strlen(random_bytes(7)), random_int(5, 5), @lcg_value() < 1);
        "#,
    );
    assert_eq!(
        out,
        "ValueError: mt_rand(): Argument #2 ($max) must be greater than or equal to argument #1 ($min)\n\
         ValueError: random_int(): Argument #1 ($min) must be less than or equal to argument #2 ($max)\n\
         ValueError: random_bytes(): Argument #1 ($length) must be greater than 0\n\
         ArgumentCountError: mt_rand() expects exactly 2 arguments, 1 given\n\
         int(2147483647)\nint(2147483647)\nint(7)\nint(5)\nbool(true)\n"
    );
}

#[test]
fn ctype_classes_bytes_and_ints() {
    let out = output(
        r#"<?php
        var_dump(ctype_digit("123"), ctype_digit(""), ctype_digit("12a"), ctype_space(" \t\n"), ctype_xdigit("aF09"),
            ctype_punct("!?"), ctype_upper("ABc"), ctype_alnum("é"), extension_loaded('ctype'));
        var_dump(@ctype_digit(53), @ctype_digit(300), @ctype_graph(-200), @ctype_alpha(null));
        "#,
    );
    assert_eq!(
        out,
        "bool(true)\nbool(false)\nbool(false)\nbool(true)\nbool(true)\nbool(true)\nbool(false)\nbool(false)\nbool(true)\n\
         bool(true)\nbool(true)\nbool(true)\nbool(false)\n"
    );
}

#[test]
fn deprecated_builtins_say_so() {
    let out = output(r#"<?php $x = lcg_value(); echo E_STRICT, "\n";"#);
    assert!(
        out.contains("Deprecated: Function lcg_value() is deprecated since 8.4, use \\Random\\Randomizer::getFloat() instead"),
        "{out}"
    );
    assert!(out.contains("Deprecated: Constant E_STRICT is deprecated"), "{out}");
}
