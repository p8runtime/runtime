//! `include` and `require`: finding the file (include_path, then the
//! including file's directory), compiling it through the embedder's
//! loader, and declaring what it declares.
//!
//! Reference: $PHP_SRC_PATH/main/fopen_wrappers.c (php_resolve_path),
//! $PHP_SRC_PATH/Zend/zend_execute.c (zend_include_or_eval)

use crate::Context;
use p8_bytecode::{IncludeKind, Program};
use p8_runtime::ext::{Ctx, Throw};
use p8_runtime::user_stream::{self, Located};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

/// Compiles a file's source: the embedder's compiler, given to a context.
pub trait Loader {
    fn load(&self, source: &[u8], file: &str) -> Result<Arc<Program>, LoadError>;
    /// eval()'s code (no `<?php`; it returns null unless it returns).
    fn load_eval(&self, code: &[u8], file: &str) -> Result<Arc<Program>, LoadError>;
}

/// A file that does not compile.
#[derive(Debug, Clone)]
pub struct LoadError {
    /// A parse error (else a compile-time fatal error).
    pub parse: bool,
    pub message: String,
    pub line: u32,
}

impl Context<'_> {
    /// The file `include`/`require` names. Ok(None): a `_once` of a file
    /// already included; Ok(Some(Err)): an include that failed (its
    /// warnings shown); Err: a require that failed, or the file does not
    /// compile.
    pub(crate) fn load_include(
        &mut self,
        name: &Value,
        kind: IncludeKind,
        current: &str,
    ) -> Result<Option<Result<Arc<Program>, ()>>, Throw> {
        let name = name.to_php_string().unwrap_or_else(PhpStr::empty);
        let shown = String::from_utf8_lossy(name.as_bytes()).into_owned();
        let include_path = self.include_path();
        let verb = kind.name();
        if !name.is_empty() {
            match user_stream::locate(self, verb, &shown) {
                Located::User(wrapper, local) => {
                    // A plain path the include_path finds goes to a file://
                    // wrapper resolved, as PHP resolves it before opening.
                    let local = if local.contains("://") { local } else { resolve(&local, &include_path, current).map_or(local, |p| p.to_string_lossy().into_owned()) };
                    return self.load_user_include(&wrapper, &local, &shown, kind, &include_path);
                }
                Located::Disabled => {
                    user_stream::warn_disabled(self, verb);
                    self.warn(format!("{verb}({shown}): Failed to open stream: no suitable wrapper could be found"));
                    return self.include_failed(kind, &shown, &include_path);
                }
                Located::Builtin => {}
            }
        }
        let found = if name.is_empty() { None } else { resolve(&shown, &include_path, current) };
        let source = found.as_ref().and_then(|p| std::fs::read(p).ok().map(|s| (p.clone(), s)));
        let Some((path, source)) = source else {
            if name.is_empty() {
                self.warn(format!("{verb}(): Filename cannot be empty"));
            } else {
                self.warn(format!("{verb}({shown}): Failed to open stream: No such file or directory"));
            }
            return self.include_failed(kind, &shown, &include_path);
        };
        let real = p8_runtime::os::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if kind.once() && self.included.contains(&real) {
            return Ok(None);
        }
        // The file is known by its resolved path (__FILE__, __DIR__), as PHP
        // opens it, not as the include named it.
        let file = real.to_string_lossy().into_owned();
        self.compile_included(real, file, &source).map(|p| Some(Ok(p)))
    }

    /// An include that found nothing to open: a require's Error, or an
    /// include's warning (and false).
    fn include_failed(&mut self, kind: IncludeKind, shown: &str, include_path: &str) -> Result<Option<Result<Arc<Program>, ()>>, Throw> {
        if kind.required() {
            return Err(self.throw("Error", format!("Failed opening required '{shown}' (include_path='{include_path}')")));
        }
        self.warn(format!("{}(): Failed opening '{shown}' for inclusion (include_path='{include_path}')", kind.name()));
        Ok(Some(Err(())))
    }

    /// An include through a user-space wrapper: opened as PHP opens code
    /// (mode "rb", STREAM_OPEN_FOR_INCLUDE, its read buffer asked off with
    /// stream_set_option), known by the path stream_open gave back (else
    /// the URL); a _once of a file already included is closed there. Else
    /// read (its size from stream_stat), compiled, then closed. A wrapper
    /// registered as a URL one is refused unless allow_url_include.
    fn load_user_include(&mut self, wrapper: &Rc<user_stream::Wrapper>, url: &str, shown: &str, kind: IncludeKind, include_path: &str) -> Result<Option<Result<Arc<Program>, ()>>, Throw> {
        let verb = kind.name();
        if wrapper.is_url && !Ctx::ini_get(self, b"allow_url_include").is_some_and(|v| v.to_bool()) {
            let scheme = shown.split("://").next().unwrap_or(shown);
            self.warn(format!("{verb}(): {scheme}:// wrapper is disabled in the server configuration by allow_url_include=0"));
            self.warn(format!("{verb}({shown}): Failed to open stream: no suitable wrapper could be found"));
            return self.include_failed(kind, shown, include_path);
        }
        let options = user_stream::USE_PATH | user_stream::OPEN_FOR_INCLUDE | user_stream::OPEN_FOR_ZEND_STREAM;
        let (mut stream, opened) = match user_stream::open(self, wrapper, url, "rb", options, Value::Null, false)? {
            Ok(opened) => opened,
            Err(reason) => {
                self.warn(format!("{verb}({shown}): Failed to open stream: {reason}"));
                return self.include_failed(kind, shown, include_path);
            }
        };
        // PHP_STREAM_OPTION_READ_BUFFER, PHP_STREAM_BUFFER_NONE.
        user_stream::set_option(self, verb, &mut stream, user_stream::OPTION_READ_BUFFER, Value::Int(0), None)?;
        let file = opened.map_or_else(|| url.to_string(), |p| String::from_utf8_lossy(p.as_bytes()).into_owned());
        let real = PathBuf::from(&file);
        if kind.once() && self.included.contains(&real) {
            user_stream::close(self, &mut stream)?;
            return Ok(None);
        }
        let size = user_stream::stat(self, verb, &mut stream)?.map_or(0, |s| s.size.max(0) as usize);
        let mut source = Vec::new();
        loop {
            let want = if size > 0 { size.saturating_sub(source.len()) } else { 4096 };
            match user_stream::read(self, verb, &mut stream, want)? {
                Some(bytes) if !bytes.is_empty() => source.extend(bytes),
                _ => break,
            }
        }
        // A file that does not compile leaves the stream to go unclosed, as
        // PHP's close finds the exception pending.
        let program = self.compile_included(real, file, &source)?;
        user_stream::close(self, &mut stream)?;
        Ok(Some(Ok(program)))
    }

    /// An included file's code compiled and declared, the file counted as
    /// included under `file`.
    fn compile_included(&mut self, real: PathBuf, file: String, source: &[u8]) -> Result<Arc<Program>, Throw> {
        self.included_order.push(file.clone());
        self.included.insert(real);
        let loader = match &self.loader {
            Some(l) => Rc::clone(l),
            None => return Err(self.throw("Error", format!("Failed opening required '{file}' (no compiler)"))),
        };
        let program = match loader.load(source, &file) {
            Ok(p) => p,
            // A ParseError the includer can catch (PHP 7+).
            Err(e) if e.parse => return Err(Throw::Error { class: "ParseError", message: e.message, file: Rc::from(file.as_str()), line: e.line, props: Vec::new() }),
            Err(e) => return Err(Throw::Fatal { message: e.message, file: Rc::from(file.as_str()), line: e.line }),
        };
        for f in &program.functions {
            if let Err(message) = self.declare(Arc::clone(f)) {
                return Err(Throw::Fatal { message, file: Rc::from(&*f.file), line: f.line });
            }
        }
        self.early_bind(&program.classes, Some(Rc::from(&*program.main.file)))?;
        Ok(program)
    }
}

impl Context<'_> {
    fn include_path(&self) -> String {
        match Ctx::ini_get(self, b"include_path") {
            Some(Value::Str(s)) => String::from_utf8_lossy(s.as_bytes()).into_owned(),
            _ => ".".to_string(),
        }
    }

    /// Where `name` is found the way an include looks for it, as a
    /// canonical path (stream_resolve_include_path()). Like PHP's
    /// php_resolve_path, anything that exists counts, a directory too (so
    /// "" names the first include_path directory).
    pub(crate) fn resolve_include_path(&self, name: &str) -> Option<PathBuf> {
        let current = self.location().0;
        let path = Path::new(name);
        let candidates: Vec<PathBuf> = if path.is_absolute() || name.starts_with("./") || name.starts_with("../") {
            vec![path.to_path_buf()]
        } else {
            let mut c: Vec<PathBuf> = self.include_path().split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join(name)).collect();
            c.extend(Path::new(&*current).parent().map(|d| d.join(name)));
            c
        };
        candidates.into_iter().find_map(|p| p8_runtime::os::canonicalize(p).ok())
    }

    /// eval(): the code compiled, its functions and classes declared. A
    /// parse error is a ParseError thrown where eval() was called.
    pub(crate) fn load_eval(&mut self, code: &Value) -> Result<Arc<Program>, Throw> {
        let code = code.to_php_string().unwrap_or_else(PhpStr::empty);
        let (file, line) = self.location();
        let name = format!("{file}({line}) : eval()'d code");
        let Some(loader) = self.loader.clone() else {
            return Err(self.throw("Error", "eval() needs a compiler"));
        };
        let program = match loader.load_eval(code.as_bytes(), &name) {
            Ok(p) => p,
            Err(e) if e.parse => return Err(Throw::Error { class: "ParseError", message: e.message, file: Rc::from(name.as_str()), line: e.line, props: Vec::new() }),
            Err(e) => return Err(Throw::Fatal { message: e.message, file: Rc::from(name.as_str()), line: e.line }),
        };
        for f in &program.functions {
            if let Err(message) = self.declare(Arc::clone(f)) {
                return Err(Throw::Fatal { message, file: Rc::from(&*f.file), line: f.line });
            }
        }
        self.early_bind(&program.classes, Some(Rc::from(&*program.main.file)))?;
        Ok(program)
    }
}

/// php_resolve_path: a path that is absolute or starts with ./ or ../ is
/// taken from the working directory; any other is looked for in each
/// include_path directory, then in the including file's directory.
fn resolve(name: &str, include_path: &str, current: &str) -> Option<PathBuf> {
    let path = Path::new(name);
    if path.is_absolute() || name.starts_with("./") || name.starts_with("../") {
        return path.is_file().then(|| path.to_path_buf());
    }
    for dir in include_path.split(':').filter(|d| !d.is_empty()) {
        let candidate = Path::new(dir).join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let beside = Path::new(current).parent()?.join(name);
    beside.is_file().then_some(beside)
}
