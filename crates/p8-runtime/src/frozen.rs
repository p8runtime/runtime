//! Values compiled code holds: immutable and `Send + Sync`, so one compiled
//! script is shared by every isolate on every thread. A context turns them
//! into [`Value`]s once, when it first runs the code that holds them.

use crate::array::{Key, PhpArray};
use crate::string::PhpStr;
use crate::value::Value;
use std::fmt;
use std::sync::Arc;

/// An immutable byte string (a name, a literal): a range of a shared
/// buffer. Usually the buffer is the text's own; a compiled file read back
/// from a cache keeps all its texts in one buffer (one allocation for
/// thousands of names and literals).
#[derive(Clone)]
pub struct Text {
    buf: Arc<[u8]>,
    start: u32,
    len: u32,
}

impl Text {
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[self.start as usize..self.start as usize + self.len as usize]
    }

    /// The text at `start..start + len` of `buf`, sharing it; None if that
    /// is outside it.
    pub fn in_buffer(buf: &Arc<[u8]>, start: usize, len: usize) -> Option<Text> {
        let end = start.checked_add(len)?;
        (end <= buf.len() && end <= u32::MAX as usize).then(|| Text { buf: Arc::clone(buf), start: start as u32, len: len as u32 })
    }

    fn own(buf: Arc<[u8]>) -> Text {
        let len = u32::try_from(buf.len()).expect("a text under 4 GiB");
        Text { buf, start: 0, len }
    }
}

impl PartialEq for Text {
    fn eq(&self, other: &Text) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for Text {}

impl std::hash::Hash for Text {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.as_bytes().hash(state);
    }
}

impl From<&[u8]> for Text {
    fn from(bytes: &[u8]) -> Self {
        Text::own(Arc::from(bytes))
    }
}

impl From<&str> for Text {
    fn from(s: &str) -> Self {
        Text::own(Arc::from(s.as_bytes()))
    }
}

impl From<Vec<u8>> for Text {
    fn from(bytes: Vec<u8>) -> Self {
        Text::own(Arc::from(bytes))
    }
}

impl fmt::Debug for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", String::from_utf8_lossy(self.as_bytes()))
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
        match self {
            Frozen::Null => Value::Null,
            Frozen::Bool(b) => Value::Bool(*b),
            Frozen::Int(i) => Value::Int(*i),
            Frozen::Float(f) => Value::Float(*f),
            Frozen::Str(s) => Value::Str(PhpStr::from(s.as_bytes())),
            Frozen::Array(entries) => {
                let mut array = PhpArray::new();
                for (k, v) in entries.iter() {
                    match k.as_ref().map(|k| Key::from_value(&k.to_value())) {
                        Some(Ok((key, _))) => array.insert(key, v.to_value()),
                        _ => {
                            let _ = array.push(v.to_value());
                        }
                    }
                }
                Value::Array(std::rc::Rc::new(array))
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
    fn texts_in_one_buffer_are_their_own_bytes() {
        use super::Text;
        use std::sync::Arc;
        let buf: Arc<[u8]> = Arc::from(&b"__constructgetName"[..]);
        let ctor = Text::in_buffer(&buf, 0, 11).expect("inside");
        let get = Text::in_buffer(&buf, 11, 7).expect("inside");
        assert_eq!((ctor.as_bytes(), get.as_bytes()), (&b"__construct"[..], &b"getName"[..]));
        // Equal and hashed as their bytes, wherever they live.
        assert_eq!(get, Text::from("getName"));
        let hash = |t: &Text| {
            use std::hash::{BuildHasher, BuildHasherDefault};
            BuildHasherDefault::<std::collections::hash_map::DefaultHasher>::default().hash_one(t)
        };
        assert_eq!(hash(&get), hash(&Text::from("getName")));
        assert!(Text::in_buffer(&buf, 11, 8).is_none() && Text::in_buffer(&buf, usize::MAX, 2).is_none());
        // One allocation for both.
        assert_eq!(Arc::strong_count(&buf), 3);
    }
}
