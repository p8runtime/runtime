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
    /// A default that is a constant, by name (isDefaultValueConstant).
    pub default_const: Option<PhpStr>,
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
    /// A builtin method's tentative return type (getTentativeReturnType):
    /// stated, but not yet enforced on overrides, so getReturnType() is null.
    pub tentative_ret: Option<TypeDecl>,
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
    /// A closure's called scope (`static`): getClosureCalledClass.
    pub closure_called: Option<Rc<Class>>,
}

/// A declaration whose doc comment, attributes and lines reflection reads.
#[derive(Clone)]
pub enum Subject {
    Class(Rc<Class>),
    Function(Callee),
    /// A class's property (its name).
    Property(Rc<Class>, PhpStr),
    /// A class constant or enum case.
    Constant(Rc<Class>, PhpStr),
    /// A function's parameter, by position.
    Param(Callee, usize),
}

/// What the engine knows of a declaration besides its code.
#[derive(Default)]
pub struct Meta {
    /// The `/** ... */` directly before it.
    pub doc: Option<PhpStr>,
    /// Its attributes' class names, resolved, in order.
    pub attrs: Vec<PhpStr>,
    /// A class's file, first and last line (a function's file and first
    /// line are in its FnInfo).
    pub file: Option<PhpStr>,
    pub line: u32,
    pub end_line: u32,
    /// A class's directly used traits, as the class names them.
    pub traits: Vec<PhpStr>,
}
