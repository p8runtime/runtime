use bumpalo::Bump;
use std::path::Path;

use crate::parser::ast::{ClassKind, ClassMember, Expr, Stmt};
use crate::parser::lexer::Lexer;
use crate::parser::parser::{Parser, ParserMode, detect_parser_mode};

#[test]
fn detect_mode_treats_phpx_cache_php_as_internal() {
    let source = b"namespace deka_module_test;\nfunction x() { return 1; }\n";
    let path = Path::new("/tmp/php_modules/.cache/phpx/core/bridge.php");
    let mode = detect_parser_mode(source, Some(path));
    assert_eq!(mode, ParserMode::PhpxInternal);
}

#[test]
fn detect_mode_treats_ds_extension_as_dekascript() {
    assert_eq!(
        detect_parser_mode(b"const answer = 42;", Some(Path::new("lesson.ds"))),
        ParserMode::Ds
    );
}

#[test]
fn ds_parses_bare_typed_parameters_and_const() {
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(b"const answer = 42; function add(left: number, right: number): number { return left + right; }"), &arena, ParserMode::Ds);
    let program = parser.parse_program();
    assert!(
        program.errors.is_empty(),
        "unexpected errors: {:?}",
        program.errors
    );
}

#[test]
fn ds_rejects_php_sigil_parameters_with_actionable_diagnostic() {
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(
        Lexer::new(b"function add($value: number): number { return $value; }"),
        &arena,
        ParserMode::Ds,
    );
    let program = parser.parse_program();
    assert!(
        program
            .errors
            .iter()
            .any(|error| error.message == "DekaScript parameters use bare identifiers")
    );
}

#[test]
fn ds_parses_let_and_for_of_without_php_foreach() {
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(
        Lexer::new(b"let total = 0; for (const item of items) { total += item; }"),
        &arena,
        ParserMode::Ds,
    );
    let program = parser.parse_program();
    assert!(
        program.errors.is_empty(),
        "unexpected errors: {:?}",
        program.errors
    );
}

#[test]
fn phpx_allows_automatic_semicolons() {
    let code = "$a = 1\n$b = 2\necho $a\n";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(
        program.errors.is_empty(),
        "unexpected errors: {:?}",
        program.errors
    );
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

    assert!(
        program.errors.is_empty(),
        "unexpected errors: {:?}",
        program.errors
    );

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
                Stmt::Return { expr, .. } => {
                    assert!(expr.is_none(), "expected return without expr")
                }
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

    assert!(
        program.errors.is_empty(),
        "unexpected errors: {:?}",
        program.errors
    );

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

    assert!(
        program.errors.is_empty(),
        "unexpected errors: {:?}",
        program.errors
    );

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
fn ds_parses_struct_with_bare_field_names() {
    let code = "struct Point { x: int; y: int }";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Ds);
    let program = parser.parse_program();

    assert!(
        program.errors.is_empty(),
        "unexpected errors: {:?}",
        program.errors
    );

    let stmt = program
        .statements
        .iter()
        .find(|s| !matches!(***s, Stmt::Nop { .. }))
        .expect("expected struct stmt");

    match &**stmt {
        Stmt::Class { kind, members, .. } => {
            assert_eq!(*kind, ClassKind::Struct);
            let fields: Vec<_> = members
                .iter()
                .filter(|m| matches!(m, ClassMember::Property { .. }))
                .collect();
            assert_eq!(fields.len(), 2, "expected two struct fields");
        }
        other => panic!("expected struct stmt, got {:?}", other),
    }
}

#[test]
fn ds_parses_struct_literal_with_bare_field_names() {
    let code = "struct Point { x: int; y: int } const p = Point { x: 3, y: 4 };";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Ds);
    let program = parser.parse_program();

    assert!(
        program.errors.is_empty(),
        "unexpected errors: {:?}",
        program.errors
    );
}

#[test]
fn ds_still_accepts_dollar_sigil_in_structs_for_back_compat() {
    // dekaruntime/deka#93: migration window — `$x` syntax continues to parse
    // while DekaScript transitions to bare identifiers.
    let code = "struct Point { $x: int; $y: int } const p = Point { $x: 3, $y: 4 };";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Ds);
    let program = parser.parse_program();

    assert!(
        program.errors.is_empty(),
        "unexpected errors: {:?}",
        program.errors
    );
}

#[test]
fn phpx_still_requires_dollar_sigil_in_struct_fields() {
    let code = "struct Point { x: int }";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(
        !program.errors.is_empty(),
        "expected parser error for bare struct field in PHPX"
    );
    assert!(
        program
            .errors
            .iter()
            .any(|err| err.message.contains("struct fields must use `$name: Type` syntax")),
        "expected PHPX sigil error, got: {:?}",
        program.errors
    );
}

#[test]
fn phpx_parses_colon_typed_parameters() {
    let code = "function Name($props: Object<{ name: string }>): string { return $props.name; }";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(
        program.errors.is_empty(),
        "unexpected errors: {:?}",
        program.errors
    );
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
fn phpx_allows_untyped_parameter() {
    // PHPX mode does not enforce type annotations at the parser level;
    // type checking is handled by the typechecker (gated behind PHPX_STRICT_JSX_TYPES).
    let code = "function Name($props) { return $props; }";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(
        program.errors.is_empty(),
        "expected no parser errors for untyped parameter, got: {:?}",
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
                members
                    .iter()
                    .any(|m| matches!(m, ClassMember::Property { .. })),
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
                Stmt::Return {
                    expr: Some(expr), ..
                } => {
                    assert!(
                        matches!(**expr, Expr::Await { .. }),
                        "expected await in return"
                    );
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
        program.errors.iter().any(|err| err
            .message
            .contains("await is only allowed in async functions")),
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
    let mut parser = Parser::new_with_mode(
        Lexer::new(code.as_bytes()),
        &arena,
        ParserMode::PhpxInternal,
    );
    let program = parser.parse_program();

    assert!(
        program.errors.is_empty(),
        "unexpected parser errors: {:?}",
        program.errors
    );
}

#[test]
fn cql_parses_simple_query() {
    let code = "cql results = MATCH (n:Person) RETURN n;";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(
        program.errors.is_empty(),
        "cql parse errors: {:?}",
        program.errors
    );

    let stmts: Vec<_> = program
        .statements
        .iter()
        .filter(|s| !matches!(***s, Stmt::Nop { .. }))
        .collect();
    assert_eq!(stmts.len(), 1);

    match **stmts[0] {
        Stmt::Expression { expr, .. } => match expr {
            Expr::Cql {
                name,
                cypher,
                params,
                ..
            } => {
                let name_text = &code.as_bytes()[name.span.start..name.span.end];
                assert_eq!(name_text, b"results");

                let cypher_text = &code.as_bytes()[cypher.start..cypher.end];
                assert!(cypher_text.starts_with(b"MATCH"));

                assert!(params.is_empty());
            }
            _ => panic!("expected Expr::Cql, got {:?}", expr),
        },
        _ => panic!("expected Stmt::Expression"),
    }
}

#[test]
fn cql_extracts_dollar_params() {
    let code =
        "cql recs = MATCH (c:Customer) WHERE c.id = $customer_id AND c.age > $min_age RETURN c;";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(
        program.errors.is_empty(),
        "cql parse errors: {:?}",
        program.errors
    );

    let stmts: Vec<_> = program
        .statements
        .iter()
        .filter(|s| !matches!(***s, Stmt::Nop { .. }))
        .collect();

    match **stmts[0] {
        Stmt::Expression { expr, .. } => match expr {
            Expr::Cql { params, .. } => {
                assert_eq!(params.len(), 2);
                assert_eq!(params[0].name, b"customer_id");
                assert_eq!(params[1].name, b"min_age");
            }
            _ => panic!("expected Expr::Cql"),
        },
        _ => panic!("expected Stmt::Expression"),
    }
}

#[test]
fn query_keyword_works_as_alias() {
    let code = "query items = MATCH (p:Product) RETURN p.name;";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(
        program.errors.is_empty(),
        "query parse errors: {:?}",
        program.errors
    );

    let stmts: Vec<_> = program
        .statements
        .iter()
        .filter(|s| !matches!(***s, Stmt::Nop { .. }))
        .collect();

    match **stmts[0] {
        Stmt::Expression { expr, .. } => match expr {
            Expr::Cql { name, .. } => {
                let name_text = &code.as_bytes()[name.span.start..name.span.end];
                assert_eq!(name_text, b"items");
            }
            _ => panic!("expected Expr::Cql"),
        },
        _ => panic!("expected Stmt::Expression"),
    }
}

#[test]
fn cql_error_on_missing_name() {
    let code = "cql = MATCH (n) RETURN n;";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(
        !program.errors.is_empty(),
        "expected an error for missing cql binding name"
    );
}

#[test]
fn query_as_function_call_not_keyword() {
    // `query(...)` should parse as a function call, not a cql statement
    let code = "query($handle, $cypher);";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(
        program.errors.is_empty(),
        "query() should parse as function call, got errors: {:?}",
        program.errors
    );

    let stmts: Vec<_> = program
        .statements
        .iter()
        .filter(|s| !matches!(***s, Stmt::Nop { .. }))
        .collect();
    assert_eq!(stmts.len(), 1);

    // Should be a function call expression, not Expr::Cql
    match **stmts[0] {
        Stmt::Expression { expr, .. } => {
            assert!(
                !matches!(expr, Expr::Cql { .. }),
                "query() should NOT be parsed as Expr::Cql"
            );
            assert!(
                matches!(expr, Expr::Call { .. }),
                "query() should be parsed as a function call"
            );
        }
        _ => panic!("expected Stmt::Expression"),
    }
}

#[test]
fn cql_binding_is_accessible_as_variable() {
    // After `cql results = ...`, $results should be a valid variable
    let code = r#"
function test() {
    $id = 42;
    cql results = MATCH (n) WHERE n.id = $id RETURN n;
    $x = $results;
}
"#;
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(
        program.errors.is_empty(),
        "parse errors: {:?}",
        program.errors
    );
}

#[test]
fn phpx_parses_bytes_type_in_param_and_return() {
    let code = "function encode($input: bytes): bytes { return $input; }";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(
        program.errors.is_empty(),
        "parse errors: {:?}",
        program.errors
    );
}

#[test]
fn phpx_parses_bytes_type_in_struct_field() {
    let code = "struct Packet { $payload: bytes; $len: int; }";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(
        program.errors.is_empty(),
        "parse errors: {:?}",
        program.errors
    );
}

#[test]
fn phpx_parses_bytes_union_type() {
    let code = "function maybe_bytes(): bytes|string { return ''; }";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(
        program.errors.is_empty(),
        "parse errors: {:?}",
        program.errors
    );
}

#[test]
fn lexer_recognizes_bytes_as_type_keyword() {
    use crate::parser::lexer::token::TokenKind;
    let code = "<?php bytes";
    let mut lexer = Lexer::new(code.as_bytes());
    lexer.start_in_scripting();
    // Skip the open tag token if the lexer emits it.
    let token = lexer.find(|t| t.kind == TokenKind::TypeBytes).expect("expected TypeBytes token");
    assert_eq!(token.kind, TokenKind::TypeBytes);
    assert_eq!(&code.as_bytes()[token.span.start..token.span.end], b"bytes");
}

#[test]
fn ds_parses_unsafe_block_with_catch_and_finally() {
    let code = r#"const val = unsafe { JSON.parse(text) } catch (e) { defaultValue } finally { cleanup() };"#;
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Ds);
    let program = parser.parse_program();

    assert!(
        program.errors.is_empty(),
        "parse errors: {:?}",
        program.errors
    );

    let unsafe_expr = program
        .statements
        .iter()
        .find_map(|s| match **s {
            Stmt::Const { consts, .. } => consts.first().map(|c| c.value),
            _ => None,
        })
        .expect("expected const declaration");
    assert!(
        matches!(unsafe_expr, Expr::Unsafe { .. }),
        "expected unsafe expression, got {:?}",
        unsafe_expr
    );
}

#[test]
fn ds_parses_unsafe_block_without_catch_or_finally() {
    let code = "const val = unsafe { JSON.parse(text) };";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Ds);
    let program = parser.parse_program();

    assert!(
        program.errors.is_empty(),
        "parse errors: {:?}",
        program.errors
    );

    let unsafe_expr = program
        .statements
        .iter()
        .find_map(|s| match **s {
            Stmt::Const { consts, .. } => consts.first().map(|c| c.value),
            _ => None,
        })
        .expect("expected const declaration");
    match unsafe_expr {
        Expr::Unsafe { catch, finally, .. } => {
            assert!(catch.is_none(), "expected no catch clause");
            assert!(finally.is_none(), "expected no finally clause");
        }
        _ => panic!("expected unsafe expression, got {:?}", unsafe_expr),
    }
}

#[test]
fn lexer_recognizes_unsafe_keyword() {
    use crate::parser::lexer::token::TokenKind;
    let code = "unsafe";
    let mut lexer = Lexer::new(code.as_bytes());
    lexer.start_in_scripting();
    let token = lexer.next().expect("expected a token");
    assert_eq!(token.kind, TokenKind::Unsafe);
    assert_eq!(&code.as_bytes()[token.span.start..token.span.end], b"unsafe");
}

#[test]
fn ds_parses_js_style_enum_body() {
    let code = "enum Status { Loading, Ready, Failed }";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Ds);
    let program = parser.parse_program();

    assert!(program.errors.is_empty(), "errors: {:?}", program.errors);

    let stmt = program
        .statements
        .iter()
        .find(|s| matches!(***s, Stmt::Enum { .. }))
        .expect("expected enum stmt");

    match **stmt {
        Stmt::Enum { members, .. } => {
            assert_eq!(members.len(), 3);
            for member in members.iter() {
                assert!(matches!(member, ClassMember::Case { .. }));
            }
        }
        _ => panic!("expected enum stmt"),
    }
}

#[test]
fn ds_parses_js_style_enum_body_with_payload() {
    let code = "enum Option<T> { Some(T), None }";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Ds);
    let program = parser.parse_program();

    assert!(program.errors.is_empty(), "errors: {:?}", program.errors);

    let stmt = program
        .statements
        .iter()
        .find(|s| matches!(***s, Stmt::Enum { .. }))
        .expect("expected enum stmt");

    match **stmt {
        Stmt::Enum {
            type_params,
            members,
            ..
        } => {
            assert_eq!(type_params.len(), 1);
            assert_eq!(members.len(), 2);

            let payloads: Vec<_> = members
                .iter()
                .filter_map(|member| match member {
                    ClassMember::Case { payload, .. } => Some(*payload),
                    _ => None,
                })
                .collect();
            assert_eq!(payloads.len(), 2);
            assert!(payloads[0].is_some(), "expected Some(T) payload");
            assert_eq!(payloads[0].unwrap().len(), 1);
            assert!(payloads[1].is_none(), "expected None to have no payload");
        }
        _ => panic!("expected enum stmt"),
    }
}

#[test]
fn phpx_accepts_case_form_enum_body() {
    // Backward compatibility: PHPX mode still accepts PHP-style `case Name;`
    // enum members.
    let code = "enum Status { case Loading; case Ready; case Failed; }";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(program.errors.is_empty(), "errors: {:?}", program.errors);

    let stmt = program
        .statements
        .iter()
        .find(|s| matches!(***s, Stmt::Enum { .. }))
        .expect("expected enum stmt");

    match **stmt {
        Stmt::Enum { members, .. } => {
            assert_eq!(members.len(), 3);
            for member in members.iter() {
                assert!(matches!(member, ClassMember::Case { .. }));
            }
        }
        _ => panic!("expected enum stmt"),
    }
}

#[test]
fn phpx_rejects_js_style_enum_body() {
    // PHPX mode requires `case Name;` and does not accept JS-style lists.
    let code = "enum Status { Loading, Ready, Failed }";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(!program.errors.is_empty());
}
