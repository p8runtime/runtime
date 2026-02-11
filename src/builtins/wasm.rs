#[cfg(target_arch = "wasm32")]
use crate::builtins::json::decode_json_to_handle;
use crate::builtins::json::encode_handle_to_json;
use crate::core::value::{ArrayData, Handle, PromiseData, PromiseState, Val};
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
    fn php_host_call(
        kind_ptr: *const u8,
        kind_len: u32,
        action_ptr: *const u8,
        action_len: u32,
        payload_ptr: *const u8,
        payload_len: u32,
    ) -> *mut WasmResult;
}

/// __deka_wasm_call(string $moduleId, string $export, ...$args): mixed
pub fn php_deka_wasm_call(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("__deka_wasm_call() expects at least 2 parameters".into());
    }
    let (caller_file, caller_line) = vm.current_file_line();
    if !is_internal_bridge_caller_path(&caller_file) {
        return Err(format!(
            "__deka_wasm_call() is internal-only (caller: {}:{})",
            caller_file, caller_line
        ));
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
        let module_name = module_id.trim_matches('\0');
        let assoc = module_name.starts_with("__deka_");
        decode_json_to_handle(vm, &parsed, assoc, 512)
            .map_err(|err| format!("wasm decode error: {}", err.message()))
    }
}

/// __deka_wasm_call_async(string $moduleId, string $export, ...$args): Promise<mixed>
pub fn php_deka_wasm_call_async(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let value = php_deka_wasm_call(vm, args)?;
    Ok(vm.arena.alloc(Val::Promise(Rc::new(PromiseData {
        state: PromiseState::Resolved(value),
    }))))
}

/// __bridge(string $kind, string $action, mixed $payload = {}): mixed
pub fn php_bridge_call(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("__bridge() expects at least 2 parameters".into());
    }
    let (caller_file, caller_line) = vm.current_file_line();
    if !is_internal_bridge_caller_path(&caller_file) {
        return Err(format!(
            "__bridge() is internal-only (caller: {}:{})",
            caller_file, caller_line
        ));
    }

    let kind_bytes = vm.value_to_string(args[0])?;
    let action_bytes = vm.value_to_string(args[1])?;
    let payload_handle = if args.len() >= 3 {
        args[2]
    } else {
        vm.arena.alloc(Val::Array(Rc::new(ArrayData::new())))
    };
    let payload_json = encode_handle_to_json(vm, payload_handle)
        .map_err(|err| format!("bridge payload encode error: {}", err.message()))?;

    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (kind_bytes, action_bytes, payload_json);
        return Err("Bridge host calls require the wasm runtime".into());
    }

    #[cfg(target_arch = "wasm32")]
    unsafe {
        let result_ptr = php_host_call(
            kind_bytes.as_ptr(),
            kind_bytes.len() as u32,
            action_bytes.as_ptr(),
            action_bytes.len() as u32,
            payload_json.as_bytes().as_ptr(),
            payload_json.as_bytes().len() as u32,
        );

        if result_ptr.is_null() {
            return Err("bridge call failed".into());
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
            .map_err(|_| "bridge result invalid utf-8".to_string())?;
        let parsed: JsonValue = serde_json::from_str(json_str)
            .map_err(|err| format!("bridge result invalid json: {}", err))?;

        if let Some(err) = parsed.get("__deka_error").and_then(|value| value.as_str()) {
            return Err(format!("bridge error: {}", err));
        }

        decode_json_to_handle(vm, &parsed, true, 512)
            .map_err(|err| format!("bridge decode error: {}", err.message()))
    }
}

/// __bridge_async(string $kind, string $action, mixed $payload = {}): Promise<mixed>
pub fn php_bridge_call_async(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let value = php_bridge_call(vm, args)?;
    Ok(vm.arena.alloc(Val::Promise(Rc::new(PromiseData {
        state: PromiseState::Resolved(value),
    }))))
}

fn is_internal_bridge_caller_path(file: &str) -> bool {
    if file.is_empty() || file == "unknown" {
        return false;
    }
    let segments: Vec<&str> = file
        .split(['/', '\\'])
        .filter(|seg| !seg.is_empty())
        .collect();
    let Some(modules_idx) = segments.iter().position(|seg| *seg == "php_modules") else {
        return false;
    };

    // Allow both source modules and compiled cache paths:
    // - php_modules/core/*
    // - php_modules/internals/*
    // - php_modules/.cache/phpx/core/*
    // - php_modules/.cache/phpx/internals/*
    let mut idx = modules_idx + 1;
    if idx >= segments.len() {
        return false;
    }
    if segments[idx] == ".cache" {
        idx += 1;
        if idx < segments.len() && segments[idx] == "phpx" {
            idx += 1;
        }
    }
    if idx >= segments.len() {
        return false;
    }
    matches!(segments[idx], "core" | "internals")
}

#[cfg(test)]
mod tests {
    use super::is_internal_bridge_caller_path;

    #[test]
    fn internal_bridge_path_allows_internal_modules() {
        assert!(is_internal_bridge_caller_path(
            "/app/php_modules/internals/wasm.phpx"
        ));
        assert!(is_internal_bridge_caller_path(
            "/app/php_modules/core/bridge.phpx"
        ));
        assert!(is_internal_bridge_caller_path(
            "/app/php_modules/.cache/phpx/core/bridge.php"
        ));
        assert!(is_internal_bridge_caller_path(
            "/app/php_modules/.cache/phpx/internals/wasm.php"
        ));
        assert!(is_internal_bridge_caller_path(
            "C:\\repo\\php_modules\\internals\\wasm.phpx"
        ));
    }

    #[test]
    fn internal_bridge_path_rejects_non_internal_modules() {
        assert!(!is_internal_bridge_caller_path("/app/index.phpx"));
        assert!(!is_internal_bridge_caller_path("/app/php_modules/db/index.phpx"));
        assert!(!is_internal_bridge_caller_path("unknown"));
    }
}
