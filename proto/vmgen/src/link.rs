//! From PHP source to a runnable program: p8's parser and compiler, the
//! constants thawed once into a pool (string literals as program-memory
//! strings, as the engine does), the file's classes linked into
//! `p8_runtime::object::Class` (plain classes only: no parent, interfaces
//! or traits), and every function translated.

use crate::translate::{Shape, World, shape, translate};
use crate::vm::Func;
use p8_bytecode::{ClassDef, Function, Program};
use p8_runtime::ext::Registry;
use p8_runtime::frozen::Frozen;
use p8_runtime::object::{Class, Constants, Method, Methods, Property, Slots};
use p8_runtime::string::{IStr, PhpStr};
use p8_runtime::symbol::{SymbolCache, Symbols};
use p8_runtime::value::Value;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

/// Compile a file with p8's real compiler (no extensions registered:
/// nothing in the subset calls a builtin).
pub fn compile(source: &[u8], file: &str) -> Result<Arc<Program>, String> {
    let registry = Registry::new(&[]);
    let arena = bumpalo::Bump::new();
    let lexer = p8_syntax::lexer::Lexer::new(source);
    let mut parser = p8_syntax::parser::Parser::new(lexer, &arena);
    let parsed = parser.parse_program();
    if let Some(e) = parsed.errors.first() {
        return Err(format!("parse error: {}", e.message));
    }
    let compiler = p8_compiler::Compiler::new(source, file, &registry);
    compiler.compile(parsed.statements).map(Arc::new).map_err(|e| format!("compile error: {} (line {})", e.message, e.line))
}

/// Every function of a program in one order: the file's functions, then
/// each class's methods (in class order).
pub fn functions(program: &Program) -> Vec<&Function> {
    let mut v: Vec<&Function> = program.functions.iter().map(|f| &**f).collect();
    for c in &program.classes {
        v.extend(c.methods.iter().map(|m| &*m.func));
    }
    v
}

/// The constants of every function, thawed once (string literals interned
/// as program-memory strings, as `Isolate::thaw` does), and a null for
/// unused literal operands.
pub struct Pool {
    pub values: Vec<Value>,
    /// Per function (in `functions` order): where its constants start.
    pub starts: Vec<usize>,
    pub null: Value,
}

impl Pool {
    pub fn new(program: &Program) -> Pool {
        let mut values = Vec::new();
        let mut starts = Vec::new();
        for f in functions(program) {
            starts.push(values.len());
            values.extend(f.consts.iter().map(thaw));
        }
        Pool { values, starts, null: Value::NULL }
    }

    pub fn consts_of(&self, func: usize, n: usize) -> &[Value] {
        &self.values[self.starts[func]..self.starts[func] + n]
    }
}

fn thaw(f: &Frozen) -> Value {
    match f {
        Frozen::Str(t) => IStr::intern(t.as_bytes()).map_or_else(|| f.to_value(), Value::istr),
        _ => f.to_value(),
    }
}

/// The program as the machine runs it.
pub struct Linked<'p> {
    pub funcs: Vec<Func<'p>>,
    pub classes: Vec<Rc<Class>>,
}

/// A plain class linked: its own properties and methods, nothing
/// inherited (a parent, interfaces, traits, enums refuse).
fn link_class(def: &ClassDef, method_funcs: &[u32], symbols: &Rc<SymbolCache>) -> Result<Rc<Class>, String> {
    let name = String::from_utf8_lossy(def.name.as_bytes()).into_owned();
    if def.parent.is_some() || !def.interfaces.is_empty() || !def.traits.is_empty() || !def.cases.is_empty() || !def.constants.is_empty() {
        return Err(format!("class {name}: inheritance, interfaces, traits, enums and class constants are not in the prototype"));
    }
    let lower = def.name.as_bytes().to_ascii_lowercase();
    let class_name = PhpStr::from(def.name.as_bytes());
    let declaring: Rc<[u8]> = Rc::from(lower.as_slice());
    let mut properties = Vec::new();
    let mut slots = Slots::default();
    for p in &def.properties {
        if p.is_static || p.code.is_some() || p.hooks != 0 {
            return Err(format!("class {name}: static, computed or hooked properties are not in the prototype"));
        }
        slots.insert(p.name.as_bytes().to_vec(), vec![properties.len()]);
        properties.push(Property {
            name: PhpStr::from(p.name.as_bytes()),
            default: p.default.as_ref().map_or(Value::UNDEF, thaw),
            visibility: p.visibility,
            set_visibility: p.set_visibility,
            declaring: lower.clone(),
            declaring_name: class_name.clone(),
            readonly: p.readonly,
            is_final: p.is_final,
            ty: p.ty.as_ref().map(|t| PhpStr::from(t.as_bytes())),
            ty_decl: p.ty_decl.clone().map(Rc::new),
            init: None,
            hooks: p.hooks,
        });
    }
    let mut methods = Methods::new(symbols);
    let mut has_destructor = false;
    for (m, &func) in def.methods.iter().zip(method_funcs) {
        let lower_m = m.func.name.as_bytes().to_ascii_lowercase();
        has_destructor |= lower_m == b"__destruct";
        if lower_m.starts_with(b"__") && lower_m != b"__construct" {
            return Err(format!("class {name}: magic method {} is not in the prototype", String::from_utf8_lossy(&lower_m)));
        }
        methods.insert(
            lower_m,
            Rc::new(Method {
                name: PhpStr::from(m.func.name.as_bytes()),
                func,
                native: None,
                visibility: m.visibility,
                is_static: m.is_static,
                is_abstract: m.is_abstract,
                is_final: m.is_final,
                declaring: Rc::clone(&declaring),
                declaring_name: class_name.clone(),
                deprecation: None,
            }),
        );
    }
    if has_destructor {
        return Err(format!("class {name}: destructors are not in the prototype"));
    }
    let property_order = def.properties.iter().map(|p| PhpStr::from(p.name.as_bytes())).collect();
    Ok(Rc::new(Class {
        name: class_name,
        lower,
        flags: def.flags,
        parent: None,
        interfaces: Vec::new(),
        properties,
        slots,
        methods,
        constants: Constants::default(),
        statics: Default::default(),
        property_order,
        debug_info: None,
        get_properties: None,
        cast_bool: None,
        cast_string: None,
        cast_int: None,
        clone_data: None,
        forward_calls: None,
        make_data: None,
        native_dtor: None,
        enum_cases: Vec::new(),
        enum_backing: None,
        static_inits: Vec::new(),
    }))
}

/// Link and translate the whole program.
pub fn link<'p>(program: &'p Program, pool: &'p Pool) -> Result<Linked<'p>, String> {
    let funcs = functions(program);
    // Function names → indexes (methods are reached through their class).
    let mut by_name: HashMap<Vec<u8>, u32> = HashMap::new();
    for (i, f) in program.functions.iter().enumerate() {
        by_name.insert(f.name.as_bytes().to_ascii_lowercase(), i as u32);
    }
    let shapes: Vec<Shape> = funcs.iter().map(|f| shape(f)).collect();
    // Classes, their methods' function indexes following the file's functions.
    let symbols = Rc::new(SymbolCache::new(&Arc::new(Symbols::default())));
    let mut classes = Vec::new();
    let mut class_index: HashMap<Vec<u8>, (u32, Option<u32>)> = HashMap::new();
    let mut next = program.functions.len() as u32;
    for def in &program.classes {
        let method_funcs: Vec<u32> = (0..def.methods.len() as u32).map(|k| next + k).collect();
        next += def.methods.len() as u32;
        let ctor = def.methods.iter().zip(&method_funcs).find(|(m, _)| m.func.name.as_bytes().eq_ignore_ascii_case(b"__construct")).map(|(_, &f)| f);
        let class = link_class(def, &method_funcs, &symbols)?;
        class_index.insert(class.lower.clone(), (classes.len() as u32, ctor));
        classes.push(class);
    }
    let world = World { funcs: &by_name, classes: &class_index, shapes: &shapes };
    let translated: Vec<Func<'p>> = funcs.iter().enumerate().map(|(i, f)| translate(f, pool, pool.consts_of(i, f.consts.len()), &world)).collect();
    Ok(Linked { funcs: translated, classes })
}
