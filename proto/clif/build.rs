#![forbid(unsafe_code)]
//! Links the object file `clif-gen` wrote (`out/clif.o`) into the harness
//! binary `clif` only, by the system linker rustc already drives (no C
//! compiler, no C code: the object is Cranelift's). `scripts/build.sh`
//! builds `clif-gen`, runs it, then builds `clif`.

fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir");
    let obj = std::path::Path::new(&dir).join("out/clif.o");
    println!("cargo:rerun-if-changed={}", obj.display());
    println!("cargo:rustc-link-arg-bin=clif={}", obj.display());
}
