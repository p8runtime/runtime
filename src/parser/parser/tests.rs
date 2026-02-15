use bumpalo::Bump;
use std::path::Path;

use crate::parser::ast::{ClassKind, ClassMember, Expr, Stmt};
use crate::parser::lexer::Lexer;
use crate::parser::parser::{detect_parser_mode, Parser, ParserMode};

#[test]
fn detect_mode_treats_phpx_cache_php_as_internal() {
    let source = b"namespace deka_module_test;\nfunction x() { return 1; }\n";
    let path = Path::new("/tmp/php_modules/.cache/phpx/core/bridge.php");
    let mode = detect_parser_mode(source, Some(path));
    assert_eq!(mode, ParserMode::PhpxInternal);
}

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
fn phpx_rejects_missing_parameter_type() {
    let code = "function Name($props) { return $props; }";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(
        !program.errors.is_empty(),
        "expected parser error for missing parameter type"
    );
    assert!(
        program
            .errors
            .iter()
            .any(|err| err.message.contains("require explicit type annotations")),
        "expected explicit missing-type error, got: {:?}",
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
fn phpx_param_object_destructure_shorthand_uses_identifier_key() {
    let code = "interface NameProps { $name: string; } function FullName({ $name }: NameProps): string { return $name; }";
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
            let first_expr_stmt = body
                .iter()
                .find_map(|stmt| match &**stmt {
                    Stmt::Expression { expr, .. } => Some(*expr),
                    _ => None,
                })
                .expect("expected lowered destructuring assignment");

            match first_expr_stmt {
                Expr::Assign { expr, .. } => match expr {
                    Expr::PropertyFetch { property, .. } => match property {
                        Expr::String { value, .. } => {
                            assert_eq!(&value[..], b"name");
                        }
                        other => panic!("expected string key expression, got {:?}", other),
                    },
                    other => panic!("expected property fetch rhs, got {:?}", other),
                },
                other => panic!("expected assignment expression, got {:?}", other),
            }
        }
        other => panic!("expected function stmt, got {:?}", other),
    }
}

#[test]
fn phpx_parses_interface_shape_fields() {
    let code = "interface NameProps { $name: string; }";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(
        program.errors.is_empty(),
        "unexpected parser errors: {:?}",
        program.errors
    );

    let iface_stmt = program
        .statements
        .iter()
        .find(|s| matches!(***s, Stmt::Interface { .. }))
        .expect("expected interface statement");

    match &**iface_stmt {
        Stmt::Interface { members, .. } => {
            assert!(
                members.iter().any(|m| matches!(m, ClassMember::Property { .. })),
                "expected interface property member"
            );
        }
        other => panic!("expected interface stmt, got {:?}", other),
    }
}

#[test]
fn phpx_parses_async_function_and_await() {
    let code = "async function load($p: Promise<int>): Promise<int> {\n  return await $p\n}\n$v = await load($p)\n";
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
        Stmt::Function { is_async, body, .. } => {
            assert!(*is_async, "expected async function");
            let return_stmt = body
                .iter()
                .find(|stmt| matches!(***stmt, Stmt::Return { .. }))
                .expect("expected return in function body");
            match &**return_stmt {
                Stmt::Return { expr: Some(expr), .. } => {
                    assert!(matches!(**expr, Expr::Await { .. }), "expected await in return");
                }
                other => panic!("expected return with await expr, got {:?}", other),
            }
        }
        other => panic!("expected function stmt, got {:?}", other),
    }

    let has_tla_await = program.statements.iter().any(|stmt| {
        matches!(
            **stmt,
            Stmt::Expression {
                expr: Expr::Assign {
                    expr: Expr::Await { .. },
                    ..
                },
                ..
            }
        )
    });
    assert!(has_tla_await, "expected top-level await assignment");
}

#[test]
fn php_mode_rejects_await_syntax() {
    let code = "<?php await $value;";
    let arena = Bump::new();
    let mut parser = Parser::new(Lexer::new(code.as_bytes()), &arena);
    let program = parser.parse_program();

    assert!(
        program
            .errors
            .iter()
            .any(|err| err.message.contains("await is only available in PHPX mode")),
        "expected php mode await error, got: {:?}",
        program.errors
    );
}

#[test]
fn phpx_non_async_function_rejects_await() {
    let code = "function load($p: Promise<int>): Promise<int> { return await $p; }";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(
        program
            .errors
            .iter()
            .any(|err| err.message.contains("await is only allowed in async functions")),
        "expected non-async await error, got: {:?}",
        program.errors
    );
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

#[test]
fn phpx_parses_jsx_namespaced_client_directive_attributes() {
    let code = r#"
function IdleCard($props: object) {
  return <section>Idle</section>
}

function App($props: object) {
  return <div id="app">
    <IdleCard client:idle={true} />
  </div>
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
fn phpx_internal_parses_jsx_namespaced_client_directive_attributes() {
    let code = r#"
function App($props: object) {
  return <div id="app">
    <IdleCard client:idle={true} />
  </div>
}
"#;
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::PhpxInternal);
    let program = parser.parse_program();

    assert!(program.errors.is_empty(), "unexpected parser errors: {:?}", program.errors);
}

