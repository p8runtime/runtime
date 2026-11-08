//! Values compiled code holds: immutable and `Send + Sync`, so one compiled
//! script is shared by every isolate on every thread. A context turns them
//! into [`Value`]s once, when it first runs the code that holds them.

use crate::array::{Key, PhpArray};
use crate::string::PhpStr;
use crate::value::Value;
use std::fmt;
use std::sync::Arc;

/// An immutable byte string (a name, a literal).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Text(Arc<[u8]>);

impl Text {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl From<&[u8]> for Text {
    fn from(bytes: &[u8]) -> Self {
        Text(Arc::from(bytes))
    }
}

impl From<&str> for Text {
    fn from(s: &str) -> Self {
        Text(Arc::from(s.as_bytes()))
    }
}

impl From<Vec<u8>> for Text {
    fn from(bytes: Vec<u8>) -> Self {
        Text(Arc::from(bytes))
    }
}

impl fmt::Debug for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", String::from_utf8_lossy(&self.0))
    }
}

/// A constant in compiled code.
#[derive(Debug, Clone, PartialEq)]
pub enum Frozen {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Text),
    /// A literal array: its entries' keys (None: the next int key) and values.
    Array(Arc<[(Option<Frozen>, Frozen)]>),
}

impl Frozen {
    pub fn to_value(&self) -> Value {
        self.value(false)
    }

    /// The value as a program's constant: its strings, an array's keys
    /// and values included, are program-memory strings
    /// ([`IStr::intern`](crate::string::IStr::intern); ordinary ones once
    /// the pool is full), so a literal probe finds a constant array's key
    /// by pointer.
    pub fn to_program_value(&self) -> Value {
        self.value(true)
    }

    fn value(&self, program: bool) -> Value {
        match self {
            Frozen::Null => Value::NULL,
            Frozen::Bool(b) => Value::bool(*b),
            Frozen::Int(i) => Value::int(*i),
            Frozen::Float(f) => Value::float(*f),
            Frozen::Str(s) => match program.then(|| crate::string::IStr::intern(s.as_bytes())).flatten() {
                Some(i) => Value::istr(i),
                None => Value::str(PhpStr::from(s.as_bytes())),
            },
            Frozen::Array(entries) => {
                let mut array = PhpArray::new();
                for (k, v) in entries.iter() {
                    match k.as_ref().map(|k| Key::from_value(&k.value(program))) {
                        Some(Ok((key, _))) => array.insert(key, v.value(program)),
                        _ => {
                            let _ = array.push(v.value(program));
                        }
                    }
                }
                Value::array(array)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn compiled_constants_are_shared_across_threads() {
        fn shared<T: Send + Sync>() {}
        shared::<super::Frozen>();
        shared::<super::Text>();
    }

    #[test]
    fn texts_stay_small() {
        // Compiled programs and call sites hold texts everywhere: a bigger
        // one costs every run (#251 measured +0.5% at 24 bytes).
        assert_eq!(std::mem::size_of::<super::Text>(), 16);
        assert_eq!(std::mem::size_of::<Option<super::Text>>(), 16);
    }
}
