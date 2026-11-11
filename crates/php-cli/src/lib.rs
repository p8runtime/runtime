#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! What p8 shares with the executables it builds (`p8 compile --native`,
//! whose runtime is the `p8-native` crate's library): the extensions it
//! loads (a package is used only by an engine with the same ones), the
//! terminal host, the deep stack, and the native option itself
//! ([`native`]).

use std::io::Write;

pub mod native;

/// The extensions p8 loads (`server`: with what the built-in web server
/// adds). A WASI build has no sockets, so it is a PHP built without
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

/// The terminal: a request's output goes to stdout, written through on
/// every write as PHP's CLI does (sapi_cli_ub_write writes each piece to
/// fd 1; implicit_flush is on). Nothing waits in a buffer, so output stays
/// in order with STDERR, a prompt shows before reading STDIN, and
/// fwrite(STDOUT)/fflush(STDOUT) reach the reader at once.
pub struct Stdout(pub std::io::Stdout);

impl p8::Host for Stdout {
    fn write(&mut self, bytes: &[u8]) {
        let mut out = self.0.lock();
        let _ = out.write_all(bytes);
        let _ = out.flush();
    }

    fn flush(&mut self) {
        let _ = self.0.flush();
    }
}
