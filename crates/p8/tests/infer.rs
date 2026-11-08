//! The kinds the compiler proves per register (`p8_bytecode::infer`) and
//! the specialised opcodes they pick (`insn::specialised`): each where it
//! is proven, and not where a reference, an undefined variable, a name
//! lookup or a handler could make it wrong.

fn platform() -> p8::Platform {
    p8::Platform::new(&[p8::extensions::STANDARD])
}

/// The opcodes the loop runs for function `name` of `source`, one per
/// instruction, each with the instruction (`--dump`'s lines).
fn ops(source: &str, name: &str) -> Vec<(String, String)> {
    let platform = platform();
    let script = platform.compile(source.as_bytes(), "t.php").expect("compiles");
    let dump = platform.disassemble(&script);
    let mut out = Vec::new();
    let mut on = false;
    for line in dump.lines() {
        if let Some(rest) = line.strip_prefix("function ") {
            on = rest.split(' ').next() == Some(name);
            continue;
        }
        if on {
            let mut words = line.split_whitespace();
            let (_pc, _line, op) = (words.next(), words.next(), words.next().expect("an opcode"));
            out.push((op.to_string(), words.collect::<Vec<_>>().join(" ")));
        }
    }
    assert!(!out.is_empty(), "no function {name} in\n{dump}");
    out
}

/// The opcode of the `nth` instruction of `name` whose text starts with `instr`.
fn op_of(source: &str, name: &str, instr: &str, nth: usize) -> String {
    let all = ops(source, name);
    all.iter().filter(|(_, i)| i.starts_with(instr)).nth(nth).unwrap_or_else(|| panic!("no {instr} #{nth} in {all:#?}")).0.clone()
}

#[test]
fn a_defined_variable_is_copied_without_tests() {
    let src = "<?php function f($a) { $b = $a; $c = $b; return $c; }";
    assert_eq!(op_of(src, "f", "Assign", 0), "AssignVal");
    assert_eq!(op_of(src, "f", "Assign", 1), "AssignVal");
}

#[test]
fn a_variable_that_may_be_undefined_keeps_its_warning() {
    let src = "<?php function f($x) { if ($x) { $a = 1; } $b = $a; return $b; }";
    assert_eq!(op_of(src, "f", "Assign { var: 2", 0), "Assign");
    let src = "<?php function f() { $b = $u; return $b; }";
    assert_eq!(op_of(src, "f", "Assign", 0), "Assign");
}

#[test]
fn references_are_never_proven() {
    // A by-reference parameter, a capture by reference, global, static,
    // a by-reference foreach, a by-reference argument, `=&`.
    for src in [
        "<?php function f(&$a) { $b = $a; return $b; }",
        "<?php function f() { $a = 1; $g = function () use (&$a) { $a = []; }; $g(); $b = $a; return $b; }",
        "<?php function f() { global $a; $b = $a; return $b; }",
        "<?php function f() { static $a = 0; $b = $a; return $b; }",
        "<?php function f($x) { $a = 1; foreach ($x as &$a) {} $b = $a; return $b; }",
        "<?php function f() { $a = [3, 1]; sort($a); $b = $a; return $b; }",
        "<?php function f() { $a = 1; $c = &$a; $b = $a; return $b; }",
    ] {
        let assign = ops(src, "f").into_iter().filter(|(_, i)| i.starts_with("Assign") && i.contains("src: 0") | i.contains("src: 1")).collect::<Vec<_>>();
        assert!(assign.iter().all(|(op, _)| !op.ends_with("Val")), "{src}: {assign:?}");
    }
}

#[test]
fn variables_read_by_name_are_never_proven() {
    for src in [
        "<?php function f($v) { $a = 1; extract($v); $b = $a; return $b; }",
        "<?php function f($n) { $a = 1; $$n = 2; $b = $a; return $b; }",
        "<?php function f($p) { $a = 1; include $p; $b = $a; return $b; }",
        "<?php $a = 1; $b = $a; echo $b;",
    ] {
        let name = if src.contains("function f") { "f" } else { "{main}" };
        let assign = ops(src, name).into_iter().filter(|(_, i)| i.starts_with("Assign { var: ") && !i.contains("src: 2") && !i.contains("src: 3") && !i.contains("src: 4")).collect::<Vec<_>>();
        assert!(assign.iter().skip(1).all(|(op, _)| !op.ends_with("Val")), "{src}: {assign:?}");
    }
}

#[test]
fn a_catch_starts_with_nothing_known() {
    let src = "<?php function f() { $a = 1; try { g(); } catch (Exception $e) { $b = $a; return $b; } return 0; }";
    assert_eq!(op_of(src, "f", "Assign { var: 2", 0), "Assign");
}

#[test]
fn an_int_sum_goes_over_plain_registers() {
    let src = "<?php function f($n) { $s = 0; for ($i = 0; $i < $n; $i++) { $s = $s + $i; } return $s; }";
    assert_eq!(op_of(src, "f", "Add", 0), "AddPlain");
    // A string there before: released by the store.
    let src = "<?php function f($n) { $s = 'x'; $s = $n + 1; return $s; }";
    assert_eq!(op_of(src, "f", "Add", 0), "Add");
}

#[test]
fn return_types_the_kinds_prove_are_not_checked() {
    let src = "<?php function f(int $a): bool { return $a < 3; }";
    assert_eq!(op_of(src, "f", "Rare { op: VerifyReturn", 0), "Nop");
    let src = "<?php function f(string $s): int { return strlen($s); }";
    assert_eq!(op_of(src, "f", "Rare { op: VerifyReturn", 0), "Nop");
    // An int sum may be a float (overflow); an int|float param may hold
    // an int (converted on return), a float one never does (converted on
    // entry); anything from a call is checked.
    let src = "<?php function f(int $a): int { return $a + 1; }";
    assert_eq!(op_of(src, "f", "Rare { op: VerifyReturn", 0), "VerifyReturn");
    let src = "<?php function f(int|float $a): float { return $a; }";
    assert_eq!(op_of(src, "f", "Rare { op: VerifyReturn", 0), "VerifyReturn");
    let src = "<?php function f(float $a): float { return $a; }";
    assert_eq!(op_of(src, "f", "Rare { op: VerifyReturn", 0), "Nop");
    let src = "<?php function f(): int { return g(); }";
    assert_eq!(op_of(src, "f", "Rare { op: VerifyReturn", 0), "VerifyReturn");
    // A generator's return is checked where it runs.
    let src = "<?php function f(): iterable { yield 1; return 2; }";
    assert!(ops(src, "f").iter().all(|(op, _)| op != "Nop"));
}

#[test]
fn a_type_test_tells_the_branch() {
    let src = "<?php function f($x): int { if (is_int($x)) { return $x; } return 0; }";
    assert_eq!(op_of(src, "f", "Rare { op: VerifyReturn", 0), "Nop");
    let src = "<?php function f($x): int { if (!is_string($x)) { return 0; } return $x; }";
    assert_eq!(op_of(src, "f", "Rare { op: VerifyReturn", 1), "VerifyReturn");
    let src = "<?php function f($x): string { if (!is_string($x)) { return ''; } return $x; }";
    assert_eq!(op_of(src, "f", "Rare { op: VerifyReturn", 1), "Nop");
    // Not on the other branch.
    let src = "<?php function f($x): int { if (is_int($x)) { return 1; } return $x; }";
    assert_eq!(op_of(src, "f", "Rare { op: VerifyReturn", 1), "VerifyReturn");
}

#[test]
fn a_typed_parameter_is_known_inside() {
    let src = "<?php function f(bool $b): bool { return $b; }";
    assert_eq!(op_of(src, "f", "Rare { op: VerifyReturn", 0), "Nop");
    // Not passed, with a default of another type: as it is.
    let src = "<?php function f(?bool $b = null): bool { return $b; }";
    assert_eq!(op_of(src, "f", "Rare { op: VerifyReturn", 0), "VerifyReturn");
}
