//! Writes the opcodes, kinds and encoders from `ops.def` (p8-vmgen).
fn main() {
    println!("cargo:rerun-if-changed=ops.def");
    let text = std::fs::read_to_string("ops.def").expect("ops.def");
    let defs = p8_vmgen::parse(&text).unwrap_or_else(|e| panic!("{e}"));
    let out = std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("ops.rs");
    std::fs::write(out, p8_vmgen::bytecode(&defs)).expect("write ops.rs");
}
