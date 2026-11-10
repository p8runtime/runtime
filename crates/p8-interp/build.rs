//! Writes the loop's dispatch arms for the generated opcodes from
//! p8-bytecode's `ops.def` (p8-vmgen).
fn main() {
    let path = "../p8-bytecode/ops.def";
    println!("cargo:rerun-if-changed={path}");
    let text = std::fs::read_to_string(path).expect("ops.def");
    let defs = p8_vmgen::parse(&text).unwrap_or_else(|e| panic!("{e}"));
    let out = std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("dispatch.rs");
    std::fs::write(out, p8_vmgen::dispatch(&defs)).expect("write dispatch.rs");
}
