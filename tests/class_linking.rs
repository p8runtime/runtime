//! Class linking: the inheritance checks PHP runs when a class is declared,
//! with its messages. Each script runs through the `php` binary, since a
//! compile error ends it.

use std::process::Command;

/// Run `code` as `name` and return (output, exit status), with the script's
/// directory removed from file paths.
fn run(name: &str, code: &str) -> (String, i32) {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("class_linking");
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
fn incompatible_override_shows_both_prototypes() {
    let (out, status) = run("c1.php", "<?php\nclass A { public function bar($x, $y = 0, $s = 'a long default string', array $a = []) {} }\nclass B extends A { public function bar($x) {} }\n");
    assert_eq!(out, "\nFatal error: Declaration of B::bar($x) must be compatible with A::bar($x, $y = 0, $s = 'a long def...', array $a = []) in c1.php on line 3");
    assert_eq!(status, 255);
}

#[test]
fn final_method_cannot_be_overridden() {
    let (out, status) = run("c2.php", "<?php\nclass A { final public function f() {} }\nclass B extends A { public function f() {} }\n");
    assert_eq!(out, "\nFatal error: Cannot override final method A::f() in c2.php on line 3");
    assert_eq!(status, 255);
}

#[test]
fn static_must_match() {
    let (out, status) = run("c3.php", "<?php\nclass A { public function f() {} }\nclass B extends A { public static function f() {} }\n");
    assert_eq!(out, "\nFatal error: Cannot make non static method A::f() static in class B in c3.php on line 3");
    assert_eq!(status, 255);
}

#[test]
fn access_level_cannot_narrow() {
    let (out, status) = run("c4.php", "<?php\nclass A { public function f() {} }\nclass B extends A { protected function f() {} }\n");
    assert_eq!(out, "\nFatal error: Access level to B::f() must be public (as in class A) in c4.php on line 3");
    assert_eq!(status, 255);
}

#[test]
fn unimplemented_interface_method() {
    let (out, status) = run("c5.php", "<?php\ninterface I { function a(); function b(int $x): string; }\nclass C implements I { function a() {} }\n");
    assert_eq!(out, "\nFatal error: Class C contains 1 abstract method and must therefore be declared abstract or implement the remaining methods (I::b) in c5.php on line 3");
    assert_eq!(status, 255);
}

#[test]
fn internal_interface_signature() {
    let (out, status) = run("c6.php", "<?php\nclass C implements ArrayAccess {\n    function offsetExists($o): bool { return true; }\n    function offsetGet($o): mixed { return 1; }\n    function offsetSet($o): void {}\n    function offsetUnset($o): void {}\n}\n");
    assert_eq!(out, "\nFatal error: Declaration of C::offsetSet($o): void must be compatible with ArrayAccess::offsetSet(mixed $offset, mixed $value): void in c6.php on line 5");
    assert_eq!(status, 255);
}

#[test]
fn tentative_return_type_is_a_deprecation_and_class_alias() {
    let (out, status) = run("c7.php", "<?php\nclass It implements Iterator {\n    function current() { return 1; }\n    function key(): mixed { return 0; }\n    function next(): void {}\n    function rewind(): void {}\n    function valid(): bool { return false; }\n}\necho \"linked\\n\";\nclass_alias('It', 'Alias');\nvar_dump(class_exists('alias'), new Alias instanceof It, get_class(new ALIAS));\n");
    assert_eq!(out, "\nDeprecated: Return type of It::current() should either be compatible with Iterator::current(): mixed, or the #[\\ReturnTypeWillChange] attribute should be used to temporarily suppress the notice in c7.php on line 3\nlinked\nbool(true)\nbool(true)\nstring(2) \"It\"");
    assert_eq!(status, 0);
}

#[test]
fn only_enums_implement_enum_interfaces() {
    let (out, status) = run("c8.php", "<?php\nclass Foo implements UnitEnum {}\n");
    assert_eq!(out, "\nFatal error: Non-enum class Foo cannot implement interface UnitEnum in c8.php on line 2");
    assert_eq!(status, 255);
}

#[test]
fn parameter_and_return_variance() {
    let (out, status) = run("c9.php", "<?php\nabstract class A { abstract function f(int $x): static; }\nclass B extends A { function f(int|string $x): static { return $this; } }\nclass C extends B { function f(mixed $x): B { return $this; } }\n");
    assert_eq!(out, "\nFatal error: Declaration of C::f(mixed $x): B must be compatible with B::f(string|int $x): static in c9.php on line 4");
    assert_eq!(status, 255);
}
