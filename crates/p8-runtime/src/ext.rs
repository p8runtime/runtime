//! How builtins plug into p8: an [`Extension`] registers its functions and
//! constants into a [`Registry`] once per process (the Platform's), and a
//! builtin reaches the running request only through [`Ctx`].
//!
//! The registry is immutable after startup and `Send + Sync`, so every
//! isolate on every thread shares one.

use crate::string::PhpStr;
use crate::value::Value;
use std::collections::HashMap;
use std::rc::Rc;

/// Something that ends the normal flow: a thrown error, or exit().
#[derive(Debug, Clone)]
pub enum Throw {
    Error { class: &'static str, message: String, file: Rc<str>, line: u32 },
    Exit(i32),
}

/// What a builtin can see and do of the request running it.
pub trait Ctx {
    /// Write to the output (through any output buffers).
    fn echo(&mut self, bytes: &[u8]);
    /// ob_start(): start an output buffer.
    fn ob_start(&mut self);
    /// ob_end_clean(): drop the innermost buffer; false if there is none.
    fn ob_end_clean(&mut self) -> bool;
    /// Report a warning at the current line.
    fn warn(&mut self, message: &str);
    /// The constant `name` (as written), defined by the script or an extension.
    fn constant(&self, name: &[u8]) -> Option<Value>;
    /// define(): false when `name` is already defined.
    fn define(&mut self, name: &[u8], value: Value) -> bool;
    /// Whether a function `name` exists (any case, optional leading `\`).
    fn function_exists(&self, name: &[u8]) -> bool;
    /// The running user function's arguments past its parameters.
    fn extra_args(&self) -> &[Value];
    /// A Throw with PHP's error `class` at the current line.
    fn throw(&self, class: &'static str, message: String) -> Throw;
}

/// A builtin function: arguments arrive by value (moved out of the caller's
/// registers); a by-reference parameter receives the variable's reference cell.
pub type Run = fn(&mut dyn Ctx, &mut [Value]) -> Result<Value, Throw>;

pub struct Builtin {
    /// Lowercase.
    pub name: &'static str,
    pub run: Run,
    /// Parameters passed by reference.
    pub by_ref: &'static [usize],
}

/// A constant's value as an extension declares it.
#[derive(Debug, Clone, Copy)]
pub enum Constant {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(&'static [u8]),
}

impl Constant {
    pub fn to_value(self) -> Value {
        match self {
            Constant::Null => Value::Null,
            Constant::Bool(b) => Value::Bool(b),
            Constant::Int(i) => Value::Int(i),
            Constant::Float(f) => Value::Float(f),
            Constant::Str(s) => Value::Str(PhpStr::from(s)),
        }
    }
}

/// A set of builtins: `register` adds them to a registry at startup.
pub struct Extension {
    pub name: &'static str,
    pub register: fn(&mut Registry),
}

/// Every builtin function and constant of the process's extensions.
#[derive(Default)]
pub struct Registry {
    functions: Vec<&'static Builtin>,
    index: HashMap<&'static str, usize>,
    constants: HashMap<&'static [u8], Constant>,
}

impl Registry {
    pub fn new(extensions: &[Extension]) -> Registry {
        let mut registry = Registry::default();
        for extension in extensions {
            (extension.register)(&mut registry);
        }
        registry
    }

    pub fn add_functions(&mut self, functions: &'static [Builtin]) {
        for f in functions {
            let i = self.functions.len();
            self.functions.push(f);
            self.index.insert(f.name, i);
        }
    }

    pub fn add_constant(&mut self, name: &'static [u8], value: Constant) {
        self.constants.insert(name, value);
    }

    /// The builtin named `name` (lowercase).
    pub fn lookup(&self, name: &[u8]) -> Option<usize> {
        let name = std::str::from_utf8(name).ok()?;
        self.index.get(name).copied()
    }

    pub fn function(&self, i: usize) -> &'static Builtin {
        self.functions[i]
    }

    /// Whether parameter `n` of builtin `i` is passed by reference.
    pub fn by_ref(&self, i: usize, n: usize) -> bool {
        self.functions[i].by_ref.contains(&n)
    }

    pub fn constant(&self, name: &[u8]) -> Option<Constant> {
        self.constants.get(name).copied()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn registry_is_shared_across_threads() {
        fn shared<T: Send + Sync>() {}
        shared::<super::Registry>();
    }
}
