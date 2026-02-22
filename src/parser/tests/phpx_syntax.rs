use bumpalo::Bump;
use php_parser::ast::{ClassKind, ClassMember, Expr, ObjectKey, Stmt};
use php_parser::lexer::Lexer;
use php_parser::parser::{Parser, ParserMode};
use php_parser::ast::Type;

#[test]
fn parses_object_literal_and_dot_access_in_phpx() {
    let code = "<?php $var = { hello: \"world\", \"count\": 2 }; $var.hello;";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(program.errors.is_empty());

    let mut stmts = program
        .statements
        .iter()
        .filter(|s| !matches!(***s, Stmt::Nop { .. }));

    let assign_stmt = stmts.next().expect("expected assignment stmt");
    match **assign_stmt {
        Stmt::Expression { expr, .. } => match *expr {
            Expr::Assign { expr: rhs, .. } => match *rhs {
                Expr::ObjectLiteral { items, .. } => {
                    assert_eq!(items.len(), 2);
                    match items[0].key {
                        ObjectKey::Ident(token) => {
                            let text =
                                &code.as_bytes()[token.span.start..token.span.end];
                            assert_eq!(text, b"hello");
                        }
                        _ => panic!("expected ident key"),
                    }
                }
                other => panic!("expected object literal, got {:?}", other),
            },
            other => panic!("expected assignment, got {:?}", other),
        },
        other => panic!("expected expression stmt, got {:?}", other),
    }

    let dot_stmt = stmts.next().expect("expected dot access stmt");
    match **dot_stmt {
        Stmt::Expression { expr, .. } => match *expr {
            Expr::DotAccess { property, .. } => {
                let text = &code.as_bytes()[property.span.start..property.span.end];
                assert_eq!(text, b"hello");
            }
            other => panic!("expected dot access, got {:?}", other),
        },
        other => panic!("expected expression stmt, got {:?}", other),
    }
}

#[test]
fn parses_struct_in_phpx() {
    let code = "<?php struct Point { }";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(program.errors.is_empty());

    let stmt = program
        .statements
        .iter()
        .find(|s| !matches!(***s, Stmt::Nop { .. }))
        .expect("expected struct stmt");

    match **stmt {
        Stmt::Class { kind, .. } => assert_eq!(kind, ClassKind::Struct),
        other => panic!("expected struct, got {:?}", other),
    }
}

#[test]
fn parses_struct_use_composition_in_phpx() {
    let code = "<?php struct A { $x: int; } struct B { use A; }";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(program.errors.is_empty());

    let mut stmts = program
        .statements
        .iter()
        .filter(|s| !matches!(***s, Stmt::Nop { .. }));

    let _ = stmts.next().expect("expected struct A");
    let stmt = stmts.next().expect("expected struct B");

    match **stmt {
        Stmt::Class { kind, members, .. } => {
            assert_eq!(kind, ClassKind::Struct);
            let embed = members
                .iter()
                .find(|m| matches!(m, ClassMember::Embed { .. }))
                .expect("expected embed member");
            match embed {
                ClassMember::Embed { types, .. } => assert_eq!(types.len(), 1),
                _ => panic!("expected embed member"),
            }
        }
        other => panic!("expected struct, got {:?}", other),
    }
}

#[test]
fn parses_struct_literal_in_phpx() {
    let code = "<?php struct Point { $x: int; $y: int; } $p = Point { $x: 1, $y: 2 };";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(program.errors.is_empty());

    let mut stmts = program
        .statements
        .iter()
        .filter(|s| !matches!(***s, Stmt::Nop { .. }));

    let first = stmts.next().expect("expected struct stmt");
    match **first {
        Stmt::Class { kind, .. } => assert_eq!(kind, ClassKind::Struct),
        other => panic!("expected struct, got {:?}", other),
    }

    let second = stmts.next().expect("expected assignment stmt");
    match **second {
        Stmt::Expression { expr, .. } => match *expr {
            Expr::Assign { expr: rhs, .. } => match *rhs {
                Expr::StructLiteral { .. } => {}
                other => panic!("expected struct literal, got {:?}", other),
            },
            other => panic!("expected assignment, got {:?}", other),
        },
        other => panic!("expected expression stmt, got {:?}", other),
    }
}

#[test]
fn parses_struct_field_annotations_in_phpx() {
    let code = "struct User { $id: int @id @autoIncrement; }";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(program.errors.is_empty(), "unexpected parse errors: {:?}", program.errors);

    let stmt = program
        .statements
        .iter()
        .find(|s| !matches!(***s, Stmt::Nop { .. }))
        .expect("expected struct stmt");

    match **stmt {
        Stmt::Class { kind, members, .. } => {
            assert_eq!(kind, ClassKind::Struct);
            let field_member = members
                .iter()
                .find(|m| matches!(m, ClassMember::Property { .. }))
                .expect("expected struct field");
            match field_member {
                ClassMember::Property { entries, .. } => {
                    assert_eq!(entries.len(), 1);
                    assert_eq!(entries[0].annotations.len(), 2);
                }
                _ => panic!("expected property member"),
            }
        }
        other => panic!("expected struct, got {:?}", other),
    }
}

#[test]
fn parses_struct_field_annotation_args_in_phpx() {
    let code = "struct User { $email: string @index(\"users_email_idx\"); }";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(program.errors.is_empty(), "unexpected parse errors: {:?}", program.errors);

    let stmt = program
        .statements
        .iter()
        .find(|s| !matches!(***s, Stmt::Nop { .. }))
        .expect("expected struct stmt");

    match **stmt {
        Stmt::Class { kind, members, .. } => {
            assert_eq!(kind, ClassKind::Struct);
            let field_member = members
                .iter()
                .find(|m| matches!(m, ClassMember::Property { .. }))
                .expect("expected struct field");
            match field_member {
                ClassMember::Property { entries, .. } => {
                    assert_eq!(entries.len(), 1);
                    assert_eq!(entries[0].annotations.len(), 1);
                    assert_eq!(entries[0].annotations[0].args.len(), 1);
                }
                _ => panic!("expected property member"),
            }
        }
        other => panic!("expected struct, got {:?}", other),
    }
}

#[test]
fn parses_jsx_element_in_phpx() {
    let code = "<?php $v = <div class=\"x\">Hello { $name }</div>;";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(program.errors.is_empty());

    let stmt = program
        .statements
        .iter()
        .find(|s| !matches!(***s, Stmt::Nop { .. }))
        .expect("expected expression stmt");

    match **stmt {
        Stmt::Expression { expr, .. } => match *expr {
            Expr::Assign { expr: rhs, .. } => match *rhs {
                Expr::JsxElement { .. } => {}
                other => panic!("expected jsx element, got {:?}", other),
            },
            other => panic!("expected assignment, got {:?}", other),
        },
        other => panic!("expected expression stmt, got {:?}", other),
    }
}


#[test]
fn parses_jsx_namespaced_attribute_in_phpx() {
    let code = "<?php $v = <Card client:idle={true} />;";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(program.errors.is_empty(), "unexpected parse errors: {:?}", program.errors);
}

#[test]
fn jsx_object_literal_requires_double_braces() {
    let code = "<?php $v = <Component config={ foo: 'bar' } />;";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(!program.errors.is_empty());
    assert!(program
        .errors
        .iter()
        .any(|e| e.message.contains("Object literal requires double braces")));
}

#[test]
fn phpx_rejects_top_level_use() {
    let code = "<?php use Foo\\Bar; $x = 1;";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(!program.errors.is_empty());
}

#[test]
fn parses_object_shape_type() {
    let code = "<?php function f(Object<{ foo: int, \"bar-baz\": string }> $x) {}";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(program.errors.is_empty());

    let stmt = program
        .statements
        .iter()
        .find(|s| !matches!(***s, Stmt::Nop { .. }))
        .expect("expected function stmt");

    match **stmt {
        Stmt::Function { params, .. } => {
            let param = params.first().expect("expected param");
            let ty = param.ty.expect("expected type");
            match ty {
                Type::ObjectShape(fields) => {
                    assert_eq!(fields.len(), 2);
                }
                other => panic!("expected object shape type, got {:?}", other),
            }
        }
        other => panic!("expected function, got {:?}", other),
    }
}

#[test]
fn parses_type_alias_object_shape() {
    let code = "<?php type Person = Object<{ foo: int, \"bar-baz\"?: string }>; ";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(program.errors.is_empty());

    let stmt = program
        .statements
        .iter()
        .find(|s| !matches!(***s, Stmt::Nop { .. }))
        .expect("expected type alias stmt");

    match **stmt {
        Stmt::TypeAlias { ty, .. } => match ty {
            Type::ObjectShape(fields) => assert_eq!(fields.len(), 2),
            other => panic!("expected object shape type, got {:?}", other),
        },
        other => panic!("expected type alias, got {:?}", other),
    }
}

#[test]
fn parses_type_alias_sugar_object_shape() {
    let code = "<?php type Person = { foo: int };";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(program.errors.is_empty());

    let stmt = program
        .statements
        .iter()
        .find(|s| !matches!(***s, Stmt::Nop { .. }))
        .expect("expected type alias stmt");

    match **stmt {
        Stmt::TypeAlias { ty, .. } => match ty {
            Type::ObjectShape(fields) => assert_eq!(fields.len(), 1),
            other => panic!("expected object shape type, got {:?}", other),
        },
        other => panic!("expected type alias, got {:?}", other),
    }
}

#[test]
fn parses_generic_type_alias() {
    let code = "<?php type Box<T> = { value: T };";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(program.errors.is_empty());

    let stmt = program
        .statements
        .iter()
        .find(|s| !matches!(***s, Stmt::Nop { .. }))
        .expect("expected type alias stmt");

    match **stmt {
        Stmt::TypeAlias { type_params, ty, .. } => {
            assert_eq!(type_params.len(), 1);
            match ty {
                Type::ObjectShape(fields) => assert_eq!(fields.len(), 1),
                other => panic!("expected object shape type, got {:?}", other),
            }
        }
        other => panic!("expected type alias, got {:?}", other),
    }
}

#[test]
fn parses_enum_case_payload_in_phpx() {
    let code = "<?php enum Msg { case Text(string $body, int $len); }";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();

    assert!(program.errors.is_empty());

    let stmt = program
        .statements
        .iter()
        .find(|s| !matches!(***s, Stmt::Nop { .. }))
        .expect("expected enum stmt");

    match **stmt {
        Stmt::Enum { members, .. } => {
            let case = members.iter().find_map(|member| {
                if let php_parser::ast::ClassMember::Case { payload, .. } = member {
                    Some(payload)
                } else {
                    None
                }
            });
            let payload = case.expect("expected enum case");
            let payload = payload.expect("expected payload params");
            assert_eq!(payload.len(), 2);
        }
        other => panic!("expected enum, got {:?}", other),
    }
}
