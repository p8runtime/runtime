//! What reflection asks the engine about functions and methods (the
//! Reflection classes are built on these, through Ctx).

use crate::object::{Class, Object};
use crate::string::PhpStr;
use crate::types::TypeDecl;
use crate::value::Value;
use std::rc::Rc;

/// A function reflection looks at.
#[derive(Clone)]
pub enum Callee {
    /// A function by name (user or builtin).
    Function(PhpStr),
    /// A class's method, by name (any case).
    Method(Rc<Class>, PhpStr),
    /// A Closure object.
    Closure(Rc<Object>),
}

/// One parameter.
pub struct ParamInfo {
    pub name: PhpStr,
    pub by_ref: bool,
    pub variadic: bool,
    pub optional: bool,
    /// The default, when it is a literal (user code) or the stub's
    /// default can be read.
    pub default: Option<Value>,
    /// A builtin's default as its stub writes it.
    pub default_text: Option<&'static str>,
    pub ty: Option<TypeDecl>,
    /// A promoted constructor parameter.
    pub promoted: bool,
}

/// A function's declaration.
pub struct FnInfo {
    /// As declared (`{closure}` for a closure).
    pub name: PhpStr,
    /// A method's declaring class.
    pub class: Option<Rc<Class>>,
    pub params: Vec<ParamInfo>,
    pub ret: Option<TypeDecl>,
    pub returns_ref: bool,
    /// Written in PHP (else a builtin).
    pub user: bool,
    pub file: Option<PhpStr>,
    pub line: u32,
    pub is_static: bool,
    pub is_closure: bool,
    pub is_generator: bool,
    pub closure_this: Option<Rc<Object>>,
    pub closure_scope: Option<Rc<Class>>,
}
