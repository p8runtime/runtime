//! PHP strings: immutable-by-default byte strings, shared until written
//! (copy-on-write through `Rc::make_mut`), with the hash computed once and
//! cached for use as array keys and names.

use std::cell::Cell;
use std::fmt;
use std::rc::Rc;

#[derive(Clone)]
struct Inner {
    /// 0 until computed (a real hash of 0 is stored as 1).
    hash: Cell<u64>,
    bytes: Vec<u8>,
}

/// A PHP string value.
#[derive(Clone)]
pub struct PhpStr(Rc<Inner>);

impl Default for PhpStr {
    fn default() -> Self {
        PhpStr::empty()
    }
}

impl PhpStr {
    pub fn new(bytes: Vec<u8>) -> Self {
        PhpStr(Rc::new(Inner { hash: Cell::new(0), bytes }))
    }

    pub fn empty() -> Self {
        Self::new(Vec::new())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0.bytes
    }

    pub fn len(&self) -> usize {
        self.0.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.bytes.is_empty()
    }

    /// The bytes for writing; copies them first if the string is shared.
    pub fn make_mut(&mut self) -> &mut Vec<u8> {
        let inner = Rc::make_mut(&mut self.0);
        inner.hash.set(0);
        &mut inner.bytes
    }

    /// Append, as `.=` does (in place when this is the only reference).
    pub fn push_bytes(&mut self, bytes: &[u8]) {
        self.make_mut().extend_from_slice(bytes);
    }

    /// The string's hash (FNV-1a), computed on first use.
    pub fn hash(&self) -> u64 {
        let cached = self.0.hash.get();
        if cached != 0 {
            return cached;
        }
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &b in &self.0.bytes {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        let h = h.max(1);
        self.0.hash.set(h);
        h
    }

    /// Whether two values share one string (no copy was made).
    pub fn ptr_eq(a: &Self, b: &Self) -> bool {
        Rc::ptr_eq(&a.0, &b.0)
    }
}

impl PartialEq for PhpStr {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0) || (self.hash() == other.hash() && self.as_bytes() == other.as_bytes())
    }
}

impl Eq for PhpStr {}

impl From<&[u8]> for PhpStr {
    fn from(bytes: &[u8]) -> Self {
        Self::new(bytes.to_vec())
    }
}

impl From<&str> for PhpStr {
    fn from(s: &str) -> Self {
        Self::new(s.as_bytes().to_vec())
    }
}

impl From<String> for PhpStr {
    fn from(s: String) -> Self {
        Self::new(s.into_bytes())
    }
}

impl fmt::Debug for PhpStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", String::from_utf8_lossy(self.as_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_on_write_and_rehashes() {
        let a = PhpStr::from("ab");
        let mut b = a.clone();
        assert!(PhpStr::ptr_eq(&a, &b));
        let h = a.hash();
        b.push_bytes(b"c");
        assert!(!PhpStr::ptr_eq(&a, &b));
        assert_eq!(a.as_bytes(), b"ab");
        assert_eq!(b.as_bytes(), b"abc");
        assert_eq!(a.hash(), h);
        assert_ne!(b.hash(), h);
        assert_eq!(b, PhpStr::from("abc"));
    }

    #[test]
    fn appends_in_place_when_unshared() {
        let mut s = PhpStr::from("a");
        s.push_bytes(b"b");
        let before = s.as_bytes().as_ptr();
        s.make_mut().reserve(0);
        assert_eq!(s.as_bytes().as_ptr(), before);
    }
}
