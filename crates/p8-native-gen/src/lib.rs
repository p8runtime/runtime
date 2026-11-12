#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! p8's native backend (Phase 4, #386, #387): Cranelift, embedded (pure
//! Rust, so making a native build needs no Rust toolchain), compiles the
//! functions of a package to machine code, and the system linker puts
//! them, the package and p8's runtime into one executable. A running p8
//! never loads machine code: the code exists only in executables built
//! this way (#387).
//!
//! What is compiled: each function every instruction of which has a
//! template (`tpl`, named by `ops.def`'s `native=`, #385) that takes its
//! operands. Everything else stays bytecode and runs on the interpreter,
//! as does any instruction a native function meets with values its code
//! does not handle (#388).
//!
//! The object file (`build`): one function per native PHP function; the
//! image (`p8_native_abi::image`): the table of those functions, which
//! package function each is, the package's bytes; and a C `main` that
//! hands the image to the boundary crate's entry (`p8_native_abi::ENTRY`).
//! [`link`] links it with the boundary crate's static library, the
//! runtime (`libp8_native.a`).

mod emit;
mod tpl;

use cranelift_codegen::ir::{self, AbiParam, InstBuilder, types};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{DataDescription, FuncId, Linkage, Module, default_libcall_names};
use cranelift_object::{ObjectBuilder, ObjectModule};
use p8_bytecode::Function;
use p8_native_abi::{self as abi, image};
use std::path::Path;

/// A function of the package to compile: which file (its record's number)
/// and which of the file's functions.
pub struct Input<'a> {
    pub file: u32,
    pub index: u32,
    pub function: &'a Function,
}

/// The object file, and what went into it.
pub struct Built {
    pub object: Vec<u8>,
    /// The functions compiled (file, function), by native id.
    pub native: Vec<(u32, u32)>,
    /// The names of the PHP functions compiled, by native id.
    pub names: Vec<String>,
}

fn err(what: &str) -> impl Fn(String) -> String + '_ {
    move |e| format!("{what}: {e}")
}

/// The ISA of this machine, with the settings of the Phase 0 prototype:
/// optimised for speed, position-independent code (an executable's).
fn isa() -> Result<cranelift_codegen::isa::OwnedTargetIsa, String> {
    let mut flags = settings::builder();
    flags.set("opt_level", "speed").map_err(|e| err("opt_level")(e.to_string()))?;
    flags.set("is_pic", "true").map_err(|e| err("is_pic")(e.to_string()))?;
    let isa = cranelift_native::builder().map_err(|e| err("this machine's ISA")(e.to_string()))?;
    isa.finish(settings::Flags::new(flags)).map_err(|e| err("the ISA")(e.to_string()))
}

/// Compile `functions` (those that can be) and write the object file of
/// an executable carrying `package`.
/// `engine` is the build of p8 making it (`Platform::engine_key`): the
/// runtime library refuses the image when it is not its own.
pub fn build(functions: &[Input], package: &[u8], engine: [u8; 16]) -> Result<Built, String> {
    let builder = ObjectBuilder::new(isa()?, "p8native", default_libcall_names()).map_err(|e| err("object")(e.to_string()))?;
    let mut module = ObjectModule::new(builder);
    let ptr = module.target_config().pointer_type();
    if ptr != types::I64 {
        return Err("native code needs a 64-bit target".to_string());
    }
    // The helper.
    let mut helper = module.make_signature();
    helper.params.extend([AbiParam::new(types::I64), AbiParam::new(types::I32), AbiParam::new(types::I32)]);
    helper.returns.push(AbiParam::new(types::I64));
    let call = module.declare_function(abi::HELP, Linkage::Import, &helper).map_err(|e| err("the helper")(e.to_string()))?;
    // A native function.
    let mut sig = module.make_signature();
    sig.params.push(AbiParam::new(types::I64));
    sig.returns.push(AbiParam::new(types::I64));
    let mut ctx = module.make_context();
    let (mut ids, mut native, mut names) = (Vec::new(), Vec::new(), Vec::new());
    for input in functions {
        ctx.func.signature = sig.clone();
        // (A fresh one: a function given up half built leaves its state.)
        let mut fbc = FunctionBuilderContext::new();
        let made = function(&mut module, &mut ctx.func, &mut fbc, call, input.function);
        if made.is_ok() {
            let id = native.len();
            let php_name = String::from_utf8_lossy(input.function.name.as_bytes()).into_owned();
            let fid = module.declare_function(&format!("p8n_f{id}"), Linkage::Local, &sig).map_err(|e| err("a function")(e.to_string()))?;
            module.define_function(fid, &mut ctx).map_err(|e| err(&format!("compiling {php_name}"))(format!("{e:?}")))?;
            ids.push(fid);
            native.push((input.file, input.index));
            names.push(php_name);
        }
        module.clear_context(&mut ctx);
    }
    image_and_main(&mut module, &ids, &native, package, engine)?;
    let object = module.finish().emit().map_err(|e| err("the object file")(e.to_string()))?;
    Ok(Built { object, native, names })
}

/// One PHP function's native function into `func`, or `NotNative`.
fn function(module: &mut ObjectModule, func: &mut ir::Function, fbc: &mut FunctionBuilderContext, call: FuncId, f: &Function) -> Result<(), emit::NotNative> {
    // A generator's call makes its Generator (never entered natively).
    if f.is_generator || f.insns.is_empty() || f.insns.len() > u32::MAX as usize {
        return Err(emit::NotNative);
    }
    let mut b = FunctionBuilder::new(func, fbc);
    let helper = module.declare_func_in_func(call, b.func);
    let entry = b.create_block();
    b.append_block_params_for_function_params(entry);
    let blocks: Vec<ir::Block> = f.insns.iter().map(|_| b.create_block()).collect();
    let next = b.declare_var(types::I32);
    let (dispatch, table, beyond) = (b.create_block(), b.create_block(), b.create_block());
    b.switch_to_block(entry);
    let ctx = b.block_params(entry)[0];
    let zero = b.ins().iconst(types::I32, 0);
    b.def_var(next, zero);
    b.ins().jump(blocks[0], &[]);
    // After a help: on at the pc it gave, by a jump table over the pcs. (A
    // pc past the code is the interpreter's to refuse: handed back there.)
    b.switch_to_block(dispatch);
    let at = b.use_var(next);
    let inside = b.ins().icmp_imm_u(ir::condcodes::IntCC::UnsignedLessThan, at, f.insns.len() as i64);
    b.ins().brif(inside, table, &[], beyond, &[]);
    b.switch_to_block(table);
    let calls: Vec<ir::BlockCall> = blocks.iter().map(|&k| b.func.dfg.block_call(k, &[])).collect();
    let default = b.func.dfg.block_call(beyond, &[]);
    let jt = b.create_jump_table(ir::JumpTableData::new(default, &calls));
    b.ins().br_table(at, jt);
    b.switch_to_block(beyond);
    b.set_cold_block(beyond);
    let wide = b.ins().uextend(types::I64, at);
    let code = b.ins().bor_imm_u(wide, (p8_native_abi::exit::GUARD << 32) as i64);
    b.ins().return_(&[code]);
    for (pc, insn) in f.insns.iter().enumerate() {
        b.switch_to_block(blocks[pc]);
        let regs = b.ins().load(types::I64, ir::MemFlagsData::trusted(), ctx, abi::CTX_REGS);
        let mut e = emit::Emit { b: &mut b, func: f, pc: pc as u32, blocks: &blocks, ctx, regs, helper, dispatch, next };
        tpl::instruction(&mut e, insn);
    }
    b.seal_all_blocks();
    b.finalize(module.target_config());
    Ok(())
}

/// The image (`p8_native_abi::image`), the table and keys it points to,
/// the package, and `main`, which passes the image to the entry.
fn image_and_main(module: &mut ObjectModule, ids: &[FuncId], native: &[(u32, u32)], package: &[u8], engine: [u8; 16]) -> Result<(), String> {
    let data = |module: &mut ObjectModule, name: &str, bytes: Vec<u8>| -> Result<cranelift_module::DataId, String> {
        let id = module.declare_data(name, Linkage::Local, false, false).map_err(|e| err(name)(e.to_string()))?;
        let mut d = DataDescription::new();
        d.define(bytes.into_boxed_slice());
        d.set_align(8);
        module.define_data(id, &d).map_err(|e| err(name)(e.to_string()))?;
        Ok(id)
    };
    let keys = data(module, "p8n_keys", native.iter().flat_map(|(f, i)| f.to_le_bytes().into_iter().chain(i.to_le_bytes())).chain([0; 8]).collect())?;
    // (One byte at least: an empty package is no symbol to point at.)
    let pkg = data(module, "p8n_package", if package.is_empty() { vec![0] } else { package.to_vec() })?;
    let table_id = module.declare_data("p8n_functions", Linkage::Local, false, false).map_err(|e| err("the table")(e.to_string()))?;
    let mut table = DataDescription::new();
    table.define(vec![0u8; 8 * ids.len().max(1)].into_boxed_slice());
    table.set_align(8);
    for (n, &fid) in ids.iter().enumerate() {
        let f = module.declare_func_in_data(fid, &mut table);
        table.write_function_addr((8 * n) as u32, f);
    }
    module.define_data(table_id, &table).map_err(|e| err("the table")(e.to_string()))?;
    let image_id = module.declare_data("p8n_image", Linkage::Local, false, false).map_err(|e| err("the image")(e.to_string()))?;
    let mut img = DataDescription::new();
    let mut bytes = vec![0u8; image::SIZE];
    let mut put = |at: i32, v: u64| bytes[at as usize..at as usize + 8].copy_from_slice(&v.to_le_bytes());
    put(image::MAGIC, image::MAGIC_WORD);
    put(image::VERSION, image::VERSION_WORD);
    put(image::COUNT, ids.len() as u64);
    put(image::PACKAGE_LEN, package.len() as u64);
    bytes[image::ENGINE as usize..image::ENGINE as usize + 16].copy_from_slice(&engine);
    img.define(bytes.into_boxed_slice());
    img.set_align(8);
    for (at, id) in [(image::FUNCTIONS, table_id), (image::KEYS, keys), (image::PACKAGE, pkg)] {
        let gv = module.declare_data_in_data(id, &mut img);
        img.write_data_addr(at as u32, gv, 0);
    }
    module.define_data(image_id, &img).map_err(|e| err("the image")(e.to_string()))?;
    // int main(int argc, char **argv) { return p8_native_main(argc, argv, &image); }
    let mut entry_sig = module.make_signature();
    entry_sig.params.extend([AbiParam::new(types::I32), AbiParam::new(types::I64), AbiParam::new(types::I64)]);
    entry_sig.returns.push(AbiParam::new(types::I32));
    let entry = module.declare_function(abi::ENTRY, Linkage::Import, &entry_sig).map_err(|e| err("the entry")(e.to_string()))?;
    let mut main_sig = module.make_signature();
    main_sig.params.extend([AbiParam::new(types::I32), AbiParam::new(types::I64)]);
    main_sig.returns.push(AbiParam::new(types::I32));
    let main = module.declare_function(abi::MAIN, Linkage::Export, &main_sig).map_err(|e| err("main")(e.to_string()))?;
    let mut ctx = module.make_context();
    ctx.func.signature = main_sig;
    let mut fbc = FunctionBuilderContext::new();
    {
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fbc);
        let entry_ref = module.declare_func_in_func(entry, b.func);
        let image_gv = module.declare_data_in_func(image_id, b.func);
        let block = b.create_block();
        b.append_block_params_for_function_params(block);
        b.switch_to_block(block);
        let (argc, argv) = (b.block_params(block)[0], b.block_params(block)[1]);
        let image = b.ins().symbol_value(types::I64, image_gv);
        let inst = b.ins().call(entry_ref, &[argc, argv, image]);
        let status = b.inst_results(inst)[0];
        b.ins().return_(&[status]);
        b.seal_all_blocks();
        b.finalize(module.target_config());
    }
    module.define_function(main, &mut ctx).map_err(|e| err("main")(format!("{e:?}")))?;
    Ok(())
}

/// The system libraries a Rust static library links against on this
/// platform (`rustc --print native-static-libs` for `libp8_native.a`).
fn system_libs() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &["-liconv", "-framework", "CoreFoundation", "-lSystem", "-lc", "-lm"]
    } else {
        &["-lgcc_s", "-lutil", "-lrt", "-lpthread", "-lm", "-ldl", "-lc"]
    }
}

/// Link `object` with the runtime (`host`, `libp8_native.a`) into the
/// executable `out`, with the system's C compiler driver (`cc`), which
/// runs the system linker: Xcode's command line tools on macOS, a C
/// toolchain (binutils and libc's development files) on Linux. No Rust
/// toolchain. The object is written beside `out` while it links.
pub fn link(object: &[u8], host: &Path, out: &Path) -> Result<(), String> {
    let obj = out.with_extension("p8native.o");
    std::fs::write(&obj, object).map_err(|e| format!("{}: {e}", obj.display()))?;
    let dead = if cfg!(target_os = "macos") { "-Wl,-dead_strip" } else { "-Wl,--gc-sections" };
    let run = std::process::Command::new("cc").arg("-o").arg(out).arg(&obj).arg(host).arg(dead).args(system_libs()).output();
    let _ = std::fs::remove_file(&obj);
    let run = run.map_err(|e| format!("could not run the system linker (cc): {e}"))?;
    if !run.status.success() {
        return Err(format!("the system linker (cc) failed:\n{}", String::from_utf8_lossy(&run.stderr)));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
