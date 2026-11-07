//! Declared types of user code (parameters, return values, properties):
//! the check PHP makes when a value is passed, returned or stored, with
//! the weak-mode conversions (or only int to float under strict_types).
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_execute.c - zend_check_type_slow,
//! zend_verify_scalar_type_hint, zend_verify_weak_scalar_type_hint

use crate::args::{Accept, accepts, convert};
use crate::frozen::Text;
use crate::object::Object;
use crate::value::{Kind, Value};
pub use p8_stubs::{T_ARRAY, T_BOOL, T_CALLABLE, T_FALSE, T_FLOAT, T_INT, T_MIXED, T_NEVER, T_NULL, T_OBJECT, T_STATIC, T_STRING, T_TRUE, T_VOID};

/// A declared type: p8_stubs' T_* bits, and class names (resolved, but
/// `self`, `parent` as written; `static` is T_STATIC).
#[derive(Debug, Clone, PartialEq)]
pub struct TypeDecl {
    pub mask: u32,
    pub classes: Vec<Text>,
    /// `classes` lowercased, as checks compare them (zend_compile_typename
    /// lowercases the names in a function's arg_info once, at compile time).
    pub lower: Vec<Text>,
    /// The class names as written (`self` stays `self`), for messages.
    pub written: Vec<Text>,
    /// `A&B`: the classes all apply (else any one does).
    pub intersection: bool,
}

/// What a check found.
pub enum Verdict {
    /// Accepted; the value to use when it was converted.
    Accepted(Option<Value>),
    /// Accepted with this deprecation (a fractional float to int).
    Deprecated(String, Option<Value>),
    Refused,
}

impl TypeDecl {
    pub fn new(mask: u32, classes: Vec<Text>, written: Vec<Text>, intersection: bool) -> TypeDecl {
        let lower = classes.iter().map(|c| Text::from(c.as_bytes().to_ascii_lowercase().as_slice())).collect();
        TypeDecl { mask, classes, lower, written, intersection }
    }

    /// As PHP prints it for a property (`self` as written).
    pub fn shown(&self) -> String {
        self.render(&self.written)
    }

    /// As PHP prints it for an argument or return value (`self` resolved).
    pub fn shown_resolved(&self) -> String {
        self.render(&self.classes)
    }

    fn render(&self, classes: &[Text]) -> String {
        let names: Vec<String> = classes.iter().map(|c| String::from_utf8_lossy(c.as_bytes()).into_owned()).collect();
        if self.intersection {
            let joined = names.join("&");
            return p8_stubs::type_to_string(self.mask, &[joined.as_str()]);
        }
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        p8_stubs::type_to_string(self.mask, &refs)
    }

    /// Whether `v` is plainly of this type, needing no conversion: its
    /// kind's bit is set, or an object is an instance of the named
    /// class(es) (ZEND_TYPE_CONTAINS_CODE, then instanceof_function).
    /// False only means the full check (`verify`) decides.
    #[inline]
    pub fn quick(&self, v: &Value) -> bool {
        let bit = match v.kind() {
            Kind::Null => T_NULL,
            Kind::Bool => T_BOOL | if v.as_bool().expect("a bool") { T_TRUE } else { T_FALSE },
            Kind::Int => T_INT,
            Kind::Float => T_FLOAT,
            Kind::Str => T_STRING,
            Kind::Array => T_ARRAY,
            // A named class or interface the object's class is or extends
            // (`self`/`parent` never match a class name here: `verify`
            // resolves them), or a Closure where a callable is wanted.
            Kind::Object => {
                let o = v.as_object().expect("an object");
                return self.mask & (T_OBJECT | T_MIXED) != 0
                    || (self.mask & T_CALLABLE != 0 && o.class.lower.as_slice() == b"closure")
                    || if self.intersection {
                        !self.lower.is_empty() && self.lower.iter().all(|c| o.class.is_a(c.as_bytes()))
                    } else {
                        self.lower.iter().any(|c| o.class.is_a(c.as_bytes()))
                    };
            }
            Kind::Undef | Kind::Ref => return false,
        };
        self.mask & (bit | T_MIXED) != 0
    }

    /// Check `v`. `is_a(o, class)`: whether object `o` is an instance of
    /// a class this type names (the caller resolves self/parent/static).
    /// `callable`: whether `v` can be called from the checking code (the
    /// caller asks the engine, zend_is_callable, when the type takes a
    /// callable; a Closure is one already, by `quick`).
    pub fn verify(&self, v: &Value, strict: bool, is_a: &dyn Fn(&Object, &[u8]) -> bool, static_ok: &dyn Fn(&Object) -> bool, callable: bool) -> Verdict {
        let v = v.deref();
        if self.mask & T_MIXED != 0 {
            return Verdict::Accepted(None);
        }
        let exact = match v.kind() {
            Kind::Null | Kind::Undef => self.mask & T_NULL != 0,
            Kind::Bool => self.mask & T_BOOL != 0 || self.mask & if v.as_bool().expect("a bool") { T_TRUE } else { T_FALSE } != 0,
            Kind::Int => self.mask & T_INT != 0,
            Kind::Float => self.mask & T_FLOAT != 0,
            Kind::Str => self.mask & T_STRING != 0,
            Kind::Array => self.mask & T_ARRAY != 0,
            Kind::Object => {
                let o = v.as_object().expect("an object");
                self.mask & T_OBJECT != 0
                    || self.mask & T_STATIC != 0 && static_ok(o)
                    || !self.classes.is_empty() && {
                        let hit = |c: &Text| is_a(o, c.as_bytes());
                        if self.intersection { self.lower.iter().all(hit) } else { self.lower.iter().any(hit) }
                    }
            }
            Kind::Ref => false,
        };
        if exact {
            return Verdict::Accepted(None);
        }
        // A callable: one the engine found it can call.
        if self.mask & T_CALLABLE != 0 && callable && matches!(v.kind(), Kind::Str | Kind::Array | Kind::Object) {
            return Verdict::Accepted(None);
        }
        let scalars = T_BOOL | T_TRUE | T_FALSE | T_INT | T_FLOAT | T_STRING;
        if strict {
            return match v.as_int() {
                Some(i) if self.mask & T_FLOAT != 0 => Verdict::Accepted(Some(Value::float(i as f64))),
                _ => Verdict::Refused,
            };
        }
        // Weak mode: scalars (not null) convert to a scalar type.
        if !matches!(v.kind(), Kind::Bool | Kind::Int | Kind::Float | Kind::Str) || self.mask & scalars == 0 {
            // A Stringable object to a string.
            return Verdict::Refused;
        }
        // A fractional float to int|string (no float): int, with the
        // precision deprecation (as PHP 8.4 does for user types).
        if let Some(f) = v.as_float()
            && self.mask & T_INT != 0
            && self.mask & T_STRING != 0
            && self.mask & T_FLOAT == 0
            && f.fract() != 0.0
            && (-9.223_372_036_854_776e18..9.223_372_036_854_776e18).contains(&f)
        {
            let shown = crate::number::float_to_string(f, crate::number::Precision::SERIALIZE, 'E');
            return Verdict::Deprecated(format!("Implicit conversion from float {shown} to int loses precision"), Some(Value::int(f as i64)));
        }
        let param = p8_stubs::Param { name: "", mask: self.mask & (scalars | T_NULL), classes: &[], by_ref: false, variadic: false, default: None };
        match accepts(&param, &v) {
            Accept::Yes => Verdict::Accepted(convert(param.mask, &v)),
            Accept::Deprecated(m) => Verdict::Deprecated(m, convert(param.mask, &v)),
            Accept::NullDeprecated | Accept::No => Verdict::Refused,
        }
    }

    /// `void`
    pub fn is_void(&self) -> bool {
        self.mask == T_VOID
    }
}
