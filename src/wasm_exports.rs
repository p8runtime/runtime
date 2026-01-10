use crate::compiler::emitter::Emitter;
use crate::runtime::context::{EngineBuilder, RequestContext};
use crate::vm::engine::{CapturingErrorHandler, CapturingOutputWriter, ErrorLevel, VM};
use bumpalo::Bump;
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn gild_log(ptr: *const u8, len: u32);
}

#[repr(C)]
pub struct WasmResult {
    pub ptr: u32,
    pub len: u32,
}

#[unsafe(no_mangle)]
pub extern "C" fn gild_alloc(size: u32) -> *mut u8 {
    let mut buf = Vec::with_capacity(size as usize);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

#[unsafe(no_mangle)]
pub extern "C" fn gild_free(ptr: *mut u8, size: u32) {
    if ptr.is_null() || size == 0 {
        return;
    }
    unsafe {
        drop(Vec::from_raw_parts(ptr, size as usize, size as usize));
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gild_run(ptr: *const u8, len: u32) -> *mut WasmResult {
    let result_json = match source_from_ptr(ptr, len) {
        Ok(source) => run_php_source(&source),
        Err(message) => json!({
            "ok": false,
            "stdout": "",
            "stderr": "",
            "error": message,
        })
        .to_string(),
    };

    let out_bytes = result_json.as_bytes();
    let out_ptr = gild_alloc(out_bytes.len() as u32);
    unsafe {
        std::ptr::copy_nonoverlapping(out_bytes.as_ptr(), out_ptr, out_bytes.len());
    }

    let result = Box::new(WasmResult {
        ptr: out_ptr as u32,
        len: out_bytes.len() as u32,
    });

    Box::into_raw(result)
}

fn source_from_ptr(ptr: *const u8, len: u32) -> Result<String, String> {
    if ptr.is_null() || len == 0 {
        return Ok(String::new());
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
    std::str::from_utf8(bytes)
        .map(|s| s.to_string())
        .map_err(|err| format!("Invalid UTF-8 input: {}", err))
}

fn run_php_source(source: &str) -> String {
    log("gild_run:start");
    let arena = Bump::new();
    let lexer = crate::parser::lexer::Lexer::new(source.as_bytes());
    let mut parser = crate::parser::parser::Parser::new(lexer, &arena);
    let program = parser.parse_program();

    if !program.errors.is_empty() {
        log("gild_run:parse_error");
        let mut stderr = String::new();
        for error in program.errors {
            stderr.push_str(&error.to_human_readable(source.as_bytes()));
            stderr.push('\n');
        }
        return json!({
            "ok": false,
            "stdout": "",
            "stderr": stderr,
            "error": "parse_error",
        })
        .to_string();
    }

    log("gild_run:engine_build");
    let engine = match EngineBuilder::new().with_core_extensions().build() {
        Ok(engine) => engine,
        Err(message) => {
            return json!({
                "ok": false,
                "stdout": "",
                "stderr": "",
                "error": format!("engine_error: {}", message),
            })
            .to_string();
        }
    };

    log("gild_run:emit");
    let mut request_context = RequestContext::new(engine);
    let emitter = Emitter::new(source.as_bytes(), &mut request_context.interner);
    let (chunk, _) = emitter.compile(&program.statements);

    log("gild_run:vm_init");
    let mut vm = VM::new_with_context(request_context);
    vm.allow_file_io = false;
    vm.allow_network = false;

    let captured_stdout = Rc::new(RefCell::new(Vec::<u8>::new()));
    let captured_stderr = Rc::new(RefCell::new(Vec::<u8>::new()));

    let stdout_clone = captured_stdout.clone();
    vm.set_output_writer(Box::new(CapturingOutputWriter::new(move |bytes| {
        stdout_clone.borrow_mut().extend_from_slice(bytes);
    })));

    let stderr_clone = captured_stderr.clone();
    vm.set_error_handler(Box::new(CapturingErrorHandler::new(
        move |level, message| {
            let level_str = match level {
                ErrorLevel::Notice => "Notice",
                ErrorLevel::Warning => "Warning",
                ErrorLevel::Error => "Error",
                ErrorLevel::ParseError => "Parse error",
                ErrorLevel::UserNotice => "User notice",
                ErrorLevel::UserWarning => "User warning",
                ErrorLevel::UserError => "User error",
                ErrorLevel::Deprecated => "Deprecated",
            };
            let formatted = format!("{}: {}\n", level_str, message);
            stderr_clone
                .borrow_mut()
                .extend_from_slice(formatted.as_bytes());
        },
    )));

    log("gild_run:vm_run");
    let result = vm.run(Rc::new(chunk));
    log("gild_run:vm_done");
    let stdout = String::from_utf8_lossy(&captured_stdout.borrow()).into_owned();
    let stderr = String::from_utf8_lossy(&captured_stderr.borrow()).into_owned();

    match result {
        Ok(()) => json!({
            "ok": true,
            "stdout": stdout,
            "stderr": stderr,
            "error": "",
        })
        .to_string(),
        Err(err) => json!({
            "ok": false,
            "stdout": stdout,
            "stderr": stderr,
            "error": format!("{:?}", err),
        })
        .to_string(),
    }
}

fn log(message: &str) {
    unsafe {
        gild_log(message.as_ptr(), message.len() as u32);
    }
}
