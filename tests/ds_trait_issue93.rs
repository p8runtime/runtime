use bumpalo::Bump;
use php_rs::parser::ast::{ClassKind, ClassMember, Stmt};
use php_rs::parser::lexer::Lexer;
use php_rs::parser::parser::{Parser, ParserMode};

fn parse_ds(code: &str) -> Vec<String> {
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Ds);
    let program = parser.parse_program();
    program.errors.iter().map(|e| e.message.to_string()).collect()
}

#[test]
fn ds_trait_declaration_parses() {
    let code = r#"trait Named {
  name(): string
}"#;
    let errors = parse_ds(code);
    assert!(errors.is_empty(), "unexpected parse errors: {:?}", errors);
}

#[test]
fn ds_trait_default_method_parses() {
    let code = r#"trait Greeter {
  greet(): string { return "hi" }
}"#;
    let errors = parse_ds(code);
    assert!(errors.is_empty(), "unexpected parse errors: {:?}", errors);
}

#[test]
fn ds_inherent_impl_parses() {
    let code = r#"struct Point { x: int; y: int }
impl Point {
  norm(): int { return this.x + this.y }
}"#;
    let errors = parse_ds(code);
    assert!(errors.is_empty(), "unexpected parse errors: {:?}", errors);
}

#[test]
fn ds_trait_impl_parses() {
    let code = r#"trait Named {
  name(): string
}
struct User {
  handle: string
}
impl Named for User {
  name(): string {
    return this.handle
  }
}"#;
    let errors = parse_ds(code);
    assert!(errors.is_empty(), "unexpected parse errors: {:?}", errors);
}

#[test]
fn ds_trait_impl_rejects_function_keyword() {
    // `function` is no longer accepted in DekaScript declaration contexts;
    // bare-method syntax is used inside trait/impl bodies.
    let code = r#"trait Named {
  function name(): string
}
struct User { handle: string }
impl Named for User {
  function name(): string { return this.handle }
}"#;
    let errors = parse_ds(code);
    assert!(
        errors.iter().any(|e| e.contains("uses `fn` for function declarations")),
        "expected `function` rejection, got: {:?}",
        errors
    );
}

#[test]
fn ds_trait_method_call_expression_parses() {
    let code = r#"trait Named {
  name(): string
}
struct User {
  handle: string
}
impl Named for User {
  name(): string {
    return this.handle
  }
}
const user = User { handle: "deka" }
console.log(user.name())
"#;
    let errors = parse_ds(code);
    assert!(errors.is_empty(), "unexpected parse errors: {:?}", errors);
}

#[test]
fn ds_trait_method_with_parameters_parses() {
    let code = r#"trait Named {
  name(prefix: string): string
}
struct User { handle: string }
impl Named for User {
  name(prefix: string): string {
    return prefix + this.handle
  }
}"#;
    let errors = parse_ds(code);
    assert!(errors.is_empty(), "unexpected parse errors: {:?}", errors);
}

#[test]
fn ds_trait_method_without_return_type_parses() {
    let code = r#"trait Printer {
  print()
}"#;
    let errors = parse_ds(code);
    assert!(errors.is_empty(), "unexpected parse errors: {:?}", errors);
}

#[test]
fn ds_trait_impl_ast_shape_is_correct() {
    let code = r#"trait Named { name(): string }
struct User { handle: string }
impl Named for User { name(): string { return this.handle } }"#;
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Ds);
    let program = parser.parse_program();
    assert!(program.errors.is_empty(), "{:?}", program.errors);

    let mut stmts = program
        .statements
        .iter()
        .filter(|s| !matches!(***s, Stmt::Nop { .. }));

    let trait_stmt = stmts.next().expect("expected trait");
    match **trait_stmt {
        Stmt::Trait { name, .. } => {
            assert_eq!(&code.as_bytes()[name.span.start..name.span.end], b"Named");
        }
        _ => panic!("expected trait statement"),
    }

    let struct_stmt = stmts.next().expect("expected struct");
    match **struct_stmt {
        Stmt::Class { kind, .. } => assert_eq!(kind, ClassKind::Struct),
        _ => panic!("expected struct statement"),
    }

    let impl_stmt = stmts.next().expect("expected impl");
    match **impl_stmt {
        Stmt::Impl {
            trait_name,
            target,
            members,
            ..
        } => {
            assert!(trait_name.is_some());
            let trait_name = trait_name.unwrap();
            assert_eq!(
                &code.as_bytes()[trait_name.span.start..trait_name.span.end],
                b"Named"
            );
            assert_eq!(
                &code.as_bytes()[target.span.start..target.span.end],
                b"User"
            );
            assert_eq!(members.len(), 1);
            assert!(matches!(members[0], ClassMember::Method { .. }));
        }
        _ => panic!("expected impl statement"),
    }
}
