use bumpalo::Bump;

use crate::parser::ast::{ClassKind, ClassMember, Stmt};
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

#[test]
fn phpx_parses_struct_field_annotations() {
    let code = "struct User { $id: int @id @autoIncrement; }";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(program.errors.is_empty(), "unexpected errors: {:?}", program.errors);

    let stmt = program
        .statements
        .iter()
        .find(|s| !matches!(***s, Stmt::Nop { .. }))
        .expect("expected struct stmt");

    match &**stmt {
        Stmt::Class { kind, members, .. } => {
            assert_eq!(*kind, ClassKind::Struct);
            let field = members
                .iter()
                .find(|m| matches!(m, ClassMember::Property { .. }))
                .expect("expected struct field");
            match field {
                ClassMember::Property { entries, .. } => {
                    assert_eq!(entries.len(), 1);
                    assert_eq!(entries[0].annotations.len(), 2);
                }
                _ => panic!("expected struct property"),
            }
        }
        other => panic!("expected struct stmt, got {:?}", other),
    }
}

#[test]
fn phpx_parses_struct_field_annotation_args() {
    let code = "struct User { $email: string @map(\"email_address\"); }";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(program.errors.is_empty(), "unexpected errors: {:?}", program.errors);

    let stmt = program
        .statements
        .iter()
        .find(|s| !matches!(***s, Stmt::Nop { .. }))
        .expect("expected struct stmt");

    match &**stmt {
        Stmt::Class { members, .. } => {
            let field = members
                .iter()
                .find(|m| matches!(m, ClassMember::Property { .. }))
                .expect("expected struct field");
            match field {
                ClassMember::Property { entries, .. } => {
                    assert_eq!(entries.len(), 1);
                    assert_eq!(entries[0].annotations.len(), 1);
                    assert_eq!(entries[0].annotations[0].args.len(), 1);
                }
                _ => panic!("expected struct property"),
            }
        }
        other => panic!("expected struct stmt, got {:?}", other),
    }
}

#[test]
fn phpx_parses_colon_typed_parameters() {
    let code = "function Name($props: Object<{ name: string }>): string { return $props.name; }";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(program.errors.is_empty(), "unexpected errors: {:?}", program.errors);
}

#[test]
fn phpx_rejects_legacy_typed_parameters() {
    let code = "function Name(Object<{ name: string }> $props): string { return $props.name; }";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(
        !program.errors.is_empty(),
        "expected parser error for legacy parameter syntax"
    );
    assert!(
        program
            .errors
            .iter()
            .any(|err| err.message.contains("must use '$name: Type' syntax")),
        "expected explicit migration error, got: {:?}",
        program.errors
    );
}

#[test]
fn php_mode_still_allows_legacy_typed_parameters() {
    let code = "<?php function Name(array $props): string { return 'ok'; }";
    let arena = Bump::new();
    let mut parser = Parser::new(Lexer::new(code.as_bytes()), &arena);
    let program = parser.parse_program();

    assert!(
        program.errors.is_empty(),
        "unexpected php-mode errors: {:?}",
        program.errors
    );
}

#[test]
fn phpx_parses_param_object_destructuring_with_defaults() {
    let code = "function FullName({ first: $first, last: $last = 'Smith' }: Object<{ first: string, last: string }>): string { return $first . ' ' . $last; }";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(
        program.errors.is_empty(),
        "unexpected parser errors: {:?}",
        program.errors
    );

    let func_stmt = program
        .statements
        .iter()
        .find(|s| matches!(***s, Stmt::Function { .. }))
        .expect("expected function statement");
    match &**func_stmt {
        Stmt::Function { body, .. } => {
            let assigns = body
                .iter()
                .filter(|stmt| matches!(***stmt, Stmt::Expression { .. }))
                .count();
            assert!(assigns >= 2, "expected lowered destructuring assignments");
        }
        other => panic!("expected function stmt, got {:?}", other),
    }
}

#[test]
fn phpx_parses_foreach_object_destructuring() {
    let code = "foreach ($rows as { id: $id, name: $name }) { echo $id; }";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(
        program.errors.is_empty(),
        "unexpected parser errors: {:?}",
        program.errors
    );

    let foreach_stmt = program
        .statements
        .iter()
        .find(|s| matches!(***s, Stmt::Foreach { .. }))
        .expect("expected foreach statement");

    match &**foreach_stmt {
        Stmt::Foreach { body, .. } => {
            let prologue_assigns = body
                .iter()
                .take(2)
                .filter(|stmt| matches!(***stmt, Stmt::Expression { .. }))
                .count();
            assert_eq!(prologue_assigns, 2, "expected lowered foreach bindings");
        }
        other => panic!("expected foreach stmt, got {:?}", other),
    }
}

#[test]
fn phpx_parses_object_assignment_destructuring() {
    let code = "echo ({ id: $id, slug: $slug } = $pkg)";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(
        program.errors.is_empty(),
        "unexpected parser errors: {:?}",
        program.errors
    );
}

#[test]
fn phpx_parses_object_destructuring_fixture_shape() {
    let code = r#"
$pkg = { id: 42, meta: { slug: "hello" } }
({ id: $id, meta: { slug: $slug } } = $pkg)

function fullName({ first: $first, last: $last = "Smith" }: Object<{ first: string, last?: string }>) {
  return $first . " " . $last
}

echo fullName({ first: "Sam" })

$rows = [
  { name: "A", count: 1 },
  { name: "B", count: 2 },
]

foreach ($rows as { name: $name, count: $count }) {
  echo $name . ":" . $count
}
"#;
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(
        program.errors.is_empty(),
        "unexpected parser errors: {:?}",
        program.errors
    );
}

#[test]
fn phpx_parses_variable_assignment_from_object_literal() {
    let code = "$a = { foo: \"bar\" }";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(
        program.errors.is_empty(),
        "unexpected parser errors: {:?}",
        program.errors
    );

    let stmt = program
        .statements
        .iter()
        .find(|s| !matches!(***s, Stmt::Nop { .. }))
        .expect("expected statement");

    match &**stmt {
        Stmt::Expression { expr, .. } => match **expr {
            crate::parser::ast::Expr::Assign { var, expr: rhs, .. } => {
                assert!(
                    matches!(*var, crate::parser::ast::Expr::Variable { .. }),
                    "expected variable assignment target"
                );
                assert!(
                    matches!(*rhs, crate::parser::ast::Expr::ObjectLiteral { .. }),
                    "expected object literal rhs"
                );
            }
            ref other => panic!("expected assignment expression, got {:?}", other),
        },
        other => panic!("expected expression statement, got {:?}", other),
    }
}

#[test]
fn phpx_inserts_asi_before_newline_open_paren() {
    let code = "$a = { foo: \"bar\" }\n({ foo: $x } = $a)\n";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(
        program.errors.is_empty(),
        "unexpected parser errors: {:?}",
        program.errors
    );

    let expr_stmt_count = program
        .statements
        .iter()
        .filter(|stmt| matches!(***stmt, Stmt::Expression { .. }))
        .count();
    assert_eq!(expr_stmt_count, 2, "expected two expression statements");
}
