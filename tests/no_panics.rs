//! Inputs that used to abort the php binary (Rust panics, stack overflows)
//! give PHP's results instead. Scripts run through the binary, which is
//! where the interpreter's stack and its guard live.

use std::process::Command;

/// Run `code` as `name`; (output, exit status), script directory removed.
fn run(name: &str, code: &str) -> (String, i32) {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("no_panics");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, code).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_php"))
        .arg(&path)
        .current_dir(&dir)
        .output()
        .expect("run php");
    let text = String::from_utf8_lossy(&output.stdout).replace(&format!("{}/", dir.display()), "");
    (text.trim_end().to_string(), output.status.code().unwrap_or(-1))
}

#[test]
fn builtins_raise_php_errors_instead_of_panicking() {
    let (out, status) = run("n1.php", "<?php\nforeach ([fn() => intdiv(PHP_INT_MIN, -1), fn() => intdiv(1, 0), fn() => array_pad([], PHP_INT_MAX, 0),\n          fn() => mb_strpos(\"abc\", \"b\", 10)] as $f) {\n    try { $f(); } catch (Throwable $e) { echo get_class($e), \": \", $e->getMessage(), \"\\n\"; }\n}\nvar_dump(strpos(\"abc\", \"\"), str_pad(\"x\", -5), substr_replace(\"hello\", \"!\", 2, PHP_INT_MAX));\n$a = [1, 2, 3]; array_splice($a, PHP_INT_MAX, 1); var_dump(count($a));\n$x = []; $y = ['k' => &$x]; $x = array_merge_recursive($x, $y);\ntry { array_merge_recursive($x, $y); } catch (Error $e) { echo $e->getMessage(), \"\\n\"; }\n");
    assert_eq!(out, "ArithmeticError: Division of PHP_INT_MIN by -1 is not an integer\nDivisionByZeroError: Division by zero\nValueError: array_pad(): Argument #2 ($length) must not exceed the maximum allowed array size\nValueError: mb_strpos(): Argument #3 ($offset) must be contained in argument #1 ($haystack)\nint(0)\nstring(1) \"x\"\nstring(3) \"he!\"\nint(3)\nRecursion detected");
    assert_eq!(status, 0);
}

#[test]
fn a_call_in_finally_while_unwinding() {
    let (out, status) = run("n2.php", "<?php\nfunction foo() {\n    echo \"4\";\n}\nfunction bar() {\n    try {\n        echo \"2\";\n        throw new Exception();\n        echo \"x\";\n    } catch (MyEx $ex) {\n        echo \"x\";\n    } finally {\n        echo \"3\";\n        foo();\n        echo \"5\";\n    }\n}\ntry {\n    echo \"1\";\n    bar();\n    echo \"x\";\n} catch (Exception $ex) {\n    echo \"6\";\n}\necho \"\\n\";\n?>\n");
    assert_eq!(out, "123456");
    assert_eq!(status, 0);
}

#[test]
fn infinite_recursion_is_an_error_not_a_crash() {
    let (out, status) = run(
        "deep.php",
        "<?php\nfunction f($n) { return array_map('f', [$n + 1])[0]; }\ntry { f(0); } catch (Error $e) { echo $e->getMessage(), \"\\n\"; }\necho \"survived\\n\";\n",
    );
    assert!(out.starts_with("Maximum call stack size of "), "{out}");
    assert!(out.contains("bytes reached. Infinite recursion?\nsurvived"), "{out}");
    assert_eq!(status, 0);
}
