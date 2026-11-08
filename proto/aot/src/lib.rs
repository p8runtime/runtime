#![forbid(unsafe_code)]
//! V4 of #365 (#370): the AOT ceiling. For each micro program V1 (#366)
//! runs, the safe Rust an ahead-of-time compiler would plausibly emit for
//! it, written by hand: one Rust function per PHP function, over the same
//! `p8-runtime` values and helpers as V1, with V1's guards (undefined
//! variables, references, int overflow, refcounts, typed properties, the
//! inline caches' class tests). Two flavours:
//!
//! - `a` (V4a, "dumb AOT"): the frame's variables in one array; every
//!   operation is a call of a generic, out-of-line helper (`rt::h_*`).
//! - `b` (V4b, "kind-aware AOT"): the variables as Rust locals; where the
//!   bytecode tells the operands' kinds (a literal of known type, a
//!   temporary that cannot be undefined, a call's result), the fast path
//!   inline and the generic helper as the cold fallback: V1's
//!   specialisations without the dispatch.
//!
//! Each compiled PHP function is `#[inline(never)]`: a call stays a call,
//! as it does in V1 and in PHP (inlining f into t is a further step a
//! compiler might take; it is not measured here).
//!
//!   aot-a|aot-b <file.php> <n>   run t(n); output on stdout
//!
//! The program is chosen by the file's name (`micro/<name>.php`; the
//! output-check variant `micro/check/<name>.php` also runs the echo after
//! the loop). Exit 0 when t ran, 2 for a program not compiled, 1 for a
//! fault.

pub mod a;
pub mod b;
pub mod load;
pub mod rt;

use p8_runtime::object::Ids;
use p8_runtime::string::IStr;
use p8_runtime::value::Value;
use rt::{Cache, Cx, St};
use std::process::ExitCode;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Flavour {
    A,
    B,
}

/// `t`'s literals, at fixed indexes (an AOT compiler's constant table):
/// the ones every program shares, then the ones some use.
pub mod k {
    pub const ZERO: usize = 0;
    pub const SPACE: usize = 1;
    pub const NL: usize = 2;
    pub const DASH: usize = 3;
    pub const ONE: usize = 4;
    pub const FIVE: usize = 5;
    pub const TWO: usize = 6;
    /// The array literal of the program, if it has one.
    pub const ARR: usize = 7;
    pub const AB: usize = 8;
    pub const C: usize = 9;
    pub const KEY_K: usize = 10;
    pub const COUNT: usize = 11;
}

/// The program-memory strings as V4b names them (no tag to test).
pub mod s {
    pub const AB: usize = 0;
    pub const C: usize = 1;
    pub const K: usize = 2;
}

fn istr(b: &[u8]) -> &'static IStr {
    IStr::intern(b).expect("room in the program-memory string pool")
}

pub fn main_with(flavour: Flavour) -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(file), Some(n)) = (args.first(), args.get(1).and_then(|s| s.parse::<i64>().ok())) else {
        eprintln!("usage: aot-a|aot-b <file.php> <n>");
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
    if let Some(arr) = data.t_consts.iter().find(|v| v.is_array()) {
        lits[k::ARR] = arr.clone();
    }
    lits[k::AB] = Value::istr(istr(b"ab"));
    lits[k::C] = Value::istr(istr(b"c"));
    lits[k::KEY_K] = Value::istr(istr(b"k"));
    let cx = Cx {
        classes: data.classes,
        k: lits,
        strs: [istr(b"ab"), istr(b"c"), istr(b"k")],
        caches: (0..4).map(|_| Cache::new()).collect(),
        methods: match flavour {
            Flavour::A => a::methods(),
            Flavour::B => b::methods(),
        },
        file: file.clone(),
        check,
    };
    let mut st = St { ids: Rc::new(Ids::default()), out: Vec::new(), depth: 0 };
    let r = match flavour {
        Flavour::A => a::run(&name, &cx, &mut st, n),
        Flavour::B => b::run(&name, &cx, &mut st, n),
    };
    rt::flush(&mut st);
    match r {
        None => {
            eprintln!("refused: {name} is not compiled in this prototype");
            ExitCode::from(2)
        }
        Some(Ok(_)) => ExitCode::SUCCESS,
        Some(Err(e)) => {
            eprintln!("fault: {e}");
            ExitCode::from(1)
        }
    }
}
