use bumpalo::Bump;

use crate::parser::lexer::Lexer;
use crate::parser::parser::{Parser, ParserMode};
use crate::phpx::typeck::check_program;

fn check(code: &str) -> Result<(), String> {
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();
    if !program.errors.is_empty() {
        return Err("parse error".to_string());
    }
    check_program(&program, code.as_bytes()).map_err(|errs| {
        let mut out = String::new();
        for err in errs {
            out.push_str(&err.message);
            out.push('\n');
        }
        out
    })
}

#[test]
fn object_literal_dot_access_ok() {
    let code = "<?php $obj = { foo: 1 }; $obj.foo;";
    assert!(check(code).is_ok());
}

#[test]
fn object_literal_dot_access_missing_field_errors() {
    let code = "<?php $obj = { foo: 1 }; $obj.bar;";
    assert!(check(code).is_err());
}

#[test]
fn struct_default_type_mismatch_errors() {
    let code = "<?php struct Point { $x: int = \"nope\"; }";
    assert!(check(code).is_err());
}

#[test]
fn struct_default_unary_const_ok() {
    let code = "<?php struct Point { $x: int = -1; $y: float = +1.5; }";
    assert!(check(code).is_ok());
}

#[test]
fn return_type_widening_allows_int_to_float() {
    let code = "<?php function f(): float { return 1; }";
    assert!(check(code).is_ok());
}

#[test]
fn return_type_mismatch_errors() {
    let code = "<?php function f(): int { return 1.5; }";
    assert!(check(code).is_err());
}

#[test]
fn union_inference_allows_multiple_assignments() {
    let code = "<?php $x = 1; $x = 2.5; $x = 3;";
    assert!(check(code).is_ok());
}

#[test]
fn call_site_argument_mismatch_errors() {
    let code = "<?php function f(int $x) {} f(\"nope\");";
    assert!(check(code).is_err());
}

#[test]
fn object_shape_annotation_enforced() {
    let code = "<?php function f(Object<{ foo: int }> $x) {} f({ foo: 1 }); f({ bar: 2 });";
    assert!(check(code).is_err());
}

#[test]
fn call_return_type_infers_object_shape() {
    let code = "<?php function f(): Object<{ foo: int }> { return { foo: 1 }; } $x = f(); $x.foo;";
    assert!(check(code).is_ok());
}

#[test]
fn jsx_assignment_is_rejected() {
    let code = "<?php $v = <div>{ $x = 1 }</div>;";
    assert!(check(code).is_err());
}

#[test]
fn jsx_vnode_assignable_to_object() {
    let code = "<?php function View(): Object { return <div />; }";
    assert!(check(code).is_ok());
}

#[test]
fn jsx_vnode_not_assignable_to_int() {
    let code = "<?php function View(): int { return <div />; }";
    assert!(check(code).is_err());
}

#[test]
fn union_allows_object_shape_dot_access() {
    let code = "<?php $x = { foo: 1 }; $x = { foo: \"bar\" }; $x.foo;";
    assert!(check(code).is_ok());
}

#[test]
fn object_shape_optional_fields_allow_missing() {
    let code = "<?php function f(Object<{ foo?: int }> $x) {} f({}); f({ foo: 1 });";
    assert!(check(code).is_ok());
}

#[test]
fn object_shape_excess_property_errors() {
    let code = "<?php function f(Object<{ foo: int }> $x) {} f({ foo: 1, bar: 2 });";
    assert!(check(code).is_err());
}

#[test]
fn return_object_shape_excess_field_errors() {
    let code = "<?php function f(): Object<{ foo: int }> { return { foo: 1, bar: 2 }; }";
    assert!(check(code).is_err());
}

#[test]
fn null_literal_is_rejected() {
    let code = "<?php $x = null;";
    assert!(check(code).is_err());
}

#[test]
fn nullable_type_annotation_is_rejected() {
    let code = "<?php function f(?int $x) {}";
    assert!(check(code).is_err());
}

#[test]
fn option_allows_none_argument() {
    let code = "<?php function f(Option<int> $x) {} f(Option::None);";
    assert!(check(code).is_ok());
}

#[test]
fn option_allows_none_assignment_to_param() {
    let code = "<?php function f(Option<int> $x) { $x = Option::None; }";
    assert!(check(code).is_ok());
}

#[test]
fn option_some_argument_type_checks() {
    let code = "<?php function f(Option<int> $x) {} f(Option::Some(1));";
    assert!(check(code).is_ok());
}

#[test]
fn result_ok_err_argument_type_checks() {
    let code = "<?php function f(Result<int, string> $r) {} f(Result::Ok(1)); f(Result::Err(\"no\"));";
    assert!(check(code).is_ok());
}

#[test]
fn null_argument_to_non_option_errors() {
    let code = "<?php function f(int $x) {} f(null);";
    assert!(check(code).is_err());
}

#[test]
fn type_alias_object_shape_enforced() {
    let code = "<?php type Person = Object<{ foo: int }>; function f(Person $p) {} f({ foo: 1 }); f({ bar: 2 });";
    assert!(check(code).is_err());
}

#[test]
fn type_alias_sugar_object_shape_ok() {
    let code = "<?php type Person = { foo: int, bar?: string }; function f(Person $p) {} f({ foo: 1 });";
    assert!(check(code).is_ok());
}

#[test]
fn generic_type_alias_infers_type_param() {
    let code = "<?php type Box<T> = { value: T }; function unbox<T>(Box<T> $b): T { return $b.value; } $x = unbox({ value: 1 });";
    assert!(check(code).is_ok());
}

#[test]
fn generic_type_param_constraint_enforced() {
    let code = "<?php function f<T: int>(T $x) {} f(\"nope\");";
    assert!(check(code).is_err());
}

#[test]
fn interface_accepts_struct_with_matching_methods() {
    let code = "<?php interface Reader { public function read(int $n): string; } struct File { public function read(int $n): string { return \"\"; } } function useReader(Reader $r) {} useReader(File { });";
    assert!(check(code).is_ok());
}

#[test]
fn interface_rejects_struct_missing_method() {
    let code = "<?php interface Reader { public function read(int $n): string; } struct Bad { } function useReader(Reader $r) {} useReader(Bad { });";
    assert!(check(code).is_err());
}

#[test]
fn interface_constraint_enforced_for_type_param() {
    let code = "<?php interface Reader { public function read(int $n): string; } struct File { public function read(int $n): string { return \"\"; } } struct Bad { } function useReader<T: Reader>(T $r) {} useReader(File { }); useReader(Bad { });";
    assert!(check(code).is_err());
}

#[test]
fn struct_embed_promotes_fields() {
    let code = "<?php struct A { $x: int; } struct B { use A; } $b = B { $A: A { $x: 1 } }; $b.x;";
    assert!(check(code).is_ok());
}

#[test]
fn struct_embed_dot_access_infers_type() {
    let code = "<?php struct A { $x: int; } struct B { use A; } function takes(int $x) {} $b = B { $A: A { $x: 1 } }; takes($b.x);";
    assert!(check(code).is_ok());
}

#[test]
fn struct_embed_ambiguous_field_errors() {
    let code = "<?php struct A { $x: int; } struct B { $x: int; } struct C { use A, B; } $c = C { $A: A { $x: 1 }, $B: B { $x: 2 } }; $c.x;";
    assert!(check(code).is_err());
}

#[test]
fn enum_payload_call_type_checks() {
    let code = "<?php enum Msg { case Text(string $body); } $m = Msg::Text(\"hi\");";
    assert!(check(code).is_ok());
}

#[test]
fn enum_payload_call_mismatch_errors() {
    let code = "<?php enum Msg { case Text(string $body); } $m = Msg::Text(123);";
    assert!(check(code).is_err());
}

#[test]
fn enum_match_exhaustive_ok() {
    let code = "<?php enum Color { case Red; case Blue; } function f(Color $c): int { return match ($c) { Color::Red => 1, Color::Blue => 2 }; }";
    assert!(check(code).is_ok());
}

#[test]
fn enum_match_missing_case_errors() {
    let code = "<?php enum Color { case Red; case Blue; } function f(Color $c): int { return match ($c) { Color::Red => 1 }; }";
    assert!(check(code).is_err());
}

#[test]
fn enum_match_arm_narrows_payload_fields() {
    let code = "<?php enum Msg { case Text(string $body); case Ping; } function f(Msg $m): string { return match ($m) { Msg::Text => $m.body, Msg::Ping => \"ok\" }; }";
    assert!(check(code).is_ok());
}

#[test]
fn enum_match_arm_rejects_invalid_payload_field() {
    let code = "<?php enum Msg { case Text(string $body); case Ping; } function f(Msg $m): string { return match ($m) { Msg::Text => \"ok\", Msg::Ping => $m.body }; }";
    assert!(check(code).is_err());
}

#[test]
fn enum_payload_assignment_allows_dot_access() {
    let code = "<?php enum Msg { case Text(string $body); } $m = Msg::Text(\"hi\"); $m.body;";
    assert!(check(code).is_ok());
}

#[test]
fn null_comparison_is_rejected() {
    let code = "<?php $x = 1; if ($x === null) { $x = 2; }";
    assert!(check(code).is_err());
}

#[test]
fn enum_match_narrows_across_multiple_enums() {
    let code = "<?php enum A { case One(string $body); } enum B { case Two(string $body); } function f(A|B $x): string { return match ($x) { A::One, B::Two => $x.body }; }";
    let result = check(code);
    assert!(result.is_ok(), "{}", result.unwrap_err());
}

#[test]
fn match_expression_infers_union_for_arguments() {
    let code = "<?php function takesInt(int $x) {} $flag = true; takesInt(match ($flag) { true => 1, false => \"no\" });";
    assert!(check(code).is_err());
}

#[test]
fn generic_array_literal_infers_type_param() {
    let code = "<?php function takes<T>(array<T> $xs) {} takes([1, 2, 3]);";
    assert!(check(code).is_ok());
}

#[test]
fn generic_array_literal_inference_enforces_constraints() {
    let code = "<?php function takes<T: int>(array<T> $xs) {} takes([1, \"no\"]);";
    assert!(check(code).is_err());
}

#[test]
fn generic_option_infers_from_some() {
    let code = "<?php function takes<T>(Option<T> $x) {} takes(Option::Some(1));";
    assert!(check(code).is_ok());
}

#[test]
fn generic_option_none_requires_type() {
    let code = "<?php function takes<T>(Option<T> $x) {} takes(Option::None);";
    assert!(check(code).is_err());
}

#[test]
fn generic_result_infers_from_ok() {
    let code = "<?php function takes<T>(Result<T, string> $x) {} takes(Result::Ok(1));";
    assert!(check(code).is_ok());
}

#[test]
fn generic_result_infers_from_err() {
    let code = "<?php function takes<E>(Result<int, E> $x) {} takes(Result::Err(\"no\"));";
    assert!(check(code).is_ok());
}

#[test]
fn struct_method_call_type_checks() {
    let code = "<?php struct Reader { public function read(int $n): string { return \"\"; } } $r = Reader { }; $r->read(1);";
    assert!(check(code).is_ok());
}

#[test]
fn struct_method_call_mismatch_errors() {
    let code = "<?php struct Reader { public function read(int $n): string { return \"\"; } } $r = Reader { }; $r->read(\"no\");";
    assert!(check(code).is_err());
}

#[test]
fn interface_method_call_mismatch_errors() {
    let code = "<?php interface Reader { public function read(int $n): string; } function useReader(Reader $r) { $r->read(\"no\"); }";
    assert!(check(code).is_err());
}

#[test]
fn class_declaration_is_rejected() {
    let code = "<?php class Foo { }";
    assert!(check(code).is_err());
}

#[test]
fn interface_inheritance_is_rejected() {
    let code = "<?php interface A { } interface B extends A { }";
    assert!(check(code).is_err());
}

#[test]
fn new_on_class_is_rejected() {
    let code = "<?php $x = new Exception('nope');";
    assert!(check(code).is_err());
}

#[test]
fn anonymous_class_is_rejected() {
    let code = "<?php $x = new class { };";
    assert!(check(code).is_err());
}

#[test]
fn static_call_on_unknown_class_is_rejected() {
    let code = "<?php Foo::bar();";
    assert!(check(code).is_err());
}

#[test]
fn class_const_on_unknown_class_is_rejected() {
    let code = "<?php $x = Foo::BAR;";
    assert!(check(code).is_err());
}

#[test]
fn class_type_annotation_is_rejected() {
    let code = "<?php function f(Exception $e) {}";
    assert!(check(code).is_err());
}
