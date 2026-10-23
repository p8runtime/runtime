//! What a script finds defined before it runs a line: the constants and ini
//! directives of the loaded extensions, with real PHP 8.4's values, and
//! the `php` command line options that change them.

mod common;
use common::run_code_capture_output;

fn output(code: &str) -> String {
    run_code_capture_output(code).expect("script failed").1
}

#[test]
fn extension_constants_have_php_values() {
    let out = output(
        r#"<?php
        var_dump(SORT_REGULAR, SORT_STRING, SEEK_END, EXTR_REFS, COUNT_RECURSIVE,
            PHP_ROUND_HALF_UP, LOCK_EX, JSON_BIGINT_AS_STRING, PREG_SPLIT_NO_EMPTY,
            M_SQRT2, PHP_FLOAT_EPSILON, DATE_ATOM, MB_CASE_TITLE, ZLIB_ENCODING_GZIP);
        "#,
    );
    assert_eq!(
        out,
        "int(0)\nint(2)\nint(2)\nint(256)\nint(1)\nint(1)\nint(2)\nint(2)\nint(1)\n\
         float(1.4142135623730951)\nfloat(2.220446049250313E-16)\n\
         string(13) \"Y-m-d\\TH:i:sP\"\nint(2)\nint(31)\n"
    );
}

#[test]
fn constants_of_unloaded_extensions_are_undefined() {
    // tokenizer is not loaded, so its T_* constants must not exist
    let out = output(
        r#"<?php
        var_dump(defined('T_STRING'), extension_loaded('tokenizer'));
        "#,
    );
    assert_eq!(out, "bool(false)\nbool(false)\n");
}

#[test]
fn built_in_extensions_report_loaded() {
    let out = output(
        r#"<?php
        var_dump(extension_loaded('spl'), extension_loaded('Reflection'), extension_loaded('CORE'));
        $loaded = get_loaded_extensions();
        var_dump($loaded[0], in_array('SPL', $loaded, true), get_loaded_extensions(true));
        "#,
    );
    assert_eq!(
        out,
        "bool(true)\nbool(true)\nbool(true)\nstring(4) \"Core\"\nbool(true)\narray(0) {\n}\n"
    );
}

#[test]
fn php_binary_is_the_running_executable() {
    let out = output(r#"<?php var_dump(is_file(PHP_BINARY), PHP_BINARY === realpath(PHP_BINARY));"#);
    assert_eq!(out, "bool(true)\nbool(true)\n");
}

#[test]
fn precision_directives_drive_float_output() {
    let out = output(
        r#"<?php
        $f = 1/3;
        ini_set('precision', 4); echo $f, " ", print_r($f, true), "\n";
        ini_set('serialize_precision', 5); var_dump($f); echo json_encode($f), " ", var_export($f, true), "\n";
        ini_restore('precision'); ini_restore('serialize_precision'); echo $f, "\n"; var_dump($f);
        "#,
    );
    assert_eq!(
        out,
        "0.3333 0.3333\nfloat(0.33333)\n0.33333 0.33333\n0.33333333333333\nfloat(0.3333333333333333)\n"
    );
}

#[test]
fn unknown_directives_do_not_exist() {
    let out = output(r#"<?php var_dump(ini_get('no.such'), ini_set('no.such', '1'), ini_get('zend.assertions'));"#);
    assert_eq!(out, "bool(false)\nbool(false)\nstring(1) \"1\"\n");
}

fn php(args: &[&str]) -> String {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_php"))
        .args(args)
        .output()
        .expect("run php");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn d_options_are_read_like_php_ini() {
    let out = php(&[
        "-n",
        "-d",
        "precision=5",
        "-d",
        "error_reporting=E_ALL & ~E_DEPRECATED",
        "-ddisplay_errors=Off",
        "-d",
        "no.such=1",
        "-r",
        "var_dump(ini_get('precision'), error_reporting(), ini_get('display_errors')); echo M_PI;",
    ]);
    assert_eq!(out, "string(1) \"5\"\nint(24575)\nstring(0) \"\"\n3.1416");
}

#[test]
fn r_runs_code_as_command_line_code() {
    let out = php(&["-r", r#"echo __FILE__, " ", $argv[0], " ", $argc, " ", $argv[2], "\n"; echo $u;"#, "--", "a", "b"]);
    assert_eq!(
        out,
        "Command line code Standard input code 3 b\n\nWarning: Undefined variable $u in Command line code on line 1\n"
    );
}
