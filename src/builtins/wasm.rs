use crate::builtins::json::{decode_json_to_handle, encode_handle_to_json};
use crate::core::value::{ArrayData, Handle, Val};
use crate::vm::engine::VM;
#[cfg(target_arch = "wasm32")]
use serde_json::Value as JsonValue;
use std::rc::Rc;

#[cfg(target_arch = "wasm32")]
use crate::wasm_exports::{php_free, WasmResult};

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn php_wasm_call(
        module_ptr: *const u8,
        module_len: u32,
        export_ptr: *const u8,
        export_len: u32,
        args_ptr: *const u8,
        args_len: u32,
    ) -> *mut WasmResult;
}

/// __deka_wasm_call(string $moduleId, string $export, ...$args): mixed
pub fn php_deka_wasm_call(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("__deka_wasm_call() expects at least 2 parameters".into());
    }

    let module_bytes = vm.value_to_string(args[0])?;
    let export_bytes = vm.value_to_string(args[1])?;

    let mut arg_array = ArrayData::with_capacity(args.len().saturating_sub(2));
    for handle in args.iter().skip(2) {
        arg_array.push(*handle);
    }
    let args_handle = vm.arena.alloc(Val::Array(Rc::new(arg_array)));

    let args_json = encode_handle_to_json(vm, args_handle)
        .map_err(|err| format!("wasm args encode error: {}", err.message()))?;

    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (module_bytes, export_bytes, args_json);
        return Err("Wasm extensions require the wasm runtime".into());
    }

    #[cfg(target_arch = "wasm32")]
    unsafe {
        let result_ptr = php_wasm_call(
            module_bytes.as_ptr(),
            module_bytes.len() as u32,
            export_bytes.as_ptr(),
            export_bytes.len() as u32,
            args_json.as_bytes().as_ptr(),
            args_json.as_bytes().len() as u32,
        );

        if result_ptr.is_null() {
            return Err("wasm call failed".into());
        }

        let result = &*result_ptr;
        let data_ptr = result.ptr as *const u8;
        let data_len = result.len as usize;

        let data = if data_ptr.is_null() || data_len == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(data_ptr, data_len).to_vec()
        };

        php_free(
            result_ptr as *mut u8,
            std::mem::size_of::<WasmResult>() as u32,
        );
        if !data_ptr.is_null() && data_len > 0 {
            php_free(data_ptr as *mut u8, data_len as u32);
        }

        if data.is_empty() {
            return Ok(vm.arena.alloc(Val::Null));
        }

        let json_str = std::str::from_utf8(&data)
            .map_err(|_| "wasm result invalid utf-8".to_string())?;
        let parsed: JsonValue = serde_json::from_str(json_str)
            .map_err(|err| format!("wasm result invalid json: {}", err))?;

        if let Some(err) = parsed
            .get("__deka_error")
            .and_then(|value| value.as_str())
        {
            return Err(format!("wasm error: {}", err));
        }

        let module_id = String::from_utf8_lossy(&module_bytes);
        let assoc = module_id.trim_matches('\0') == "__deka_db";
        decode_json_to_handle(vm, &parsed, assoc, 512)
            .map_err(|err| format!("wasm decode error: {}", err.message()))
    }
}
