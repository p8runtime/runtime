#![deny(unsafe_code)]
//! #397, MEASUREMENT ONLY: the harness that runs Cranelift's code for
//! V4b's `add`, `call` and `prop_read`. Everything but the boundary is
//! V4's (`aot`): the loader, the program data, the request state and the
//! cold helpers. The boundary (`ffi`) is the one module allowed `unsafe`.
//!
//!   clif <file.php> <n>   run t(n); output on stdout (as aot-b)

mod abi;
mod ffi;

use aot::rt::{self, Cache, Cx, St};
use aot::{b, k, load};
use p8_runtime::object::Ids;
use p8_runtime::string::IStr;
use p8_runtime::value::Value;
use std::process::ExitCode;
use std::rc::Rc;

fn istr(b: &[u8]) -> &'static IStr {
    IStr::intern(b).expect("room in the program-memory string pool")
}

/// `aot::main_with` with the generated code in place of V4b's.
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(file), Some(n)) = (args.first(), args.get(1).and_then(|s| s.parse::<i64>().ok())) else {
        eprintln!("usage: clif <file.php> <n>");
        return ExitCode::from(2);
    };
    let source = match std::fs::read(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{file}: {e}");
            return ExitCode::from(2);
        }
    };
    let program = match load::compile(&source, file) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{file}: {e}");
            return ExitCode::from(2);
        }
    };
    let data = match load::load(&program) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("refused: {e}");
            return ExitCode::from(2);
        }
    };
    let name = std::path::Path::new(file).file_stem().and_then(|s| s.to_str()).unwrap_or("").to_owned();
    let check = file.contains("/check/") || file.starts_with("check/");
    let mut lits = vec![Value::NULL; k::COUNT];
    lits[k::ZERO] = Value::int(0);
    lits[k::SPACE] = Value::istr(istr(b" "));
    lits[k::NL] = Value::istr(istr(b"\n"));
    lits[k::DASH] = Value::istr(istr(b"-"));
    lits[k::ONE] = Value::int(1);
    lits[k::FIVE] = Value::int(5);
    lits[k::TWO] = Value::int(2);
    lits[k::AB] = Value::istr(istr(b"ab"));
    lits[k::C] = Value::istr(istr(b"c"));
    lits[k::KEY_K] = Value::istr(istr(b"k"));
    let cx = Cx {
        classes: data.classes,
        k: lits,
        strs: [istr(b"ab"), istr(b"c"), istr(b"k")],
        caches: (0..4).map(|_| Cache::new()).collect(),
        methods: b::methods(),
        file: std::fs::canonicalize(file).map_or_else(|_| file.clone(), |p| p.to_string_lossy().into_owned()),
        check,
    };
    let mut st = St { ids: Rc::new(Ids::default()), out: Vec::new(), depth: 0 };
    let r = ffi::run(&name, &cx, &mut st, n);
    rt::flush(&mut st);
    match r {
        None => {
            eprintln!("refused: {name} is not compiled in this prototype ({:?})", abi::PROGRAMS.map(|p| p.0));
            ExitCode::from(2)
        }
        Some(Ok(_)) => ExitCode::SUCCESS,
        Some(Err(e)) => {
            eprintln!("fault: {e}");
            ExitCode::from(1)
        }
    }
}
