#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! What p8's command-line hosts share: `php2` (the php command, named so
//! while p8 is built next to the legacy engine) and `p8` (`p8 compile`,
//! `p8 serve`). The web server is one, PHP's built-in one (`php2 -S`),
//! which `p8 serve` runs over a project's package.

pub mod http;
pub mod mime_types;
pub mod server;
pub mod status_phrases;

/// The extensions a host loads (`server`: with what the built-in web
/// server adds). A WASI build has no sockets, so it is a PHP built without
/// mysqli, curl, pdo_mysql, sockets and pdo_pgsql: they are not loaded.
pub fn extensions(server: bool) -> Vec<p8::Extension> {
    use p8::extensions::*;
    let mut all = vec![STANDARD];
    if server {
        all.push(SERVER);
    }
    all.extend([
        XML,
        TOKENIZER,
        MBSTRING,
        PCRE,
        #[cfg(not(target_os = "wasi"))]
        MYSQLI,
        #[cfg(not(target_os = "wasi"))]
        CURL,
        GD,
        ZIP,
        FILEINFO,
        SESSION,
        PDO,
        PDO_SQLITE,
        #[cfg(not(target_os = "wasi"))]
        PDO_MYSQL,
        #[cfg(not(target_os = "wasi"))]
        SOCKETS,
        OPENSSL,
        INTL,
        #[cfg(not(target_os = "wasi"))]
        PDO_PGSQL,
    ]);
    all
}

/// `opcache.file_cache=<dir>`: keep compiled files in that directory, for
/// the runs after this one (off unless set, as in PHP). It is OPcache's,
/// so it takes effect only when OPcache starts: in the CLI (`cli`) that
/// needs opcache.enable_cli=1 too, as in PHP 8.6.
pub fn file_cache(platform: p8::Platform, ini: &[(String, String)], cli: bool) -> p8::Platform {
    let value = |name: &str| ini.iter().rev().find(|(n, _)| n == name).map(|(_, v)| v.trim().to_string()).or_else(|| platform.ini_default(name));
    if !p8::OpcacheSettings::from_ini(cli, &value).started() {
        return platform;
    }
    let read_only = value("opcache.file_cache_read_only").is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "on" | "yes" | "true"));
    match value("opcache.file_cache") {
        Some(dir) if !dir.is_empty() && read_only => platform.read_only_file_cache(dir),
        Some(dir) if !dir.is_empty() => platform.file_cache(dir),
        _ => platform,
    }
}

/// Zend OPcache over the kept files, started as PHP starts it from these
/// settings (`-d`, else the built-in defaults): the CLI (`cli`) needs
/// opcache.enable_cli as well as opcache.enable; php -S does not.
pub fn opcache(platform: p8::Platform, ini: &[(String, String)], cli: bool) -> p8::Platform {
    let value = |name: &str| ini.iter().rev().find(|(n, _)| n == name).map(|(_, v)| v.clone()).or_else(|| platform.ini_default(name));
    let settings = p8::OpcacheSettings::from_ini(cli, &value);
    platform.opcache(settings)
}

/// Every PHP file under `dir` compiled into a package at `out`, for an
/// engine with the extensions of `server` (see [`extensions`]: a package
/// is used only by the engine that wrote it, extensions included), and
/// with `bake`, a copy of this executable carrying it. What it did, or
/// why it could not, is said under the command's name `name`. Returns the
/// exit status.
pub fn write_package(name: &str, server: bool, dir: &str, out: &std::path::Path, bake: Option<&str>) -> i32 {
    let platform = p8::Platform::new(&extensions(server));
    let packed = match platform.write_package(std::path::Path::new(dir), out) {
        Ok(packed) => packed,
        Err(e) => {
            eprintln!("{name}: could not write {}: {e}", out.display());
            return 1;
        }
    };
    for error in &packed.failed {
        eprintln!("{name}: left out {} (it does not compile: {} on line {})", error.file, error.message, error.line);
    }
    println!("{} files compiled into {} ({} bytes)", packed.files, out.display(), packed.bytes);
    if let Some(exe) = bake {
        if let Err(e) = platform.bake(out, std::path::Path::new(exe)) {
            eprintln!("{name}: could not write {exe}: {e}");
            return 1;
        }
        println!("{exe}: {name} with the package");
    }
    0
}

/// Run `f` with a native stack deep enough for deep PHP recursion: on a
/// thread with a 64 MiB stack. WASI has no threads; there the binary's
/// own stack is that size (see build.rs).
#[cfg(not(target_os = "wasi"))]
pub fn on_deep_stack(f: impl FnOnce() -> i32 + Send + 'static) -> i32 {
    std::thread::Builder::new().stack_size(64 * 1024 * 1024).spawn(f).expect("interpreter thread").join().unwrap_or(255)
}

/// Run `f` with a native stack deep enough for deep PHP recursion: WASI
/// has no threads, so the binary's own stack is 64 MiB (see build.rs).
#[cfg(target_os = "wasi")]
pub fn on_deep_stack(f: impl FnOnce() -> i32 + Send + 'static) -> i32 {
    f()
}
