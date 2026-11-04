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

thread_local! {
    /// The empty string and every one-byte string, made once per thread
    /// and shared (PHP's ZSTR_EMPTY and ZSTR_CHAR): `$s[$i]`, `chr()`,
    /// `''` allocate nothing.
    static SMALL: Vec<PhpStr> = std::iter::once(&[][..])
        .chain((0..=255u8).map(|b| std::slice::from_ref(&BYTES[b as usize])))
        .map(PhpStr::inline)
        .collect();
}

/// Every byte value, for SMALL's one-byte strings.
const BYTES: [u8; 256] = {
    let mut all = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        all[i] = i as u8;
        i += 1;
    }
    all
};

impl PhpStr {
    /// A string of these bytes, taken as they are (no copy).
    pub fn new(bytes: Vec<u8>) -> Self {
        if let Some(s) = Self::small(&bytes) {
            return s;
        }
        PhpStr(Rc::new(Inner { hash: Cell::new(0), bytes: Bytes::Heap(bytes) }))
    }

    /// The shared string for zero or one bytes (None for longer ones, and
    /// while the thread is being torn down).
    #[inline]
    fn small(bytes: &[u8]) -> Option<PhpStr> {
        let i = match bytes {
            [] => 0,
            [b] => 1 + *b as usize,
            _ => return None,
        };
        SMALL.try_with(|all| all[i].clone()).ok()
    }

    /// A string in its own allocation (`bytes` at most INLINE long).
    fn inline(bytes: &[u8]) -> PhpStr {
        let mut buf = [0; INLINE];
        buf[..bytes.len()].copy_from_slice(bytes);
        PhpStr(Rc::new(Inner { hash: Cell::new(0), bytes: Bytes::Inline { len: bytes.len() as u8, buf } }))
    }

    /// A copy of `bytes`: in the string's own allocation when short, and
    /// no allocation at all for zero or one bytes.
    fn copy_of(bytes: &[u8]) -> Self {
        if bytes.len() > INLINE {
            return PhpStr(Rc::new(Inner { hash: Cell::new(0), bytes: Bytes::Heap(bytes.to_vec()) }));
        }
        Self::small(bytes).unwrap_or_else(|| Self::inline(bytes))
    }

    /// `a` then `b`, in one allocation when short (no buffer to build it in).
    pub fn concat(a: &[u8], b: &[u8]) -> Self {
        let len = a.len() + b.len();
        if len <= INLINE {
            let mut buf = [0; INLINE];
            buf[..a.len()].copy_from_slice(a);
            buf[a.len()..len].copy_from_slice(b);
            return Self::copy_of(&buf[..len]);
        }
        let mut v = Vec::with_capacity(len);
        v.extend_from_slice(a);
        v.extend_from_slice(b);
        Self::new(v)
    }

    /// An int's decimal digits, in one allocation (no buffer to build them in).
    pub fn from_int(i: i64) -> Self {
        let mut buf = [0u8; 20];
        let mut at = buf.len();
        let mut n = i.unsigned_abs();
        loop {
            at -= 1;
            buf[at] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        if i < 0 {
            // i64::MIN has 19 digits: room for the sign.
            at -= 1;
            buf[at] = b'-';
        }
        Self::copy_of(&buf[at..])
    }

    pub fn empty() -> Self {
        Self::copy_of(b"")
    }

    /// The one-byte string `b` (`$s[$i]`, chr()): the thread's shared one
    /// (SMALL), no allocation.
    pub fn byte(b: u8) -> Self {
        Self::copy_of(&[b])
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
        self.writable(0)
    }

    /// The bytes for writing, with room for `more` (zend_string_extend):
    /// in place when this is the only reference and its buffer is on the
    /// heap; else one new buffer, with room to grow, holding a copy.
    fn writable(&mut self, more: usize) -> &mut Vec<u8> {
        let heap = matches!(self.0.bytes, Bytes::Heap(_));
        match Rc::get_mut(&mut self.0) {
            Some(_) if heap => {}
            Some(inner) => {
                let old = inner.bytes.as_slice();
                let mut v = Vec::with_capacity(grown(old.len(), more));
                v.extend_from_slice(old);
                inner.bytes = Bytes::Heap(v);
            }
            None => {
                let old = self.as_bytes();
                let mut v = Vec::with_capacity(grown(old.len(), more));
                v.extend_from_slice(old);
                self.0 = Rc::new(Inner { hash: Cell::new(0), bytes: Bytes::Heap(v) });
            }
        }
        let inner = Rc::get_mut(&mut self.0).expect("unshared above");
        inner.hash.set(0);
        match &mut inner.bytes {
            Bytes::Heap(v) => v,
            Bytes::Inline { .. } => unreachable!("moved to the heap above"),
        }
    }

    /// Append, as `.=` does (in place when this is the only reference).
    pub fn push_bytes(&mut self, bytes: &[u8]) {
        self.writable(bytes.len()).extend_from_slice(bytes);
    }

    /// The string's hash, computed on first use and kept. Nothing PHP
    /// code sees depends on it (arrays keep insertion order).
    #[inline]
    pub fn hash(&self) -> u64 {
        let cached = self.0.hash.get();
        if cached != 0 {
            return cached;
        }
        let h = hash_bytes(self.as_bytes()).max(1);
        self.0.hash.set(h);
        h
    }

    /// Whether two values share one string (no copy was made).
    pub fn ptr_eq(a: &Self, b: &Self) -> bool {
        Rc::ptr_eq(&a.0, &b.0)
    }
}

/// Room for a string of `len` bytes about to grow by `more`: at least
/// double, so a run of appends copies each byte a bounded number of times.
/// A plain copy (no growth asked) is exact.
fn grown(len: usize, more: usize) -> usize {
    if more == 0 {
        return len;
    }
    (len + more).max(len.saturating_mul(2)).max(2 * INLINE)
}

/// A hash of `bytes`, eight at a time (as PHP's DJBX33A is unrolled):
/// each word folded in with a multiply, then the bits mixed so that the
/// low and high ones both depend on every byte (hash tables use both).
fn hash_bytes(bytes: &[u8]) -> u64 {
    const K: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut h = (bytes.len() as u64).wrapping_mul(K);
    let mut words = bytes.chunks_exact(8);
    for w in &mut words {
        let w = u64::from_le_bytes(w.try_into().expect("eight bytes"));
        h = (h ^ w).wrapping_mul(K).rotate_left(29);
    }
    let rest = words.remainder();
    if !rest.is_empty() {
        let mut last = [0u8; 8];
        last[..rest.len()].copy_from_slice(rest);
        h = (h ^ u64::from_le_bytes(last)).wrapping_mul(K).rotate_left(29);
    }
    // MurmurHash3's finalizer.
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    h ^ (h >> 33)
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
    fn empty_and_one_byte_strings_are_shared() {
        assert!(PhpStr::ptr_eq(&PhpStr::from("a"), &PhpStr::new(vec![b'a'])));
        assert!(PhpStr::ptr_eq(&PhpStr::empty(), &PhpStr::from(String::new())));
        assert!(PhpStr::ptr_eq(&PhpStr::from(&[0u8][..]), &PhpStr::concat(b"", &[0])));
        assert!(!PhpStr::ptr_eq(&PhpStr::from("a"), &PhpStr::from("b")));
        // Writing one never changes the shared one.
        let mut s = PhpStr::from("x");
        s.push_bytes(b"y");
        assert_eq!((s.as_bytes(), PhpStr::from("x").as_bytes()), (&b"xy"[..], &b"x"[..]));
        let mut t = PhpStr::empty();
        t.make_mut().push(b'z');
        assert_eq!((t.as_bytes(), PhpStr::empty().as_bytes()), (&b"z"[..], &b""[..]));
    }

    #[test]
    fn ints_and_concatenations_build_in_one_piece() {
        for i in [0, 7, -7, 10, 1234567890, i64::MAX, i64::MIN] {
            assert_eq!(PhpStr::from_int(i).as_bytes(), i.to_string().as_bytes());
        }
        assert!(matches!(PhpStr::from_int(i64::MIN).0.bytes, Bytes::Inline { .. }));
        assert_eq!(PhpStr::concat(b"ab", b"cd").as_bytes(), b"abcd");
        let long = PhpStr::concat(&[b'x'; 20], &[b'y'; 20]);
        assert_eq!(long.as_bytes(), [[b'x'; 20], [b'y'; 20]].concat());
        assert!(matches!(PhpStr::concat(&[b'x'; 11], &[b'y'; 11]).0.bytes, Bytes::Inline { .. }));
    }

    #[test]
    fn hashes_spread_and_follow_the_bytes() {
        let mut seen = std::collections::HashSet::new();
        for i in 0..100_000 {
            assert!(seen.insert(PhpStr::from(format!("key{i}")).hash()));
        }
        // Low bits too (a table's slot): no pile-up on a few values.
        let mut low = [0u32; 256];
        for i in 0..25_600 {
            low[(PhpStr::from(format!("k{i}")).hash() & 255) as usize] += 1;
        }
        assert!(low.iter().all(|&n| (50..=170).contains(&n)), "{low:?}");
        let a = PhpStr::from("same bytes, other strings");
        assert_eq!(a.hash(), PhpStr::new(b"same bytes, other strings".to_vec()).hash());
        assert_ne!(PhpStr::from("ab").hash(), PhpStr::from("ab\0").hash());
    }

    #[test]
    fn appends_grow_by_doubling() {
        // A shared or short string's first append makes room to grow;
        // later appends reallocate O(log n) times.
        let base = PhpStr::from("shared string, longer than inline");
        let mut s = base.clone();
        s.push_bytes(b"!");
        assert!(s.make_mut().capacity() >= 2 * base.len());
        let mut grows = 0;
        let mut cap = s.make_mut().capacity();
        for _ in 0..10_000 {
            s.push_bytes(b"0123456789");
            let c = s.make_mut().capacity();
            grows += usize::from(c != cap);
            cap = c;
        }
        assert!(grows < 20, "{grows}");
        assert_eq!(base.len(), 33);
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
