use super::*;
use p8_runtime::ext::Registry;

fn compile(source: &str) -> p8_bytecode::Program {
    let registry = Registry::new(&[]);
    let arena = bumpalo::Bump::new();
    let mut parser = p8_syntax::parser::Parser::new(p8_syntax::lexer::Lexer::new(source.as_bytes()), &arena);
    let program = parser.parse_program();
    assert!(program.errors.is_empty(), "{:?}", program.errors.first().map(|e| e.message.to_string()));
    p8_compiler::Compiler::new(source.as_bytes(), "t.php", &registry).compile(program.statements).expect("compiles")
}

fn native_names(source: &str) -> Vec<String> {
    let program = compile(source);
    let inputs: Vec<Input> = program.functions.iter().enumerate().map(|(i, f)| Input { file: 0, index: i as u32, function: f }).collect();
    let built = build(&inputs, b"package bytes").expect("an object file");
    assert_eq!(built.native.len(), built.names.len());
    built.names
}

/// A function made only of instructions with templates is compiled; one
/// with any other instruction (a concatenation, a typed parameter's
/// check, a string literal copied into a variable) stays bytecode.
#[test]
fn functions_of_templated_instructions_are_compiled_and_no_others() {
    let names = native_names(
        r#"<?php
        function sq($x) { return $x * $x; }
        function sum_to($n) { $s = 0; for ($i = 0; $i < $n; $i++) { $s = $s + sq($i) - 1; } return $s; }
        function wave($n) { $x = 0.5; $k = 0; while ($k < $n) { $x = $x * 1.5 + 0.25; if ($x > 1000.0) { $x = $x - 999.0; } $k += 1; } return $x; }
        function text($a) { return $a . "x"; }
        function typed(int $a) { return $a + 1; }
        function greeting() { $s = "hello"; return $s; }
        "#,
    );
    assert_eq!(names, ["sq", "sum_to", "wave"]);
}

/// The object has what the executable needs: `main`, which calls the
/// boundary's entry, and the call helper it imports.
#[test]
fn the_object_has_main_and_names_the_boundary() {
    let program = compile("<?php function one() { return 1; }");
    let inputs = [Input { file: 3, index: 0, function: &program.functions[0] }];
    let built = build(&inputs, b"p8pack").expect("an object file");
    assert_eq!(built.native, [(3, 0)]);
    let has = |s: &str| built.object.windows(s.len()).any(|w| w == s.as_bytes());
    assert!(has(abi::ENTRY) && has(abi::CALL) && has("main"));
    // The package's bytes are in it as they are.
    assert!(has("p8pack"));
}
