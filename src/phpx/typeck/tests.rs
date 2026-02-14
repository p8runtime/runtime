use bumpalo::Bump;
use std::path::Path;

use crate::parser::lexer::Lexer;
use crate::parser::parser::{Parser, ParserMode};
use crate::phpx::typeck::{check_program, check_program_with_path};

fn normalize_phpx_snippet(code: &str) -> &str {
    let trimmed = code.trim_start();
    if let Some(rest) = trimmed.strip_prefix("<?php") {
        return rest;
    }
    if let Some(rest) = trimmed.strip_prefix("<?") {
        return rest;
    }
    code
}

fn check(code: &str) -> Result<(), String> {
    let code = normalize_phpx_snippet(code);
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();
    if !program.errors.is_empty() {
        let mut out = String::new();
        for err in program.errors {
            out.push_str(&err.message);
            out.push('\n');
        }
        return Err(out);
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

fn check_with_path(code: &str, path: &str) -> Result<(), String> {
    let code = normalize_phpx_snippet(code);
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();
    if !program.errors.is_empty() {
        let mut out = String::new();
        for err in program.errors {
            out.push_str(&err.message);
            out.push('\n');
        }
        return Err(out);
    }
    check_program_with_path(&program, code.as_bytes(), Some(Path::new(path))).map_err(|errs| {
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
fn struct_default_allows_struct_and_object_literals() {
    let code = "<?php
        struct Point { $x: int = 0; $y: int = 0; }
        struct Box { $pos: Point = Point { $x: 1, $y: 2 }; $meta: Object = { foo: 'bar' }; }
    ";
    assert!(check(code).is_ok());
}

#[test]
fn struct_field_annotations_basic_ok() {
    let code = "struct User { $id: int @id @autoIncrement; }";
    let res = check(code);
    assert!(res.is_ok(), "expected ok, got: {:?}", res);
}

#[test]
fn struct_field_annotation_duplicate_errors() {
    let code = "struct User { $id: int @id @id; }";
    assert!(check(code).is_err());
}

#[test]
fn struct_field_annotation_unknown_errors() {
    let code = "struct User { $id: int @banana; }";
    assert!(check(code).is_err());
}

#[test]
fn struct_field_annotation_autoincrement_requires_int() {
    let code = "struct User { $id: string @autoIncrement; }";
    assert!(check(code).is_err());
}

#[test]
fn struct_field_annotation_map_requires_string_arg() {
    let code = "struct User { $name: string @map(123); }";
    assert!(check(code).is_err());
}

#[test]
fn struct_field_annotation_relation_basic_ok() {
    let code = "struct Post { $id: int @id; } struct User { $posts: array<Post> @relation(\"hasMany\", \"Post\", \"authorId\"); }";
    let res = check(code);
    assert!(res.is_ok(), "expected ok, got: {:?}", res);
}

#[test]
fn struct_field_annotation_relation_requires_string_args() {
    let code = "struct User { $posts: array<Post> @relation(123, \"Post\", \"authorId\"); }";
    assert!(check(code).is_err());
}

#[test]
fn struct_field_annotation_relation_requires_hasmany_array_field() {
    let code = "struct User { $post: Post @relation(\"hasMany\", \"Post\", \"authorId\"); }";
    assert!(check(code).is_err());
}

#[test]
fn struct_field_annotation_relation_model_mismatch_errors() {
    let code = "struct Post { $id: int @id; } struct User { $posts: array<Post> @relation(\"hasMany\", \"User\", \"authorId\"); }";
    assert!(check(code).is_err());
}

#[test]
fn struct_field_annotation_relation_belongsto_fk_missing_errors() {
    let code = "struct User { $id: int @id; } struct Post { $author: User @relation(\"belongsTo\", \"User\", \"authorId\"); }";
    assert!(check(code).is_err());
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
fn deka_wasm_call_forbidden_outside_internals() {
    let code = "__deka_wasm_call('__deka_db', 'open', {})";
    let res = check_with_path(code, "/tmp/app/index.phpx");
    assert!(res.is_err());
    assert!(res
        .unwrap_err()
        .contains("__deka_wasm_call is internal-only"));
}

#[test]
fn deka_wasm_call_async_forbidden_outside_internals() {
    let code = "__deka_wasm_call_async('__deka_db', 'open', {})";
    let res = check_with_path(code, "/tmp/app/index.phpx");
    assert!(res.is_err());
    assert!(res
        .unwrap_err()
        .contains("__deka_wasm_call_async is internal-only"));
}

#[test]
fn deka_wasm_call_allowed_inside_internals() {
    let code = "__deka_wasm_call('__deka_db', 'open', {})";
    let res = check_with_path(code, "/tmp/app/php_modules/internals/wasm.phpx");
    assert!(res.is_ok());
}

#[test]
fn deka_wasm_call_async_allowed_inside_internals() {
    let code = "__deka_wasm_call_async('__deka_db', 'open', {})";
    let res = check_with_path(code, "/tmp/app/php_modules/internals/wasm.phpx");
    assert!(res.is_ok());
}

#[test]
fn bridge_forbidden_outside_core() {
    let code = "__bridge('db', 'open', {})";
    let res = check_with_path(code, "/tmp/app/index.phpx");
    assert!(res.is_err());
    assert!(res.unwrap_err().contains("__bridge is internal-only"));
}

#[test]
fn bridge_async_forbidden_outside_core() {
    let code = "__bridge_async('db', 'open', {})";
    let res = check_with_path(code, "/tmp/app/index.phpx");
    assert!(res.is_err());
    assert!(res.unwrap_err().contains("__bridge_async is internal-only"));
}

#[test]
fn bridge_allowed_inside_core() {
    let code = "__bridge('db', 'open', {})";
    let res = check_with_path(code, "/tmp/app/php_modules/core/bridge.phpx");
    assert!(res.is_ok());
}

#[test]
fn bridge_async_allowed_inside_core() {
    let code = "__bridge_async('db', 'open', {})";
    let res = check_with_path(code, "/tmp/app/php_modules/core/bridge.phpx");
    assert!(res.is_ok());
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
fn jsx_component_requires_typed_props_param() {
    let code = "<?php function FullName($name) { return $name; } $v = <FullName name=\"Bob\" />;";
    assert!(check(code).is_err());
}

#[test]
fn jsx_component_typed_props_param_is_allowed() {
    let code = "interface FullNameProps { $name: string; } function FullName($props: FullNameProps): string { return $props.name; } $v = <FullName name='Bob' />;";
    assert!(check(code).is_ok());
}

#[test]
fn jsx_component_unknown_prop_suggests_expected_name() {
    let code = "interface FullNameProps { $name: string; } function FullName($props: FullNameProps): string { return $props.name; } $v = <FullName nam='Bob' />;";
    let err = check(code).expect_err("expected unknown prop to fail");
    assert!(
        err.contains("Unknown prop 'nam'") && err.contains("did you mean 'name'"),
        "expected prop suggestion, got: {}",
        err
    );
}

#[test]
fn jsx_component_missing_required_prop_errors() {
    let code = "interface FullNameProps { $name: string; } function FullName($props: FullNameProps): string { return $props.name; } $v = <FullName />;";
    let err = check(code).expect_err("expected missing required prop to fail");
    assert!(
        err.contains("Missing required prop 'name'"),
        "expected required prop error, got: {}",
        err
    );
}

#[test]
fn jsx_component_missing_required_prop_errors_when_nested() {
    let code = "interface FullNameProps { $name: string; } function FullName($props: FullNameProps): string { return $props.name; } $v = <div><FullName /></div>;";
    let err = check(code).expect_err("expected nested missing required prop to fail");
    assert!(
        err.contains("Missing required prop 'name'"),
        "expected required prop error, got: {}",
        err
    );
}

#[test]
fn jsx_component_struct_props_is_rejected_with_guidance() {
    let code = "struct FullNameProps { $name: string; } function FullName($props: FullNameProps): string { return $props.name; } $v = <FullName name='Bob' />;";
    let err = check(code).expect_err("expected struct props to be rejected");
    assert!(
        err.contains("cannot be a struct") && err.contains("use interface"),
        "expected guidance in error, got: {}",
        err
    );
}

#[test]
fn destructured_param_struct_type_is_rejected_with_guidance() {
    let code = "struct NameProps { $name: string; } function FullName({ $name }: NameProps): string { return $name; }";
    let err = check(code).expect_err("expected destructured struct param to be rejected");
    assert!(
        err.contains("Destructured parameter") && err.contains("use interface"),
        "expected guidance in error, got: {}",
        err
    );
}

#[test]
fn unknown_variable_suggests_nearby_name() {
    let code = "function fullName($name: string): string { return $nam; }";
    let err = check(code).expect_err("expected unknown variable diagnostic");
    assert!(
        err.contains("Unknown variable '$nam'") && err.contains("did you mean '$name'"),
        "expected variable suggestion, got: {}",
        err
    );
}

#[test]
fn await_in_non_async_function_errors() {
    let code = "function load($p: Promise<int>): int { return await $p; }";
    let err = check(code).expect_err("expected await in non-async function to fail");
    assert!(
        err.contains("await is only allowed in async functions"),
        "expected async-context error, got: {}",
        err
    );
}

#[test]
fn await_unwraps_promise_in_async_function() {
    let code = "async function load($p: Promise<int>): Promise<int> { return await $p; }";
    let res = check(code);
    assert!(res.is_ok(), "expected ok, got: {:?}", res);
}

#[test]
fn await_non_promise_errors() {
    let code = "async function load($x: int): Promise<int> { return await $x; }";
    let err = check(code).expect_err("expected await non-promise to fail");
    assert!(
        err.contains("await expects Promise<T>"),
        "expected promise type error, got: {}",
        err
    );
}

#[test]
fn async_function_requires_promise_return_type() {
    let code = "async function load($p: Promise<int>): int { return await $p; }";
    let err = check(code).expect_err("expected async return type enforcement");
    assert!(
        err.contains("Async function must declare Promise<T> return type"),
        "expected Promise<T> return error, got: {}",
        err
    );
}

#[test]
fn await_promise_result_flows_into_result_typed_param() {
    let code = "type LoadResult = Result<int, string>;\nfunction consume($r: LoadResult): int { return 1; }\nasync function load($p: Promise<LoadResult>): Promise<int> {\n  $r = await $p;\n  consume($r);\n  return 1;\n}";
    let res = check(code);
    assert!(res.is_ok(), "expected ok, got: {:?}", res);
}

#[test]
fn union_allows_object_shape_dot_access() {
    let code = "<?php $x = { foo: 1 }; $x = { foo: \"bar\" }; $x.foo;";
    assert!(check(code).is_ok());
}

#[test]
fn object_shape_optional_fields_allow_missing() {
    let code = "<?php function f($x: Object<{ foo?: int }>) {} f({}); f({ foo: 1 });";
    assert!(check(code).is_ok());
}

#[test]
fn object_shape_excess_property_errors() {
    let code = "<?php function f($x: Object<{ foo: int }>) {} f({ foo: 1, bar: 2 });";
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
    let code = "<?php function f($x: ?int) {}";
    assert!(check(code).is_err());
}

#[test]
fn option_allows_none_argument() {
    let code = "<?php function f($x: Option<int>) {} f(Option::None);";
    assert!(check(code).is_ok());
}

#[test]
fn option_allows_none_assignment_to_param() {
    let code = "<?php function f($x: Option<int>) { $x = Option::None; }";
    assert!(check(code).is_ok());
}

#[test]
fn option_some_argument_type_checks() {
    let code = "<?php function f($x: Option<int>) {} f(Option::Some(1));";
    assert!(check(code).is_ok());
}

#[test]
fn result_ok_err_argument_type_checks() {
    let code = "<?php function f($r: Result<int, string>) {} f(Result::Ok(1)); f(Result::Err(\"no\"));";
    assert!(check(code).is_ok());
}

#[test]
fn null_argument_to_non_option_errors() {
    let code = "<?php function f($x: int) {} f(null);";
    assert!(check(code).is_err());
}

#[test]
fn type_alias_object_shape_enforced() {
    let code = "<?php type Person = Object<{ foo: int }>; function f($p: Person) {} f({ foo: 1 }); f({ bar: 2 });";
    assert!(check(code).is_err());
}

#[test]
fn type_alias_sugar_object_shape_ok() {
    let code = "<?php type Person = { foo: int, bar?: string }; function f($p: Person) {} f({ foo: 1 });";
    assert!(check(code).is_ok());
}

#[test]
fn generic_type_alias_infers_type_param() {
    let code = "<?php type Box<T> = { value: T }; function unbox<T>($b: Box<T>): T { return $b.value; } $x = unbox({ value: 1 });";
    assert!(check(code).is_ok());
}

#[test]
fn generic_type_param_constraint_enforced() {
    let code = "<?php function f<T: int>($x: T) {} f(\"nope\");";
    assert!(check(code).is_err());
}

#[test]
fn interface_accepts_struct_with_matching_methods() {
    let code = "<?php interface Reader { public function read($n: int): string; } struct File { public function read($n: int): string { return \"\"; } } function useReader($r: Reader) {} useReader(File { });";
    assert!(check(code).is_ok());
}

#[test]
fn interface_rejects_struct_missing_method() {
    let code = "<?php interface Reader { public function read($n: int): string; } struct Bad { } function useReader($r: Reader) {} useReader(Bad { });";
    assert!(check(code).is_err());
}

#[test]
fn interface_constraint_enforced_for_type_param() {
    let code = "<?php interface Reader { public function read($n: int): string; } struct File { public function read($n: int): string { return \"\"; } } struct Bad { } function useReader<T: Reader>($r: T) {} useReader(File { }); useReader(Bad { });";
    assert!(check(code).is_err());
}

#[test]
fn interface_shape_accepts_object_literal() {
    let code = "interface NameProps { $name: string; } function fullName($props: NameProps): string { return $props.name; } fullName({ name: \"Bob\" });";
    assert!(check(code).is_ok());
}

#[test]
fn interface_shape_accepts_destructured_param_binding() {
    let code = "interface NameProps { $name: string; } function FullName({ $name }: NameProps): string { return $name; } FullName({ name: 'Bob' });";
    if let Err(err) = check(code) {
        panic!("expected destructured interface param to type-check, got:\n{}", err);
    }
}

#[test]
fn interface_shape_rejects_missing_required_field() {
    let code = "interface NameProps { $name: string; } function fullName($props: NameProps): string { return $props.name; } fullName({});";
    assert!(check(code).is_err());
}

#[test]
fn struct_embed_promotes_fields() {
    let code = "<?php struct A { $x: int; } struct B { use A; } $b = B { $A: A { $x: 1 } }; $b.x;";
    assert!(check(code).is_ok());
}

#[test]
fn struct_embed_dot_access_infers_type() {
    let code = "<?php struct A { $x: int; } struct B { use A; } function takes($x: int) {} $b = B { $A: A { $x: 1 } }; takes($b.x);";
    assert!(check(code).is_ok());
}

#[test]
fn struct_embed_ambiguous_field_errors() {
    let code = "<?php struct A { $x: int; } struct B { $x: int; } struct C { use A, B; } $c = C { $A: A { $x: 1 }, $B: B { $x: 2 } }; $c.x;";
    assert!(check(code).is_err());
}

#[test]
fn enum_payload_call_type_checks() {
    let code = "<?php enum Msg { case Text($body: string); } $m = Msg::Text(\"hi\");";
    assert!(check(code).is_ok());
}

#[test]
fn enum_payload_call_mismatch_errors() {
    let code = "<?php enum Msg { case Text($body: string); } $m = Msg::Text(123);";
    assert!(check(code).is_err());
}

#[test]
fn enum_match_exhaustive_ok() {
    let code = "<?php enum Color { case Red; case Blue; } function f($c: Color): int { return match ($c) { Color::Red => 1, Color::Blue => 2 }; }";
    assert!(check(code).is_ok());
}

#[test]
fn enum_match_missing_case_errors() {
    let code = "<?php enum Color { case Red; case Blue; } function f($c: Color): int { return match ($c) { Color::Red => 1 }; }";
    assert!(check(code).is_err());
}

#[test]
fn enum_match_arm_narrows_payload_fields() {
    let code = "<?php enum Msg { case Text($body: string); case Ping; } function f($m: Msg): string { return match ($m) { Msg::Text => $m.body, Msg::Ping => \"ok\" }; }";
    assert!(check(code).is_ok());
}

#[test]
fn enum_match_arm_rejects_invalid_payload_field() {
    let code = "<?php enum Msg { case Text($body: string); case Ping; } function f($m: Msg): string { return match ($m) { Msg::Text => \"ok\", Msg::Ping => $m.body }; }";
    assert!(check(code).is_err());
}

#[test]
fn enum_payload_assignment_allows_dot_access() {
    let code = "<?php enum Msg { case Text($body: string); } $m = Msg::Text(\"hi\"); $m.body;";
    assert!(check(code).is_ok());
}

#[test]
fn null_comparison_is_rejected() {
    let code = "<?php $x = 1; if ($x === null) { $x = 2; }";
    assert!(check(code).is_err());
}

#[test]
fn enum_match_narrows_across_multiple_enums() {
    let code = "<?php enum A { case One($body: string); } enum B { case Two($body: string); } function f($x: A|B): string { return match ($x) { A::One, B::Two => $x.body }; }";
    let result = check(code);
    assert!(result.is_ok(), "{}", result.unwrap_err());
}

#[test]
fn match_expression_infers_union_for_arguments() {
    let code = "<?php function takesInt($x: int) {} $flag = true; takesInt(match ($flag) { true => 1, false => \"no\" });";
    assert!(check(code).is_err());
}

#[test]
fn generic_array_literal_infers_type_param() {
    let code = "<?php function takes<T>($xs: array<T>) {} takes([1, 2, 3]);";
    assert!(check(code).is_ok());
}

#[test]
fn generic_array_literal_inference_enforces_constraints() {
    let code = "<?php function takes<T: int>($xs: array<T>) {} takes([1, \"no\"]);";
    assert!(check(code).is_err());
}

#[test]
fn generic_option_infers_from_some() {
    let code = "<?php function takes<T>($x: Option<T>) {} takes(Option::Some(1));";
    assert!(check(code).is_ok());
}

#[test]
fn generic_option_none_requires_type() {
    let code = "<?php function takes<T>($x: Option<T>) {} takes(Option::None);";
    assert!(check(code).is_err());
}

#[test]
fn generic_result_infers_from_ok() {
    let code = "<?php function takes<T>($x: Result<T, string>) {} takes(Result::Ok(1));";
    assert!(check(code).is_ok());
}

#[test]
fn generic_result_infers_from_err() {
    let code = "<?php function takes<E>($x: Result<int, E>) {} takes(Result::Err(\"no\"));";
    assert!(check(code).is_ok());
}

#[test]
fn struct_method_call_type_checks() {
    let code = "<?php struct Reader { public function read($n: int): string { return \"\"; } } $r = Reader { }; $r->read(1);";
    assert!(check(code).is_ok());
}

#[test]
fn struct_method_call_mismatch_errors() {
    let code = "<?php struct Reader { public function read($n: int): string { return \"\"; } } $r = Reader { }; $r->read(\"no\");";
    assert!(check(code).is_err());
}

#[test]
fn interface_method_call_mismatch_errors() {
    let code = "<?php interface Reader { public function read($n: int): string; } function useReader($r: Reader) { $r->read(\"no\"); }";
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

#[test]
fn destructured_assignment_bindings_follow_source_shape() {
    let code = "$obj = { count: 3 }; ({ count: $count } = $obj); $count + 1;";
    assert!(check(code).is_ok());
}

#[test]
fn foreach_binds_key_and_value_variables() {
    let code = "function sum($items: array): int { $total = 0; foreach ($items as $idx => $item) { $total = $total + $idx + $item; } return $total; }";
    assert!(check(code).is_ok());
}

#[test]
fn arrow_function_params_are_in_scope() {
    let code = "$f = fn($x: int): int => $x + 1;";
    assert!(check(code).is_ok());
}

#[test]
fn closure_params_are_in_scope() {
    let code = "$f = function($x: int): int { return $x + 1; };";
    assert!(check(code).is_ok());
}
