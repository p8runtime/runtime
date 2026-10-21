use anyhow::Context;
use bumpalo::Bump;
use indexmap::IndexMap;
use php_rs::{
    compiler::emitter::Emitter,
    core::value::{ArrayData, ArrayKey, Val},
    parser::lexer::Lexer,
    parser::parser::Parser as PhpParser,
    runtime::context::{EngineBuilder, EngineContext},
    vm::engine::{VM, VmError},
};
use std::env;
use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

pub fn create_engine() -> anyhow::Result<Arc<EngineContext>> {
    let builder = EngineBuilder::new();

    builder
        .with_core_extensions()
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build engine: {}", e))
}

pub fn run_script(script_path: &Path, args: &[String]) -> anyhow::Result<()> {
    let source = fs::read_to_string(script_path)
        .with_context(|| format!("Failed to read script: {}", script_path.display()))?;

    let canonical_path = script_path
        .canonicalize()
        .unwrap_or_else(|_| script_path.to_path_buf());

    #[cfg(not(target_arch = "wasm32"))]
    if let Some(parent) = canonical_path.parent() {
        env::set_current_dir(parent)?;
    }

    let script_name = canonical_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("script")
        .to_string();

    let engine_context = create_engine()?;
    let mut vm = VM::new(engine_context);

    if let Some(server_handle) = vm
        .context
        .globals
        .get(&vm.context.interner.intern(b"_SERVER"))
        .copied()
    {
        let mut array_data_rc = if let Val::Array(rc) = &vm.arena.get(server_handle).value {
            rc.clone()
        } else {
            Rc::new(ArrayData::new())
        };

        let script_filename = canonical_path.to_string_lossy().into_owned();
        let val_handle_filename = vm
            .arena
            .alloc(Val::String(Rc::new(script_filename.into_bytes())));

        let script_name_str = script_path.to_string_lossy().into_owned();
        let val_handle_script_name = vm
            .arena
            .alloc(Val::String(Rc::new(script_name_str.clone().into_bytes())));

        let val_handle_php_self = vm
            .arena
            .alloc(Val::String(Rc::new(script_name_str.into_bytes())));

        let doc_root = canonical_path
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let val_handle_doc_root = vm.arena.alloc(Val::String(Rc::new(doc_root.into_bytes())));

        let pwd = env::current_dir()
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let val_handle_pwd = vm.arena.alloc(Val::String(Rc::new(pwd.into_bytes())));

        let array_data: &mut ArrayData = Rc::make_mut(&mut array_data_rc);
        array_data.insert(
            ArrayKey::Str(Rc::new(b"SCRIPT_FILENAME".to_vec())),
            val_handle_filename,
        );
        array_data.insert(
            ArrayKey::Str(Rc::new(b"SCRIPT_NAME".to_vec())),
            val_handle_script_name,
        );
        array_data.insert(
            ArrayKey::Str(Rc::new(b"PHP_SELF".to_vec())),
            val_handle_php_self,
        );
        array_data.insert(
            ArrayKey::Str(Rc::new(b"DOCUMENT_ROOT".to_vec())),
            val_handle_doc_root,
        );
        array_data.insert(ArrayKey::Str(Rc::new(b"PWD".to_vec())), val_handle_pwd);

        let slot = vm.arena.get_mut(server_handle);
        slot.value = Val::Array(array_data_rc);
    }

    let mut argv_map = IndexMap::new();
    argv_map.insert(
        ArrayKey::Int(0),
        vm.arena
            .alloc(Val::String(Rc::new(script_name.into_bytes()))),
    );

    for (i, arg) in args.iter().enumerate() {
        argv_map.insert(
            ArrayKey::Int((i + 1) as i64),
            vm.arena
                .alloc(Val::String(Rc::new(arg.clone().into_bytes()))),
        );
    }

    let argv_handle = vm.arena.alloc(Val::Array(ArrayData::from(argv_map).into()));
    let argc_handle = vm.arena.alloc(Val::Int((args.len() + 1) as i64));

    let argv_symbol = vm.context.interner.intern(b"argv");
    let argc_symbol = vm.context.interner.intern(b"argc");

    vm.context.globals.insert(argv_symbol, argv_handle);
    vm.context.globals.insert(argc_symbol, argc_handle);

    execute_source(&source, Some(&canonical_path), &mut vm)
        .map_err(|e| anyhow::anyhow!("VM Error: {:?}", e))?;

    Ok(())
}

pub fn execute_source(
    source: &str,
    file_path: Option<&Path>,
    vm: &mut VM,
) -> std::result::Result<(), VmError> {
    let source_bytes = source.as_bytes();
    let arena = Bump::new();
    let lexer = Lexer::new(source_bytes);
    let mut parser = PhpParser::new(lexer, &arena);

    let program = parser.parse_program();

    if !program.errors.is_empty() {
        for error in program.errors {
            println!("{}", error.to_human_readable(source_bytes));
        }
        return Ok(());
    }

    let mut emitter = Emitter::new(source_bytes, &mut vm.context.interner);
    if let Some(path) = file_path {
        let path_string = path.to_string_lossy().into_owned();
        emitter = emitter.with_file_path(path_string);
    }

    let (chunk, _has_error) = emitter.compile(program.statements);
    vm.run(Rc::new(chunk))?;

    Ok(())
}
