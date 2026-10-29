//! `include` and `require`: finding the file (include_path, then the
//! including file's directory), compiling it through the embedder's
//! loader, and declaring what it declares.
//!
//! Reference: $PHP_SRC_PATH/main/fopen_wrappers.c (php_resolve_path),
//! $PHP_SRC_PATH/Zend/zend_execute.c (zend_include_or_eval)

use crate::Context;
use p8_bytecode::{IncludeKind, Program};
use p8_runtime::ext::{Ctx, Throw};
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
        let include_path = match Ctx::ini_get(self, b"include_path") {
            Some(Value::Str(s)) => String::from_utf8_lossy(s.as_bytes()).into_owned(),
            _ => ".".to_string(),
        };
        let found = if name.is_empty() { None } else { resolve(&shown, &include_path, current) };
        let source = found.as_ref().and_then(|p| std::fs::read(p).ok().map(|s| (p.clone(), s)));
        let Some((path, source)) = source else {
            let verb = kind.name();
            if name.is_empty() {
                self.warn(format!("{verb}(): Filename cannot be empty"));
            } else {
                self.warn(format!("{verb}({shown}): Failed to open stream: No such file or directory"));
            }
            if kind.required() {
                return Err(self.throw("Error", format!("Failed opening required '{shown}' (include_path='{include_path}')")));
            }
            self.warn(format!("{verb}(): Failed opening '{shown}' for inclusion (include_path='{include_path}')"));
            return Ok(Some(Err(())));
        };
        let real = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if kind.once() && self.included.contains(&real) {
            return Ok(None);
        }
        // The file is known by its resolved path (__FILE__, __DIR__), as PHP
        // opens it, not as the include named it.
        let file = real.to_string_lossy().into_owned();
        self.included_order.push(file.clone());
        self.included.insert(real);
        let loader = match &self.loader {
            Some(l) => Rc::clone(l),
            None => return Err(self.throw("Error", format!("Failed opening required '{shown}' (no compiler)"))),
        };
        let program = match loader.load(&source, &file) {
            Ok(p) => p,
            Err(e) if e.parse => return Err(Throw::Parse { message: e.message, file: Rc::from(file.as_str()), line: e.line }),
            Err(e) => return Err(Throw::Fatal { message: e.message, file: Rc::from(file.as_str()), line: e.line }),
        };
        for f in &program.functions {
            if let Err(message) = self.declare(Arc::clone(f)) {
                return Err(Throw::Fatal { message, file: Rc::from(&*f.file), line: f.line });
            }
        }
        self.early_bind(&program.classes, Some(Rc::from(&*program.main.file)))?;
        Ok(Some(Ok(program)))
    }
}

impl Context<'_> {
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
