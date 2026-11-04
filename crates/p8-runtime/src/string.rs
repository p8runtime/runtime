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
    bytes: Bytes,
}

/// Up to this many bytes live in the string's own allocation.
const INLINE: usize = 22;

/// A short string's bytes inside the Rc's allocation, as zend_string keeps
/// its bytes after its header: one allocation, not two. Written strings
/// move to a Vec (make_mut), which `.=` then grows in place.
#[derive(Clone)]
enum Bytes {
    Inline { len: u8, buf: [u8; INLINE] },
    Heap(Vec<u8>),
}

impl Bytes {
    #[inline]
    fn as_slice(&self) -> &[u8] {
        match self {
            Bytes::Inline { len, buf } => &buf[..*len as usize],
            Bytes::Heap(v) => v,
        }
    }
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
        PhpStr(Rc::new(Inner { hash: Cell::new(0), bytes: Bytes::Heap(bytes) }))
    }

    /// A copy of `bytes`: in the string's own allocation when short.
    fn copy_of(bytes: &[u8]) -> Self {
        if bytes.len() > INLINE {
            return Self::new(bytes.to_vec());
        }
        let mut buf = [0; INLINE];
        buf[..bytes.len()].copy_from_slice(bytes);
        PhpStr(Rc::new(Inner { hash: Cell::new(0), bytes: Bytes::Inline { len: bytes.len() as u8, buf } }))
    }

    /// `a` followed by `b` (a concatenation of two strings): in the
    /// string's own allocation when short, else one buffer of the exact
    /// size; no intermediate buffer either way.
    pub fn concat(a: &[u8], b: &[u8]) -> Self {
        let len = a.len() + b.len();
        if len <= INLINE {
            let mut buf = [0; INLINE];
            buf[..a.len()].copy_from_slice(a);
            buf[a.len()..len].copy_from_slice(b);
            return PhpStr(Rc::new(Inner { hash: Cell::new(0), bytes: Bytes::Inline { len: len as u8, buf } }));
        }
        let mut v = Vec::with_capacity(len);
        v.extend_from_slice(a);
        v.extend_from_slice(b);
        Self::new(v)
    }

    pub fn empty() -> Self {
        Self::copy_of(b"")
    }

    /// The one-byte string `b` (`$s[$i]`, chr()): one shared string per
    /// byte on each thread, as PHP interns them (ZSTR_CHAR); a write to
    /// one copies it first, as to any shared string.
    pub fn byte(b: u8) -> Self {
        thread_local! {
            static BYTES: [std::cell::OnceCell<PhpStr>; 256] = const { [const { std::cell::OnceCell::new() }; 256] };
        }
        BYTES.with(|bytes| bytes[b as usize].get_or_init(|| Self::copy_of(&[b])).clone())
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.bytes.as_slice()
    }

    pub fn len(&self) -> usize {
        self.as_bytes().len()
    }

    pub fn is_empty(&self) -> bool {
        self.as_bytes().is_empty()
    }

    /// The bytes for writing; copies them first if the string is shared.
    pub fn make_mut(&mut self) -> &mut Vec<u8> {
        let inner = Rc::make_mut(&mut self.0);
        inner.hash.set(0);
        if let Bytes::Inline { len, buf } = &inner.bytes {
            inner.bytes = Bytes::Heap(buf[..*len as usize].to_vec());
        }
        match &mut inner.bytes {
            Bytes::Heap(v) => v,
            Bytes::Inline { .. } => unreachable!("moved to the heap above"),
        }
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
        for &b in self.as_bytes() {
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
        Self::copy_of(bytes)
    }
}

impl From<&str> for PhpStr {
    fn from(s: &str) -> Self {
        Self::copy_of(s.as_bytes())
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
    fn short_strings_are_inline_and_move_to_the_heap_when_written() {
        let short = PhpStr::from("ru_oublock");
        assert!(matches!(short.0.bytes, Bytes::Inline { .. }));
        let long = PhpStr::from("a string longer than the inline buffer");
        assert!(matches!(long.0.bytes, Bytes::Heap(_)));
        let mut s = short.clone();
        s.push_bytes(b"!");
        assert!(matches!(s.0.bytes, Bytes::Heap(_)));
        assert_eq!(s.as_bytes(), b"ru_oublock!");
        assert_eq!(short.as_bytes(), b"ru_oublock");
        assert_eq!(PhpStr::from("x".repeat(INLINE).as_str()).len(), INLINE);
        assert_eq!(PhpStr::empty().as_bytes(), b"");
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
