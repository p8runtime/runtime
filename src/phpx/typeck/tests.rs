use bumpalo::Bump;
use std::path::Path;

use crate::parser::lexer::Lexer;
use crate::parser::parser::{Parser, ParserMode};
use crate::phpx::typeck::{
    TypeckProgramSummary, check_program, check_program_with_imports, check_program_with_path,
    summarize_program_with_path,
};
use std::collections::HashMap;
use std::sync::Mutex;

static HOST_GRANT_ENV: Mutex<()> = Mutex::new(());

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
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Ds);
    let program = parser.parse_program();
    if !program.errors.is_empty() {
        let mut out = String::new();
        for err in program.errors {
            out.push_str(&err.message);
            out.push('\n');
        }
        return Err(out);
    }
    check_program(&program, code.as_bytes())
        .map(|_warnings| ())
        .map_err(|errs| {
            let mut out = String::new();
            for err in errs {
                out.push_str(&err.message);
                out.push('\n');
            }
            out
        })
}

// DekaScript-mode variant for .ds-only features. `check` above hardcodes
// ParserMode::Ds, so it cannot reach DekaScript-specific syntax.
fn check_ds(code: &str) -> Result<(), String> {
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Ds);
    let program = parser.parse_program();
    if !program.errors.is_empty() {
        let mut out = String::new();
        for err in program.errors {
            out.push_str(&err.message);
            out.push('\n');
        }
        return Err(out);
    }
    check_program(&program, code.as_bytes())
        .map(|_warnings| ())
        .map_err(|errs| {
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
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Ds);
    let program = parser.parse_program();
    if !program.errors.is_empty() {
        let mut out = String::new();
        for err in program.errors {
            out.push_str(&err.message);
            out.push('\n');
        }
        return Err(out);
    }
    check_program_with_path(&program, code.as_bytes(), Some(Path::new(path)))
        .map(|_warnings| ())
        .map_err(|errs| {
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
    let code = "<?php struct Point { $x: number = -1; $y: number = +1.5; }";
    assert!(check(code).is_ok());
}

#[test]
fn struct_default_allows_struct_and_object_literals() {
    let code = "<?php
        struct Point { $x: number = 0; $y: number = 0; }
        struct Box { $pos: Point = Point { $x: 1, $y: 2 }; $meta: Object = { foo: 'bar' }; }
    ";
    assert!(check(code).is_ok());
}

#[test]
fn ds_bare_struct_field_names_typecheck() {
    // dekaruntime/deka#93: DekaScript structs use bare identifiers.
    let code = "struct Point { x: number; y: number } fn f(): number { return Point { x: 3, y: 4 }.x; }";
    let res = check_ds(code);
    assert!(res.is_ok(), "expected ok, got: {:?}", res);
}

#[test]
fn ds_bare_struct_field_missing_field_errors() {
    let code = "struct Point { x: int; y: int } fn f(): Point { return Point { x: 3 }; }";
    assert!(check_ds(code).is_err());
}

#[test]
fn ds_bare_struct_field_wrong_type_errors() {
    let code = "struct Point { x: int; y: int } fn f(): Point { return Point { x: \"nope\", y: 4 }; }";
    assert!(check_ds(code).is_err());
}

#[test]
fn struct_field_annotations_basic_ok() {
    let code = "struct User { $id: number @id @autoIncrement; }";
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
    let code = "struct Post { $id: number @id; } struct User { $posts: array<Post> @relation(\"hasMany\", \"Post\", \"authorId\"); }";
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
    assert!(
        res.unwrap_err()
            .contains("__deka_wasm_call is internal-only")
    );
}

#[test]
fn deka_wasm_call_async_forbidden_outside_internals() {
    let code = "__deka_wasm_call_async('__deka_db', 'open', {})";
    let res = check_with_path(code, "/tmp/app/index.phpx");
    assert!(res.is_err());
    assert!(
        res.unwrap_err()
            .contains("__deka_wasm_call_async is internal-only")
    );
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
fn jsx_component_return_type_annotation_ok() {
    // dekaruntime/deka#122: Component is the canonical name for JSX return
    // types. VNode and JSX are not accepted as aliases pre-launch.
    let code = "<?php function Hero({ $name }: Object): Component { return <div>{ $name }</div>; }";
    let res = check(code);
    assert!(res.is_ok(), "expected ok, got: {:?}", res);
}

#[test]
fn jsx_component_inside_generic_return_type_ok() {
    let code = "<?php async function Hero({ $name }: Object): Promise<Component> { return <div>{ $name }</div>; }";
    let res = check(code);
    assert!(res.is_ok(), "expected ok, got: {:?}", res);
}

#[test]
fn jsx_vnode_alias_is_rejected() {
    let code = "<?php function Hero({ $name }: Object): VNode { return <div>{ $name }</div>; }";
    assert!(check(code).is_err());
}

#[test]
fn jsx_jsx_alias_is_rejected() {
    let code = "<?php function Hero({ $name }: Object): JSX { return <div>{ $name }</div>; }";
    assert!(check(code).is_err());
}

#[test]
fn jsx_component_untyped_props_allowed_in_default_mode() {
    // Strict JSX type checking is gated by PHPX_STRICT_JSX_TYPES env var;
    // in default mode, untyped props parameters are allowed.
    let code = "<?php function FullName($name) { return $name; } $v = <FullName name=\"Bob\" />;";
    assert!(check(code).is_ok());
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
fn jsx_component_struct_props_allowed_in_default_mode() {
    // Strict JSX type checking is gated by PHPX_STRICT_JSX_TYPES env var;
    // in default mode, struct props are not rejected at the JSX call site.
    let code = "struct FullNameProps { $name: string; } function FullName($props: FullNameProps): string { return $props.name; } $v = <FullName name='Bob' />;";
    assert!(check(code).is_ok());
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
fn ds_jsx_component_destructured_param_props_are_recognized() {
    // dekaruntime/deka#93: DekaScript JSX components use bare destructured
    // params ({ name }: GreetingProps) and bare interface fields.
    let code = "interface GreetingProps { name: string } fn Greeting({ name }: GreetingProps): Component { return <h1>Hello {name}</h1> } <Greeting name=\"DekaScript\" />";
    let res = check_ds(code);
    assert!(res.is_ok(), "expected ok, got: {:?}", res);
}

#[test]
fn ds_jsx_component_with_separator_destructured_param_props_are_recognized() {
    // Component files separate script and template with '---'.
    let code = "interface GreetingProps { name: string } fn Greeting({ name }: GreetingProps): Component { return <h1>Hello {name}</h1> }\n---\n<Greeting name=\"DekaScript\" />";
    let res = check_ds(code);
    assert!(res.is_ok(), "expected ok, got: {:?}", res);
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
fn null_literal_allowed_in_default_mode() {
    // Strict null checking is gated by DEKA_STRICT_NULL env var;
    // in default mode, null literals are allowed.
    let code = "<?php $x = null;";
    assert!(check(code).is_ok());
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
    let code =
        "<?php function f($r: Result<int, string>) {} f(Result::Ok(1)); f(Result::Err(\"no\"));";
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
    let code =
        "<?php type Person = { foo: int, bar?: string }; function f($p: Person) {} f({ foo: 1 });";
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
        panic!(
            "expected destructured interface param to type-check, got:\n{}",
            err
        );
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

// --- Generic type-parameter assignability (issue #46) ---------------------
// Before the fix, `is_assignable_base` returned true whenever either side was
// a TypeParam, which switched off generic checking entirely: `T` accepted
// anything and anything accepted `T`. Nothing exercised those arms, so the
// hole was invisible. These tests are the missing coverage.

#[test]
fn generic_identity_is_assignable() {
    let code = "<?php function id<T>($v: T): T { return $v; }";
    assert!(check(code).is_ok(), "T should be assignable to T");
}

#[test]
fn generic_param_to_concrete_return_errors() {
    let code = "<?php function bad<T>($v: T): int { return $v; }";
    assert!(
        check(code).is_err(),
        "an unconstrained T must not satisfy a concrete int return type"
    );
}

#[test]
fn concrete_to_generic_param_return_errors() {
    let code = "<?php function bad<T>($v: int): T { return $v; }";
    assert!(
        check(code).is_err(),
        "int must not satisfy an unconstrained T return type"
    );
}

#[test]
fn distinct_type_params_are_not_interchangeable() {
    let code = "<?php function bad<A, B>($v: A): B { return $v; }";
    assert!(
        check(code).is_err(),
        "A and B are distinct type parameters and must not be assignable to each other"
    );
}

// --- RFD 19 Phase 5: trait/impl removed; receiver methods remain ----------

#[test]
fn ds_trait_declaration_rejected() {
    let code = "trait Greeter {\n  greet(): string\n}";
    assert!(check_ds(code).is_err(), "expected trait to be rejected");
}

#[test]
fn ds_impl_rejected() {
    let code = "struct Point { $x: int; }\n\nimpl Point { norm(): int { return 1; } }";
    assert!(check_ds(code).is_err(), "expected impl to be rejected");
}

#[test]
fn ds_legacy_php_trait_still_rejected_outside_ds() {
    let code = "<?php trait Foo { public fn bar() {} }";
    assert!(check(code).is_err(), "PHP horizontal-reuse traits must stay rejected in PHPX");
}

#[test]
fn bytes_type_in_param_and_return_is_ok() {
    let code = "function encode($input: bytes): bytes { return $input; }";
    assert!(check(code).is_ok());
}

#[test]
fn bytes_type_rejects_string_assignment() {
    let code = "function f(): bytes { return 'hello'; }";
    assert!(check(code).is_err());
}

#[test]
fn bytes_type_accepts_bytes_variable() {
    let code = "function f($b: bytes): bytes { return $b; }";
    assert!(check(code).is_ok());
}

#[test]
fn bytes_type_in_struct_field_is_ok() {
    let code = "struct Packet { $payload: bytes; }";
    assert!(check(code).is_ok());
}

// --- Diagnostic severity mechanism (deka#59) -------------------------------
//
// check_program's Result discriminant is the severity signal: Ok(diagnostics)
// means the program checked out (diagnostics, if any, are all Warning-level);
// Err(diagnostics) means at least one Error-level diagnostic is present.


#[test]
fn real_type_error_still_fails_exactly_as_before() {
    // Same shape of program as the pre-existing
    // `generic_param_to_concrete_return_errors` regression test above --
    // confirms the severity mechanism did not soften real errors into
    // warnings, and that check_program still returns Err for them.
    let code = normalize_phpx_snippet("<?php function bad<T>($v: T): int { return $v; }");
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(code.as_bytes()), &arena, ParserMode::Ds);
    let program = parser.parse_program();
    assert!(program.errors.is_empty(), "program should parse cleanly");

    let result = check_program(&program, code.as_bytes());
    let diagnostics = match result {
        Err(diagnostics) => diagnostics,
        Ok(_) => panic!("an unconstrained T must not satisfy a concrete int return type"),
    };

    assert!(
        diagnostics.iter().any(|d| d.severity == crate::parser::ast::Severity::Error),
        "Err(..) must contain at least one Error-severity diagnostic"
    );
}

#[test]
fn ds_enum_js_style_body_typechecks() {
    let code = r#"
        enum Status { Loading, Ready, Failed }
        fn getStatus(): Status { return Status.Ready; }
    "#;
    assert!(check_ds(code).is_ok(), "{}", check_ds(code).unwrap_err());
}

#[test]
fn ds_enum_generic_payload_typechecks() {
    let code = r#"
        enum Option<T> { Some(T), None }
        fn getOption(): Option<number> { return Option::Some(1); }
    "#;
    assert!(check_ds(code).is_ok(), "{}", check_ds(code).unwrap_err());
}

#[test]
fn ds_enum_generic_payload_mismatch_errors() {
    let code = r#"
        enum Option<T> { Some(T), None }
        fn getOption(): Option<int> { return Option::Some("no"); }
    "#;
    assert!(check_ds(code).is_err());
}

#[test]
fn ds_enum_non_generic_payload_typechecks() {
    let code = r#"
        enum Msg { Text(string), Ping }
        fn getMsg(): Msg { return Msg::Text("hi"); }
    "#;
    assert!(check_ds(code).is_ok(), "{}", check_ds(code).unwrap_err());
}

#[test]
fn ds_enum_non_generic_payload_mismatch_errors() {
    let code = r#"
        enum Msg { Text(string), Ping }
        fn getMsg(): Msg { return Msg::Text(123); }
    "#;
    assert!(check_ds(code).is_err());
}

#[test]
fn ds_enum_match_destructures_non_generic_payload() {
    // DekaScript enum payloads are anonymous; the compiler uses the type text
    // as the synthetic field name so the typechecker can narrow it.
    let code = r#"
        enum Msg { Text(string), Ping }
        fn body(m: Msg): string {
            return match (m) {
                Msg::Text => m.string,
                Msg::Ping => "ok",
            }
        }
    "#;
    assert!(check_ds(code).is_ok(), "{}", check_ds(code).unwrap_err());
}

#[test]
fn ds_enum_match_non_generic_payload_field_outside_arm_errors() {
    let code = r#"
        enum Msg { Text(string), Ping }
        fn body(m: Msg): string {
            return match (m) {
                Msg::Text => "ok",
                Msg::Ping => m.string,
            }
        }
    "#;
    assert!(check_ds(code).is_err());
}

#[test]
fn ds_enum_match_exhaustive_on_js_style_enum() {
    let code = r#"
        enum Status { Loading, Ready, Failed }
        fn f(s: Status): number {
            match (s) {
                Status::Loading => 0,
                Status::Ready => 1,
                Status::Failed => 2,
            }
        }
    "#;
    assert!(check_ds(code).is_ok(), "{}", check_ds(code).unwrap_err());
}

#[test]
fn ds_enum_match_exhaustive_using_dot_access() {
    // DekaScript also accepts `Status.Ready` as syntactic sugar for
    // `Status::Ready` when the left-hand side names an enum.
    let code = r#"
        enum Status { Loading, Ready, Failed }
        fn f(s: Status): number {
            match (s) {
                Status.Loading => 0,
                Status.Ready => 1,
                Status.Failed => 2,
            }
        }
    "#;
    assert!(check_ds(code).is_ok(), "{}", check_ds(code).unwrap_err());
}

#[test]
fn ds_enum_match_missing_case_errors() {
    let code = r#"
        enum Status { Loading, Ready, Failed }
        fn f(s: Status): int {
            match (s) {
                Status::Loading => 0,
                Status::Ready => 1,
            }
        }
    "#;
    assert!(check_ds(code).is_err());
}


#[test]
fn ds_function_return_type_inferred_from_literal() {
    // dekaruntime/deka#120: unannotated fn return types are inferred.
    let code = "fn answer() { return 42; } const x = answer();";
    assert!(check_ds(code).is_ok());
}

#[test]
fn ds_function_return_type_inferred_from_parameters() {
    let code = "fn add(left: number, right: number) { return left + right; } const x = add(1, 2);";
    assert!(check_ds(code).is_ok());
}

#[test]
fn ds_function_return_type_inferred_async_wraps_promise() {
    let code = "async fn fetch() { return 1; } const x = await fetch();";
    assert!(check_ds(code).is_ok());
}

#[test]
fn ds_function_inferred_return_mismatch_is_rejected() {
    // Once a return type is inferred, inconsistent return branches should error.
    let code = "fn maybe() { if (true) { return 1; } else { return \"two\"; } }";
    assert!(check_ds(code).is_err());
}

#[test]
fn ds_function_inferred_return_used_in_typed_call() {
    // Inferred return types should flow to callers.
    let code = "fn one() { return 1; } fn add(left: number, right: number) { return left + right; } const x = add(one(), 2);";
    assert!(check_ds(code).is_ok());
}


#[test]
fn ds_function_return_type_inferred_from_arithmetic() {
    let code = "fn add(left: number, right: number) { return left + right; } fn useNumber(n: number) {} useNumber(add(1, 2));";
    assert!(check_ds(code).is_ok());
}

#[test]
fn ds_function_return_type_inferred_from_float_arithmetic() {
    let code = "fn add(left: number, right: number) { return left + right; } fn useFloat(n: number) {} useFloat(add(1.0, 2.0));";
    assert!(check_ds(code).is_ok());
}

#[test]
fn ds_function_return_type_inferred_from_concatenation() {
    let code = "fn greet(name: string) { return \"Hello, \" + name; } fn useString(s: string) {} useString(greet(\"Deka\"));";
    assert!(check_ds(code).is_ok());
}

#[test]
fn ds_function_return_type_inferred_from_comparison() {
    let code = "fn check(a: number, b: number) { return a > b; } fn useBool(b: boolean) {} useBool(check(1, 2));";
    assert!(check_ds(code).is_ok());
}

#[test]
fn ds_function_return_type_inferred_from_logical() {
    let code = "fn both(a: boolean, b: boolean) { return a && b; } fn useBool(b: boolean) {} useBool(both(true, false));";
    assert!(check_ds(code).is_ok());
}

#[test]
fn ds_function_return_type_inferred_inside_switch() {
    let code = r#"fn pick(n: number) {
  switch (n) {
    case 1: return "one";
    case 2: return "two";
    default: return "many";
  }
}
fn useString(s: string) {}
useString(pick(1));"#;
    assert!(check_ds(code).is_ok());
}

#[test]
fn ds_async_function_return_not_double_wrapped() {
    // Returning a Promise<T> from an async fn should not become Promise<Promise<T>>.
    let code = r#"async fn fetch() { return 1; }
async fn wrapper() { return await fetch(); }
fn useNumber(n: number) {}
useNumber(await wrapper());"#;
    assert!(check_ds(code).is_ok());
}

#[test]
fn phpx_function_return_type_inferred_from_literal() {
    let code = "<?php function answer() { return 42; } $x = answer();";
    assert!(check(code).is_ok());
}

#[test]
fn phpx_function_return_type_inferred_mismatch_rejected() {
    let code = "<?php function maybe($x) { if ($x) { return 1; } else { return \"two\"; } }";
    assert!(check(code).is_err());
}

// --- RFD 19 Phase 3: receiver methods, optional fields, spread -------------

#[test]
fn ds_receiver_method_type_checks() {
    let code = r#"
        struct Person { name: string }
        fn (p Person) greet(): string { return "hi" }
        fn f(): string {
          const person = Person { name: "Ada" };
          return person.greet();
        }
    "#;
    let res = check_ds(code);
    assert!(res.is_ok(), "expected receiver method to type-check: {:?}", res);
}

#[test]
fn ds_receiver_method_body_type_error_is_caught() {
    // Receiver method bodies must be type-checked even when the method is
    // never called; before the fix this fell through the Stmt::ReceiverMethod
    // arm and produced no diagnostics.
    let code = r#"
        struct Person { name: string }
        fn (p Person) greet(): string { return 123 }
    "#;
    let res = check_ds(code);
    assert!(res.is_err(), "expected receiver method body type error: {:?}", res);
    assert!(
        res.unwrap_err().contains("Return type mismatch"),
        "expected return type mismatch error"
    );
}

#[test]
fn ds_receiver_method_receiver_is_bound_in_body() {
    let code = r#"
        struct Person { name: string }
        fn (p Person) greet(): string { return p.name }
    "#;
    let res = check_ds(code);
    assert!(
        res.is_ok(),
        "expected receiver variable to be in scope: {:?}",
        res
    );
}

#[test]
fn ds_receiver_method_param_type_error_is_caught() {
    let code = r#"
        struct Person { name: string }
        fn (p Person) greet(greeting: string): string { return greeting }
        fn f(): string {
          const person = Person { name: "Ada" };
          return person.greet(123);
        }
    "#;
    let res = check_ds(code);
    assert!(res.is_err(), "expected receiver method parameter type mismatch: {:?}", res);
}

#[test]
fn ds_mutable_receiver_on_const_errors() {
    let code = r#"
        struct Person { name: string }
        fn (p mut Person) setName(name: string) {}
        fn f() {
          const person = Person { name: "Ada" };
          person.setName("Bob");
        }
    "#;
    let res = check_ds(code);
    assert!(res.is_err(), "expected mutable receiver call on const to fail");
    assert!(
        res.unwrap_err().contains("mutable method"),
        "expected mutable receiver error"
    );
}

#[test]
fn ds_mutable_receiver_on_let_ok() {
    let code = r#"
        struct Person { name: string }
        fn (p mut Person) setName(name: string) {}
        fn f() {
          let person = Person { name: "Ada" };
          person.setName("Bob");
        }
    "#;
    let res = check_ds(code);
    assert!(res.is_ok(), "expected mutable receiver call on let to pass: {:?}", res);
}

#[test]
fn ds_mutable_receiver_on_function_return_errors() {
    // expr_is_mutable used to return true for any non-variable expression,
    // allowing mutable receiver calls on temporary values.
    let code = r#"
        struct Person { name: string }
        fn makePerson(): Person { return Person { name: "Ada" } }
        fn (p mut Person) setName(name: string) {}
        fn f() {
          makePerson().setName("Bob");
        }
    "#;
    let res = check_ds(code);
    assert!(
        res.is_err(),
        "expected mutable receiver call on function return to fail"
    );
    assert!(
        res.unwrap_err().contains("mutable method"),
        "expected mutable receiver error"
    );
}

#[test]
fn ds_mutable_receiver_on_immutable_param_errors() {
    // Function parameters were unconditionally added to the mutable
    // environment, making every parameter mutable.
    let code = r#"
        struct Person { name: string }
        fn (p mut Person) setName(name: string) {}
        fn f(person: Person) {
          person.setName("Bob");
        }
    "#;
    let res = check_ds(code);
    assert!(
        res.is_err(),
        "expected mutable receiver call on immutable param to fail"
    );
    assert!(
        res.unwrap_err().contains("mutable method"),
        "expected mutable receiver error"
    );
}

#[test]
fn ds_interface_bare_field_names_accept_object_literal() {
    let code = r#"
        interface NameProps { name: string }
        fn fullName(props: NameProps): string { return props.name; }
        fullName({ name: "Bob" });
    "#;
    let res = check_ds(code);
    assert!(res.is_ok(), "expected bare interface fields to work: {:?}", res);
}

#[test]
fn ds_interface_optional_field_allows_missing() {
    let code = r#"
        interface Media { title: string; subtitle?: string }
        fn getSubtitle(m: Media): Option<string> { return m.subtitle; }
        getSubtitle({ title: "A" });
    "#;
    let res = check_ds(code);
    assert!(res.is_ok(), "expected optional interface field to allow missing: {:?}", res);
}

#[test]
fn ds_object_literal_spread_from_interface_ok() {
    let code = r#"
        interface Base { a: number; b: string }
        fn f(base: Base): number {
          const copy = { ...base, b: "y" };
          return copy.a;
        }
    "#;
    let res = check_ds(code);
    assert!(res.is_ok(), "expected object-literal spread to type-check: {:?}", res);
}

#[test]
fn ds_object_literal_spread_type_mismatch_errors() {
    let code = r#"
        interface Base { a: int; b: string }
        fn f(base: Base): Base {
          return { ...base, b: 123 };
        }
    "#;
    let res = check_ds(code);
    assert!(res.is_err(), "expected spread field type mismatch to fail");
}

#[test]
fn ds_jsx_spread_props_satisfies_required() {
    let code = r#"
        interface GreetingProps { name: string }
        fn Greeting(props: GreetingProps): Component { return <h1>Hello {props.name}</h1> }
        fn render(): Component {
          const props = { name: "Deka" };
          return <Greeting {...props} />;
        }
    "#;
    let res = check_ds(code);
    assert!(res.is_ok(), "expected JSX spread props to satisfy required prop: {:?}", res);
}

#[test]
fn ds_jsx_spread_missing_required_prop_errors() {
    let code = r#"
        interface GreetingProps { name: string }
        fn Greeting(props: GreetingProps): Component { return <h1>Hello {props.name}</h1> }
        fn render(): Component {
          const props = {};
          return <Greeting {...props} />;
        }
    "#;
    let res = check_ds(code);
    assert!(res.is_err(), "expected JSX spread missing required prop to fail");
    assert!(
        res.unwrap_err().contains("Missing required prop 'name'"),
        "expected missing required prop error"
    );
}

#[test]
fn ds_embed_promotes_methods() {
    let code = r#"
        struct Person { name: string }
        fn (p Person) greet(): string { return "hi, " + p.name }
        struct Employee { Person; employeeId: string }
        fn f(): string {
          const e = Employee { Person: Person { name: "Ada" }, employeeId: "E1" };
          return e.greet();
        }
    "#;
    let res = check_ds(code);
    assert!(res.is_ok(), "expected embedded methods to be promoted: {:?}", res);
}

#[test]
fn ds_embed_promoted_method_satisfies_interface() {
    let code = r#"
        interface Greeter { fn greet(): string }
        struct Person { name: string }
        fn (p Person) greet(): string { return "hi" }
        struct Employee { Person; employeeId: string }
        fn useGreeter(g: Greeter): string { return g.greet(); }
        fn f(): string {
          const e = Employee { Person: Person { name: "Ada" }, employeeId: "E1" };
          return useGreeter(e);
        }
    "#;
    let res = check_ds(code);
    assert!(
        res.is_ok(),
        "expected promoted methods to satisfy interface: {:?}",
        res
    );
}


#[test]
fn ds_const_struct_field_mutation_errors() {
    let code = r#"
        struct Point { x: int }
        fn f() {
          const p = Point { x: 1 };
          p.x = 2;
        }
    "#;
    let res = check_ds(code);
    assert!(res.is_err(), "expected const struct field mutation to fail");
    assert!(
        res.unwrap_err().contains("cannot assign to field of immutable value"),
        "expected immutable value error"
    );
}

#[test]
fn ds_let_struct_field_mutation_ok() {
    let code = r#"
        struct Point { x: number }
        fn f() {
          let p = Point { x: 1 };
          p.x = 2;
        }
    "#;
    let res = check_ds(code);
    assert!(res.is_ok(), "expected let struct field mutation to pass: {:?}", res);
}

#[test]
fn ds_const_struct_nested_field_mutation_errors() {
    let code = r#"
        struct Point { x: int }
        struct Nested { p: Point }
        fn f() {
          const nested = Nested { p: Point { x: 1 } };
          nested.p.x = 2;
        }
    "#;
    let res = check_ds(code);
    assert!(res.is_err(), "expected const struct nested field mutation to fail");
    assert!(
        res.unwrap_err().contains("cannot assign to field of immutable value"),
        "expected immutable value error"
    );
}

#[test]
fn ds_interface_mut_field_assignment_ok() {
    let code = r#"
        interface Person { mut name: string }
        fn rename(p: Person) {
          p.name = "Bob";
        }
    "#;
    let res = check_ds(code);
    assert!(res.is_ok(), "expected mut interface field assignment to pass: {:?}", res);
}

#[test]
fn ds_interface_readonly_field_assignment_errors() {
    let code = r#"
        interface Person { name: string }
        fn rename(p: Person) {
          p.name = "Bob";
        }
    "#;
    let res = check_ds(code);
    assert!(res.is_err(), "expected readonly interface field assignment to fail");
    assert!(
        res.unwrap_err().contains("field 'name' is read-only"),
        "expected read-only field error"
    );
}

#[test]
fn ds_struct_field_assignment_uses_base_mutability() {
    let code = r#"
        struct Point { x: int }
        fn f() {
          const p = Point { x: 1 };
          p.x = 2;
        }
    "#;
    let res = check_ds(code);
    assert!(res.is_err(), "expected struct field assignment on const to fail");
    assert!(
        res.unwrap_err().contains("cannot assign to field of immutable value"),
        "expected immutable value error"
    );
}


#[test]
fn ds_cross_module_import_type_checks_against_remote_signature() {
    let arena = Bump::new();
    let math_code = r#"
        export fn add(a: number, b: number): number {
            return a + b;
        }
    "#;
    let mut parser = Parser::new_with_mode(Lexer::new(math_code.as_bytes()), &arena, ParserMode::Ds);
    let math_program = parser.parse_program();
    assert!(math_program.errors.is_empty(), "{:?}", math_program.errors);
    let math_summary = summarize_program_with_path(&math_program, math_code.as_bytes(), None)
        .expect("math module should summarize");

    let app_code = r#"
        import { add } from "./math.ds";
        const result = add(1, 2);
        console.log(result);
    "#;
    let mut parser = Parser::new_with_mode(Lexer::new(app_code.as_bytes()), &arena, ParserMode::Ds);
    let app_program = parser.parse_program();
    assert!(app_program.errors.is_empty(), "{:?}", app_program.errors);

    let mut imports = HashMap::new();
    imports.insert("./math.ds".to_string(), math_summary);
    let result = check_program_with_imports(
        &app_program,
        app_code.as_bytes(),
        None,
        &imports,
    );
    assert!(result.is_ok(), "expected cross-module import to type-check, got: {:?}", result);
}

#[test]
fn ds_cross_module_import_rejects_missing_export() {
    let arena = Bump::new();
    let math_summary = TypeckProgramSummary {
        structs: HashMap::new(),
        enums: HashMap::new(),
        functions: HashMap::new(),
        type_aliases: HashMap::new(),
    };

    let app_code = r#"
        import { missing } from "./math.ds";
        missing();
    "#;
    let mut parser = Parser::new_with_mode(Lexer::new(app_code.as_bytes()), &arena, ParserMode::Ds);
    let app_program = parser.parse_program();
    assert!(app_program.errors.is_empty(), "{:?}", app_program.errors);

    let mut imports = HashMap::new();
    imports.insert("./math.ds".to_string(), math_summary);
    let result = check_program_with_imports(
        &app_program,
        app_code.as_bytes(),
        None,
        &imports,
    );
    let errors = result.expect_err("expected missing export error");
    assert!(
        errors.iter().any(|e| e.message.contains("is not exported by")),
        "expected 'not exported' error, got: {:?}",
        errors
    );
}

#[test]
fn bridge_rejected_without_host_grant() {
    let code = "fn go() { bridge crypto.random_bytes(32); }";
    let err = check_ds(code).expect_err("app code must not call bridge");
    assert!(
        err.contains("host-granted stdlib package"),
        "unexpected error: {err}"
    );
}

#[test]
fn bridge_allowed_in_workspace_stdlib_package() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("deka.json"),
        r#"{ "name": "crypto", "host": { "kinds": ["crypto"] } }"#,
    )
    .unwrap();
    let src = dir.path().join("index.ds");
    let code = "fn go() { bridge crypto.random_bytes(32); }\n";
    std::fs::write(&src, code).unwrap();
    assert!(
        check_with_path(code, src.to_str().unwrap()).is_ok(),
        "workspace crypto package should be allowed to bridge crypto.*"
    );
}

#[test]
fn bridge_rejects_kind_not_on_workspace_grant() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("deka.json"),
        r#"{ "name": "crypto", "host": { "kinds": ["crypto"] } }"#,
    )
    .unwrap();
    let src = dir.path().join("index.ds");
    let code = "fn go() { bridge fs.read_file(\"a\"); }\n";
    std::fs::write(&src, code).unwrap();
    let err = check_with_path(code, src.to_str().unwrap()).expect_err("fs is not granted");
    assert!(
        err.contains("does not include kind fs"),
        "unexpected error: {err}"
    );
}

#[test]
fn app_deka_json_cannot_declare_host_kinds() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("deka.json"),
        r#"{ "name": "my-app", "host": { "kinds": ["crypto"] } }"#,
    )
    .unwrap();
    let src = dir.path().join("main.ds");
    let code = "fn go() { bridge crypto.random_bytes(32); }\n";
    std::fs::write(&src, code).unwrap();
    let err = check_with_path(code, src.to_str().unwrap()).expect_err("app grant is illegal");
    assert!(
        err.contains("official stdlib package"),
        "unexpected error: {err}"
    );
}

#[test]
fn bridge_allowed_for_installed_package_with_digest_grant() {
    let app = tempfile::tempdir().unwrap();
    std::fs::write(app.path().join("deka.json"), r#"{ "name": "my-app" }"#).unwrap();
    let pkg = app.path().join("ds_modules").join("crypto");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(
        pkg.join("deka.json"),
        r#"{ "name": "crypto", "host": { "kinds": ["crypto"] } }"#,
    )
    .unwrap();
    let src = pkg.join("index.ds");
    let code = "fn go() { bridge crypto.random_bytes(32); }\n";
    std::fs::write(&src, code).unwrap();

    let digest = crate::phpx::typeck::check::package_fs_digest(&pkg).expect("digest");
    let grants = format!(r#"[{{"digest":"{digest}","kinds":["crypto"]}}]"#);
    let _guard = HOST_GRANT_ENV.lock().expect("grant env lock");
    // SAFETY: test-only process env for grant lookup; restored below.
    let prev = std::env::var("DEKA_HOST_GRANTS").ok();
    unsafe {
        std::env::set_var("DEKA_HOST_GRANTS", &grants);
    }
    let result = check_with_path(code, src.to_str().unwrap());
    unsafe {
        match prev {
            Some(value) => std::env::set_var("DEKA_HOST_GRANTS", value),
            None => std::env::remove_var("DEKA_HOST_GRANTS"),
        }
    }
    assert!(
        result.is_ok(),
        "installed crypto with matching digest grant should typecheck: {:?}",
        result.err()
    );
}

#[test]
fn bridge_rejects_installed_package_without_digest_grant() {
    let app = tempfile::tempdir().unwrap();
    std::fs::write(app.path().join("deka.json"), r#"{ "name": "my-app" }"#).unwrap();
    let pkg = app.path().join("ds_modules").join("crypto");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(
        pkg.join("deka.json"),
        r#"{ "name": "crypto", "host": { "kinds": ["crypto"] } }"#,
    )
    .unwrap();
    let src = pkg.join("index.ds");
    let code = "fn go() { bridge crypto.random_bytes(32); }\n";
    std::fs::write(&src, code).unwrap();
    let _guard = HOST_GRANT_ENV.lock().expect("grant env lock");
    let prev = std::env::var("DEKA_HOST_GRANTS").ok();
    unsafe {
        std::env::remove_var("DEKA_HOST_GRANTS");
    }
    let err = check_with_path(code, src.to_str().unwrap())
        .expect_err("installed package without grant table entry");
    unsafe {
        match prev {
            Some(value) => std::env::set_var("DEKA_HOST_GRANTS", value),
            None => {}
        }
    }
    assert!(
        err.contains("digest does not match a host grant"),
        "unexpected error: {err}"
    );
}

#[test]
fn bridge_allowed_for_installed_package_with_host_grants_file() {
    let app = tempfile::tempdir().unwrap();
    std::fs::write(app.path().join("deka.json"), r#"{ "name": "my-app" }"#).unwrap();
    let pkg = app.path().join("ds_modules").join("crypto");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(
        pkg.join("deka.json"),
        r#"{ "name": "crypto", "host": { "kinds": ["crypto"] } }"#,
    )
    .unwrap();
    let src = pkg.join("index.ds");
    let code = "fn go() { bridge crypto.random_bytes(32); }\n";
    std::fs::write(&src, code).unwrap();

    let digest = crate::phpx::typeck::check::package_fs_digest(&pkg).expect("digest");
    std::fs::write(
        app.path().join("host-grants.json"),
        format!(r#"[{{"digest":"{digest}","kinds":["crypto"]}}]"#),
    )
    .unwrap();

    let _guard = HOST_GRANT_ENV.lock().expect("grant env lock");
    let prev = std::env::var("DEKA_HOST_GRANTS").ok();
    unsafe {
        std::env::remove_var("DEKA_HOST_GRANTS");
    }
    let result = check_with_path(code, src.to_str().unwrap());
    unsafe {
        match prev {
            Some(value) => std::env::set_var("DEKA_HOST_GRANTS", value),
            None => std::env::remove_var("DEKA_HOST_GRANTS"),
        }
    }
    assert!(
        result.is_ok(),
        "host-grants.json next to the app should grant crypto: {:?}",
        result.err()
    );
}

#[test]
fn bridge_crypto_digest_hmac_secure_compare_typecheck() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("deka.json"),
        r#"{ "name": "@deka/crypto", "host": { "kinds": ["crypto"] } }"#,
    )
    .unwrap();
    let src = dir.path().join("index.ds");
    let code = r#"
fn go(data: bytes, key: bytes) {
  bridge crypto.digest("sha256", data)
  bridge crypto.hmac("sha256", key, data)
  bridge crypto.secure_compare(data, key)
}
"#;
    std::fs::write(&src, code).unwrap();
    assert!(
        check_with_path(code, src.to_str().unwrap()).is_ok(),
        "workspace @deka/crypto should typecheck digest/hmac/secure_compare: {:?}",
        check_with_path(code, src.to_str().unwrap()).err()
    );
}

#[test]
fn bridge_crypto_digest_rejects_string_data() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("deka.json"),
        r#"{ "name": "crypto", "host": { "kinds": ["crypto"] } }"#,
    )
    .unwrap();
    let src = dir.path().join("index.ds");
    let code = "fn go() { bridge crypto.digest(\"sha256\", \"not-bytes\"); }\n";
    std::fs::write(&src, code).unwrap();
    let err = check_with_path(code, src.to_str().unwrap()).expect_err("string is not bytes");
    assert!(
        err.contains("expected bytes"),
        "unexpected error: {err}"
    );
}

#[test]
fn bridge_unknown_crypto_action_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("deka.json"),
        r#"{ "name": "crypto", "host": { "kinds": ["crypto"] } }"#,
    )
    .unwrap();
    let src = dir.path().join("index.ds");
    let code = "fn go() { bridge crypto.sign(\"x\"); }\n";
    std::fs::write(&src, code).unwrap();
    let err = check_with_path(code, src.to_str().unwrap()).expect_err("sign is not catalogued");
    assert!(
        err.contains("unknown bridge action"),
        "unexpected error: {err}"
    );
}

#[test]
fn host_module_runtime_import_typechecks() {
    let code = r#"
import { runtime } from "host"
fn go() {
  runtime
}
"#;
    assert!(
        check_ds(code).is_ok(),
        "from \"host\" import {{ runtime }} should typecheck: {:?}",
        check_ds(code).err()
    );
}

#[test]
fn host_module_unknown_export_errors() {
    let code = r#"
import { not_a_thing } from "host"
"#;
    let err = check_ds(code).expect_err("unknown host export");
    assert!(
        err.contains("not exported by 'host'"),
        "unexpected error: {err}"
    );
}

#[test]
fn host_module_runtime_match_typechecks() {
    let code = r#"
import { runtime } from "host"
fn go() {
  match (runtime) {
    HostRuntime.Browser => 1,
    HostRuntime.Native => 2,
  }
}
"#;
    assert!(
        check_ds(code).is_ok(),
        "match on host runtime should typecheck: {:?}",
        check_ds(code).err()
    );
}
