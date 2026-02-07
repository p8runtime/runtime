use bumpalo::Bump;
use php_rs::compiler::emitter::Emitter;
use php_rs::core::value::{Handle, Val};
use php_rs::parser::lexer::Lexer;
use php_rs::parser::parser::{Parser, ParserMode};
use php_rs::runtime::context::EngineBuilder;
use php_rs::vm::engine::{VM, VmError};
use std::rc::Rc;

fn run_phpx(code: &str) -> Result<(VM, Handle), VmError> {
    let engine = EngineBuilder::new()
        .with_core_extensions()
        .build()
        .expect("Failed to build engine");
    let mut vm = VM::new(engine);

    let arena = Bump::new();
    let lexer = Lexer::new(code.as_bytes());
    let mut parser = Parser::new_with_mode(lexer, &arena, ParserMode::Phpx);
    let program = parser.parse_program();
    assert!(
        program.errors.is_empty(),
        "parse errors: {:?}",
        program.errors
    );

    let emitter = Emitter::new(code.as_bytes(), &mut vm.context.interner);
    let (chunk, _) = emitter.compile(program.statements);
    vm.run(Rc::new(chunk))?;

    let handle = vm
        .last_return_value
        .ok_or_else(|| VmError::RuntimeError("no return".into()))?;
    Ok((vm, handle))
}

fn val_to_string(vm: &VM, handle: Handle) -> String {
    match &vm.arena.get(handle).value {
        Val::String(s) => String::from_utf8_lossy(s).to_string(),
        Val::Int(i) => i.to_string(),
        Val::Bool(b) => {
            if *b {
                "1".into()
            } else {
                "".into()
            }
        }
        Val::Null => "".into(),
        other => format!("{:?}", other),
    }
}

#[test]
fn enum_payload_constructs_value() {
    let code = r#"<?php
        enum Msg { case Text(string $body); }
        $m = Msg::Text("hi");
        return $m->body;
    "#;
    let (vm, handle) = run_phpx(code).expect("vm run");
    assert_eq!(val_to_string(&vm, handle), "hi");
}

#[test]
fn enum_payload_has_name() {
    let code = r#"<?php
        enum Msg { case Text(string $body); }
        return Msg::Text("hi")->name;
    "#;
    let (vm, handle) = run_phpx(code).expect("vm run");
    assert_eq!(val_to_string(&vm, handle), "Text");
}

#[test]
fn enum_equality_is_case_based() {
    let code = r#"<?php
        enum Msg { case Text(string $body); }
        return Msg::Text("a") === Msg::Text("b");
    "#;
    let (vm, handle) = run_phpx(code).expect("vm run");
    assert_eq!(val_to_string(&vm, handle), "1");
}

#[test]
fn enum_payload_arity_mismatch_errors() {
    let code = r#"<?php
        enum Msg { case Text(string $body); }
        return Msg::Text();
    "#;
    let res = run_phpx(code);
    match res {
        Err(VmError::RuntimeError(msg)) => {
            assert!(msg.contains("Enum case expects"), "unexpected msg {msg}")
        }
        Err(other) => panic!("unexpected error variant {other:?}"),
        Ok(_) => panic!("vm unexpectedly succeeded"),
    }
}

#[test]
fn enum_payload_on_unit_case_errors() {
    let code = r#"<?php
        enum Color { case Red; }
        return Color::Red("nope");
    "#;
    let res = run_phpx(code);
    match res {
        Err(VmError::RuntimeError(msg)) => {
            assert!(
                msg.contains("Enum case has no payload"),
                "unexpected msg {msg}"
            )
        }
        Err(other) => panic!("unexpected error variant {other:?}"),
        Ok(_) => panic!("vm unexpectedly succeeded"),
    }
}

#[test]
fn enum_cases_returns_descriptors() {
    let code = r#"<?php
        enum Msg { case Text(string $body); }
        $cases = Msg::cases();
        return $cases[0]->name;
    "#;
    let (vm, handle) = run_phpx(code).expect("vm run");
    assert_eq!(val_to_string(&vm, handle), "Text");
}

#[test]
fn enum_case_descriptor_equals_payload_value() {
    let code = r#"<?php
        enum Msg { case Text(string $body); }
        $case = Msg::cases()[0];
        return $case === Msg::Text("hi");
    "#;
    let (vm, handle) = run_phpx(code).expect("vm run");
    assert_eq!(val_to_string(&vm, handle), "1");
}
