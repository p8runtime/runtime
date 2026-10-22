//! Helpers for the Rust integration tests: run a PHP script through
//! `vm::executor` and return the value of its top-level `return`.

#![allow(dead_code)] // each test binary uses a different subset

use php_rs::core::value::Val;
use php_rs::vm::executor::execute_code;

/// Run `code` (with its `<?php` tag) and return the script's return value.
pub fn run_code(code: &str) -> Val {
    execute_code(code).expect("code execution failed").value
}

/// Alias for `run_code`.
pub fn run_php(code: &str) -> Val {
    run_code(code)
}
