mod common;

use anyhow::{Result, anyhow};
use common::run_script;
use std::env;
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        let bin_name = args.get(0).map(|s| s.as_str()).unwrap_or("php-wasm");
        return Err(anyhow!(
            "Usage: {} <script.php> [args...]\nPass the PHP-like script you want to execute.",
            bin_name
        ));
    }

    let script = PathBuf::from(&args[1]);
    run_script(&script, &args[2..].to_vec())
}
