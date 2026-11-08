//! What an AOT-compiled program would carry as data beside its code: the
//! file's classes linked into `p8_runtime::object::Class`, and the
//! function `t`'s literals thawed once (string literals as program-memory
//! strings, as the engine does). Made here with p8's real parser and
//! compiler, as V1's `link.rs` makes them, so both prototypes run on the
//! same class and literal values. (Startup only: the measurement is the
//! difference of two runs, so none of this is in it.)

use p8_bytecode::{ClassDef, Program};
use p8_runtime::ext::Registry;
use p8_runtime::frozen::Frozen;
use p8_runtime::object::{Class, Constants, Method, Methods, Property, Slots};
use p8_runtime::string::{IStr, PhpStr};
use p8_runtime::symbol::{SymbolCache, Symbols};
use p8_runtime::value::Value;
use std::rc::Rc;
use std::sync::Arc;

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

pub fn thaw(f: &Frozen) -> Value {
    match f {
        Frozen::Str(t) => IStr::intern(t.as_bytes()).map_or_else(|| f.to_value(), Value::istr),
        _ => f.to_value(),
    }
}

/// A plain class linked (no parent, interfaces, traits, destructor): the
/// same as V1's `link_class`. A method's `func` is its index among the
/// file's methods (the AOT code is found by name, `Program::method`).
fn link_class(def: &ClassDef, symbols: &Rc<SymbolCache>) -> Result<Rc<Class>, String> {
    let name = String::from_utf8_lossy(def.name.as_bytes()).into_owned();
    if def.parent.is_some() || !def.interfaces.is_empty() || !def.traits.is_empty() || !def.cases.is_empty() || !def.constants.is_empty() {
        return Err(format!("class {name}: not a plain class"));
    }
    let lower = def.name.as_bytes().to_ascii_lowercase();
    let class_name = PhpStr::from(def.name.as_bytes());
    let declaring: Rc<[u8]> = Rc::from(lower.as_slice());
    let mut properties = Vec::new();
    let mut slots = Slots::default();
    for p in &def.properties {
        if p.is_static || p.code.is_some() || p.hooks != 0 {
            return Err(format!("class {name}: static, computed or hooked property"));
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
    for (k, m) in def.methods.iter().enumerate() {
        let lower_m = m.func.name.as_bytes().to_ascii_lowercase();
        if lower_m.starts_with(b"__") {
            return Err(format!("class {name}: magic method"));
        }
        methods.insert(
            lower_m,
            Rc::new(Method {
                name: PhpStr::from(m.func.name.as_bytes()),
                func: k as u32,
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

/// The program's data: its classes, and `t`'s literals.
pub struct Data {
    pub classes: Vec<Rc<Class>>,
    pub t_consts: Vec<Value>,
}

pub fn load(program: &Program) -> Result<Data, String> {
    let symbols = Rc::new(SymbolCache::new(&Arc::new(Symbols::default())));
    let classes = program.classes.iter().map(|d| link_class(d, &symbols)).collect::<Result<Vec<_>, _>>()?;
    let t = program.functions.iter().find(|f| f.name.as_bytes() == b"t").ok_or("no function t")?;
    Ok(Data { classes, t_consts: t.consts.iter().map(thaw).collect() })
}
