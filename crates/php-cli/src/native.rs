//! The native option (#402, Phase 4), in one place: `p8 compile [dir]
//! --native <exe>` writes an executable holding the runtime, the package
//! and the package's functions as machine code ([`write`]); that
//! executable's runtime runs a script with them ([`run`], called by the
//! `p8-native` boundary crate).

use std::path::Path;

/// The runtime library a native executable is linked with, beside this
/// executable unless `--native-host` names it.
pub const HOST_LIBRARY: &str = "libp8_native.a";

/// Write the executable `exe`: the package at `package` (which `platform`
/// wrote), its functions compiled where they can be, linked with the
/// runtime library. Says what it wrote.
#[cfg(all(feature = "native", not(target_family = "wasm")))]
pub fn write(platform: &p8::Platform, package: &Path, host: Option<&str>, exe: &str) -> Result<String, String> {
    let host = match host {
        Some(h) => std::path::PathBuf::from(h),
        None => std::env::current_exe().map_err(|e| e.to_string())?.with_file_name(HOST_LIBRARY),
    };
    if !host.is_file() {
        return Err(format!("no runtime library at {} (it is built with `cargo build --release -p p8-native`; --native-host names another)", host.display()));
    }
    let functions = platform.packaged_functions(package).map_err(|e| e.to_string())?;
    let bytes = std::fs::read(package).map_err(|e| e.to_string())?;
    let inputs: Vec<p8_native_gen::Input> = functions.iter().map(|f| p8_native_gen::Input { file: f.file, index: f.index, function: &f.function }).collect();
    let built = p8_native_gen::build(&inputs, &bytes)?;
    p8_native_gen::link(&built.object, &host, Path::new(exe))?;
    Ok(format!("{exe}: the package, with {} of its {} functions as machine code ({})", built.native.len(), functions.len(), built.names.join(", ")))
}

/// Without the native backend (built with `--no-default-features`, or
/// for wasm).
#[cfg(not(all(feature = "native", not(target_family = "wasm"))))]
pub fn write(_: &p8::Platform, _: &Path, _: Option<&str>, _: &str) -> Result<String, String> {
    Err("this build has no native backend (the `native` feature)".to_string())
}

/// What a native executable carries: its machine code, which package
/// function each native function is (by id: file, function), the package.
pub struct Embedded {
    pub code: &'static dyn p8::native::NativeCode,
    pub keys: Vec<(u32, u32)>,
    pub package: Vec<u8>,
}

/// Run a native executable's script: `[-d name=value]... <file>
/// [args...]`, as p8 runs it with the executable's package, the
/// package's functions that have machine code run as it.
pub fn run(args: Vec<String>, native: Embedded) -> i32 {
    let mut args = args.into_iter().peekable();
    let mut ini = Vec::new();
    while let Some(arg) = args.next_if(|a| a.starts_with("-d")) {
        let text = if arg == "-d" { args.next().unwrap_or_default() } else { arg[2..].to_string() };
        let (name, value) = text.split_once('=').unwrap_or((&text, "1"));
        ini.push((name.trim().to_string(), value.trim().trim_matches('"').to_string()));
    }
    let Some(path) = args.next() else {
        eprintln!("usage: <this executable> [-d name=value]... <file> [args...]");
        return 64;
    };
    let source = match std::fs::read(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Could not open input file: {path} ({e})");
            return 1;
        }
    };
    let platform = p8::Platform::new(&crate::extensions(false)).keep_compiled();
    let package = match platform.package_from_bytes(native.package, p8::PackageCheck::Hash) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("this executable's package is not for its own engine: {e}");
            return 70;
        }
    };
    let platform = platform.with_package(package).with_native(native.code, &native.keys);
    let value = |name: &str| ini.iter().rev().find(|(n, _)| n == name).map(|(_, v)| v.clone()).or_else(|| platform.ini_default(name));
    let settings = p8::OpcacheSettings::from_ini(true, &value);
    let platform = platform.opcache(settings);
    let file = p8::canonicalize(&path).map_or(path.clone(), |p| p.to_string_lossy().into_owned());
    let argv: Vec<String> = std::iter::once(path).chain(args).collect();
    crate::on_deep_stack(move || {
        let mut isolate = platform.new_isolate();
        let mut stdout = crate::Stdout(std::io::stdout());
        let mut context = isolate.new_context(&mut stdout);
        for (name, value) in &ini {
            context.set_ini(name, value);
        }
        context.set_args(argv);
        match platform.compile(&source, &file) {
            Ok(script) => context.run(&script),
            Err(error) => context.fail(&error),
        }
    })
}
