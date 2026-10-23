//! Errors and warnings PHP reports while compiling a file: a compile error
//! stops the whole file (nothing in it runs); warnings are shown before it
//! runs. Expected text is real PHP 8.4's.

fn php(code: &str) -> (String, Option<i32>) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_php"))
        .args(["-r", code])
        .output()
        .expect("run php");
    (String::from_utf8_lossy(&out.stdout).into_owned(), out.status.code())
}

fn fatal(code: &str, message: &str) {
    let (out, status) = php(&format!("echo 'ran';\n{}", code));
    assert_eq!(out, format!("\nFatal error: {} in Command line code on line 2\n", message), "{code}");
    assert_eq!(status, Some(255), "{code}");
}

#[test]
fn this_cannot_be_reassigned_or_bound() {
    fatal("$this = 1;", "Cannot re-assign $this");
    fatal("[$a, $this] = [1, 2];", "Cannot re-assign $this");
    fatal("foreach ([1] as $this) {}", "Cannot re-assign $this");
    fatal("try {} catch (Exception $this) {}", "Cannot re-assign $this");
    fatal("function f($this) {}", "Cannot use $this as parameter");
    fatal("$f = function () use ($this) {};", "Cannot use $this as lexical variable");
    fatal("function f() { global $this; }", "Cannot use $this as global variable");
    fatal("function f() { static $this; }", "Cannot use $this as static variable");
    fatal("unset($this);", "Cannot unset $this");
}

#[test]
fn empty_dim_cannot_be_read() {
    fatal("$b = $a[];", "Cannot use [] for reading");
    fatal("isset($a[]);", "Cannot use [] for reading");
    // As the object of a property write, or a call argument, it is not a read.
    let (out, _) = php("function y() { if (false) { $arr[]->y = 1; s($arr[]); } } echo 'ok';");
    assert_eq!(out, "ok");
}

#[test]
fn magic_methods_have_fixed_signatures() {
    fatal("class A { function __get($a, $b) {} }", "Method A::__get() must take exactly 1 argument");
    fatal("class A { function __set($a) {} }", "Method A::__set() must take exactly 2 arguments");
    fatal("class A { function __toString($x): string {} }", "Method A::__toString() cannot take arguments");
    fatal("class A { function __get(&$n) {} }", "Method A::__get() cannot take arguments by reference");
    fatal("class A { static function __get($n) {} }", "Method A::__get() cannot be static");
    fatal("class A { function __callStatic($n, $a) {} }", "Method A::__callStatic() must be static");
    fatal(
        "class A { function __isset(int $n) {} }",
        "A::__isset(): Parameter #1 ($n) must be of type string when declared",
    );
    fatal("class A { function __toString(): int {} }", "A::__toString(): Return type must be string when declared");
    fatal("class A { function __debugInfo(): string {} }", "A::__debugInfo(): Return type must be ?array when declared");
    fatal("class A { function __construct(): void {} }", "Method A::__construct() cannot declare a return type");
    // never and mixed-compatible declarations are accepted
    let (out, _) = php("class A { function __get(mixed $n) {} function __toString(): never { throw new Exception; } } echo 'ok';");
    assert_eq!(out, "ok");
    let (out, _) = php("class A { private function __get($n) {} } echo 'ran';");
    assert_eq!(
        out,
        "\nWarning: The magic method A::__get() must have public visibility in Command line code on line 1\nran"
    );
}

#[test]
fn use_statements_are_checked() {
    let (out, _) = php("use A;\necho 'ran';");
    assert_eq!(
        out,
        "\nWarning: The use statement with non-compound name 'A' has no effect in Command line code on line 1\nran"
    );
    fatal("use A\\B; use C\\B;", "Cannot use C\\B as B because the name is already in use");
    fatal("use function A\\f; use function B\\f;", "Cannot use function B\\f as f because the name is already in use");
    fatal("use A\\B as self;", "Cannot use A\\B as self because 'self' is a special class name");
}

#[test]
fn class_scope_keywords_need_a_class() {
    fatal("function f() { return self::X; }", "Cannot use \"self\" when no class scope is active");
    fatal("function f() { return new static; }", "Cannot use \"static\" when no class scope is active");
    // A closure or the file's top level may run inside a class.
    let (out, _) = php("$f = function () { return static::class; }; echo 'ok';");
    assert_eq!(out, "ok");
}

#[test]
fn reserved_names_cannot_be_declared() {
    fatal("interface mixed {}", "Cannot use \"mixed\" as an interface name as it is reserved");
    fatal("trait never {}", "Cannot use \"never\" as a trait name as it is reserved");
}
