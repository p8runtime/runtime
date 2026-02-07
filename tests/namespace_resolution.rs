use bumpalo::Bump;
use php_rs::compiler::emitter::Emitter;
use php_rs::core::value::{Handle, Val};
use php_rs::parser::lexer::Lexer;
use php_rs::parser::parser::{Parser, ParserMode};
use php_rs::runtime::context::EngineBuilder;
use php_rs::vm::engine::{VM, VmError};
use std::rc::Rc;

fn run_php(code: &str) -> Result<(VM, Handle), VmError> {
    let engine = EngineBuilder::new()
        .with_core_extensions()
        .build()
        .expect("Failed to build engine");
    let mut vm = VM::new(engine);

    let arena = Bump::new();
    let lexer = Lexer::new(code.as_bytes());
    let mut parser = Parser::new_with_mode(lexer, &arena, ParserMode::Php);
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

fn val_to_int(vm: &VM, handle: Handle) -> i64 {
    match &vm.arena.get(handle).value {
        Val::Int(i) => *i,
        Val::Bool(b) => if *b { 1 } else { 0 },
        other => panic!("expected int, got {:?}", other),
    }
}

#[test]
fn namespace_bracketed_blocks_work() {
    let code = r#"<?php
        namespace Foo { function bar(){ return 1; } }
        namespace { function baz(){ return 2; } }
        return \Foo\bar() + baz();
    "#;
    let (vm, handle) = run_php(code).expect("vm run");
    assert_eq!(val_to_int(&vm, handle), 3);
}

#[test]
fn namespace_unqualified_fallback_and_use_aliases() {
    let code = r#"<?php
        namespace Foo;
        use function Bar\baz as baz_alias;

        function local() { return 1; }
        function call_local() { return local(); }
        function call_global() { return intdiv(9, 3); }
        function call_use() { return baz_alias(); }

        namespace Bar;
        function baz() { return 3; }

        namespace Foo;
        const L = 10;

        namespace;
        const G = 7;

        namespace Foo;
        return call_local() + call_use() + call_global() + L + G;
    "#;
    let (vm, handle) = run_php(code).expect("vm run");
    assert_eq!(val_to_int(&vm, handle), 24);
}
