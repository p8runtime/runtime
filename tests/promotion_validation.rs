use bumpalo::Bump;
use php_rs::parser::lexer::Lexer;
use php_rs::parser::parser::Parser;

#[test]
fn promotion_requires_visibility_in_constructor() {
    let code = "<?php class C { public function __construct(readonly int $x) {} }";
    let arena = Bump::new();
    let mut parser = Parser::new(Lexer::new(code.as_bytes()), &arena);
    let program = parser.parse_program();

    assert!(
        program
            .errors
            .iter()
            .any(|e| e.message.contains("requires visibility")),
        "expected missing visibility error for readonly promotion"
    );
}

#[test]
fn promotion_only_allowed_in_constructor() {
    let code = "<?php class C { public function foo(public int $x) {} }";
    let arena = Bump::new();
    let mut parser = Parser::new(Lexer::new(code.as_bytes()), &arena);
    let program = parser.parse_program();

    assert!(
        program
            .errors
            .iter()
            .any(|e| e.message.contains("only allowed in constructors")),
        "expected promotion outside constructor to error"
    );
}

#[test]
fn promotion_rejects_by_ref() {
    let code = "<?php class C { public function __construct(public int &$x) {} }";
    let arena = Bump::new();
    let mut parser = Parser::new(Lexer::new(code.as_bytes()), &arena);
    let program = parser.parse_program();

    assert!(
        program
            .errors
            .iter()
            .any(|e| e.message.contains("by-reference")),
        "expected by-reference promotion to error"
    );
}

#[test]
fn promotion_rejects_variadic() {
    let code = "<?php class C { public function __construct(public int ...$x) {} }";
    let arena = Bump::new();
    let mut parser = Parser::new(Lexer::new(code.as_bytes()), &arena);
    let program = parser.parse_program();

    assert!(
        program
            .errors
            .iter()
            .any(|e| e.message.contains("variadic")),
        "expected variadic promotion to error"
    );
}

#[test]
fn promotion_not_allowed_in_trait_constructor() {
    let code = "<?php trait T { public function __construct(public int $x) {} }";
    let arena = Bump::new();
    let mut parser = Parser::new(Lexer::new(code.as_bytes()), &arena);
    let program = parser.parse_program();

    assert!(
        program
            .errors
            .iter()
            .any(|e| e.message.contains("interfaces/traits")),
        "expected trait promotion to error"
    );
}
