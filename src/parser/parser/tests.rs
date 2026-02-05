use bumpalo::Bump;

use crate::parser::ast::Stmt;
use crate::parser::lexer::Lexer;
use crate::parser::parser::{Parser, ParserMode};

#[test]
fn phpx_allows_automatic_semicolons() {
    let code = "$a = 1\n$b = 2\necho $a\n";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(program.errors.is_empty(), "unexpected errors: {:?}", program.errors);
}

#[test]
fn php_requires_semicolons() {
    let code = "<?php $a = 1\n$b = 2\necho $a\n";
    let arena = Bump::new();
    let mut parser = Parser::new(Lexer::new(code.as_bytes()), &arena);
    let program = parser.parse_program();

    assert!(!program.errors.is_empty());
}

#[test]
fn phpx_return_line_terminator_ends_statement() {
    let code = "function f() { return\n $x\n }";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(program.errors.is_empty(), "unexpected errors: {:?}", program.errors);

    let func_stmt = program
        .statements
        .iter()
        .find(|s| matches!(***s, Stmt::Function { .. }))
        .expect("expected function stmt");

    match &**func_stmt {
        Stmt::Function { body, .. } => {
            let mut stmts = body.iter().filter(|s| !matches!(***s, Stmt::Nop { .. }));
            let ret_stmt = stmts.next().expect("expected return stmt");
            match &**ret_stmt {
                Stmt::Return { expr, .. } => assert!(expr.is_none(), "expected return without expr"),
                other => panic!("expected return stmt, got {:?}", other),
            }

            let expr_stmt = stmts.next().expect("expected expression stmt");
            match &**expr_stmt {
                Stmt::Expression { .. } => {}
                other => panic!("expected expression stmt, got {:?}", other),
            }
        }
        other => panic!("expected function stmt, got {:?}", other),
    }
}
