//! PHP strings: immutable-by-default byte strings, shared until written
//! (copy-on-write through `Rc::make_mut`), with the hash computed once and
//! cached for use as array keys and names.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// What a string handle points at: the bytes and the cached hash. This is
/// the view a `Value` lends out ([`Value::as_str`]), whatever handle holds
/// the string (an `Rc`, or a program-memory [`IStr`]): read it through
/// [`PhpStr`]'s `Deref`.
///
/// The hash is an atomic so that a head can live in program memory,
/// shared by every thread (`Sync`): a relaxed load or store is a plain
/// `mov` on x86-64 and aarch64, the same code a `Cell` made.
pub struct StrHead {
    /// 0 until computed (a real hash of 0 is stored as 1).
    hash: AtomicU64,
    bytes: Bytes,
}

impl Clone for StrHead {
    fn clone(&self) -> Self {
        StrHead { hash: AtomicU64::new(self.hash.load(Ordering::Relaxed)), bytes: self.bytes.clone() }
    }
}

impl StrHead {
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.as_bytes().len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.as_bytes().is_empty()
    }

    /// The bytes allocated beside the head (none for a short string,
    /// whose bytes are in it).
    pub fn heap_bytes(&self) -> usize {
        match &self.bytes {
            Bytes::Inline { .. } => 0,
            Bytes::Heap(v) => v.capacity(),
        }
    }

    /// The string's hash, computed on first use and kept. Nothing PHP
    /// code sees depends on it (arrays keep insertion order).
    ///
    /// Inline it is the load and a test (ZSTR_H); computing it is out of
    /// line, so a lookup by a string whose hash is known (a literal, a key
    /// read before) carries no hashing code.
    #[inline]
    pub fn hash(&self) -> u64 {
        let cached = self.hash.load(Ordering::Relaxed);
        if cached != 0 {
            return cached;
        }
        self.compute_hash()
    }

    /// The first byte, 0 for the empty string (as PHP's NUL-terminated
    /// `*ZSTR_VAL`): what the numeric-key test looks at first. A short
    /// string's buffer is zero past its length, so this is one load.
    #[inline(always)]
    pub fn first_byte(&self) -> u8 {
        match &self.bytes {
            Bytes::Inline { buf, .. } => buf[0],
            Bytes::Heap(v) => v.first().copied().unwrap_or(0),
        }
    }

    /// Whether two heads hold the same bytes, decided inline when both
    /// are short: a short string's buffer is zero past its length and
    /// never written once made (a write moves it to the heap), so equal
    /// bytes are an equal length and an equal buffer, compared as words
    /// with no call. None when either is on the heap (compare the slices).
    #[inline(always)]
    pub fn short_eq(&self, other: &StrHead) -> Option<bool> {
        match (&self.bytes, &other.bytes) {
            (Bytes::Inline { len: a, buf: x }, Bytes::Inline { len: b, buf: y }) => Some(a == b && x == y),
            _ => None,
        }
    }

    /// Whether two heads hold the same bytes, where a comparison operator
    /// can tell with no call: the same head, two lengths that differ, two
    /// short strings (`short_eq`). None for two strings of one length
    /// with either on the heap (compare the slices).
    #[inline(always)]
    pub fn eq_in_place(&self, other: &StrHead) -> Option<bool> {
        if std::ptr::eq(self, other) {
            return Some(true);
        }
        match (&self.bytes, &other.bytes) {
            (Bytes::Inline { len: a, buf: x }, Bytes::Inline { len: b, buf: y }) => Some(a == b && x == y),
            // (The lengths read here, not through `as_slice`: no call.)
            (Bytes::Inline { len, .. }, Bytes::Heap(v)) | (Bytes::Heap(v), Bytes::Inline { len, .. }) => {
                (usize::from(*len) != v.len()).then_some(false)
            }
            (Bytes::Heap(v), Bytes::Heap(w)) => (v.len() != w.len()).then_some(false),
        }
    }

    /// Whether both hashes are known and differ: two strings that are
    /// then not the same bytes, told by two loads. (False says nothing.)
    #[inline(always)]
    pub fn hashes_differ(&self, other: &StrHead) -> bool {
        let (a, b) = (self.hash.load(Ordering::Relaxed), other.hash.load(Ordering::Relaxed));
        a != 0 && b != 0 && a != b
    }

    /// `hash` the first time (zend_string_hash_func).
    #[cold]
    #[inline(never)]
    fn compute_hash(&self) -> u64 {
        crate::counts::bump(crate::counts::Count::Hash);
        let h = hash_bytes(self.as_bytes()).max(1);
        self.hash.store(h, Ordering::Relaxed);
        h
    }

    /// A head of these bytes, its hash not computed yet.
    fn of(bytes: Bytes) -> StrHead {
        StrHead { hash: AtomicU64::new(0), bytes }
    }

    /// A string handle's allocation, holding these bytes (each one
    /// counted, for the tests: `counts::Count::StrMade`).
    #[inline(always)]
    fn made(bytes: Bytes) -> Rc<StrHead> {
        crate::counts::bump(crate::counts::Count::StrMade);
        Rc::new(StrHead::of(bytes))
    }
}

/// A string in program memory (PHP's interned strings): a compiled
/// literal, made once for the process and never freed, so a value
/// holding one is a plain pointer: copied with no count, never released.
/// Its hash is computed when it is made.
///
/// Made by [`IStr::intern`] only, from a pool bounded in size (literals
/// past the bound stay ordinary strings), and the same bytes give the
/// same `IStr`: compiling a file again (a new version, another request
/// with no code cache) adds only the literals not seen before.
#[repr(C)]
pub struct IStr {
    /// First (`repr(C)`), so a value's string view is at the pointer.
    head: StrHead,
    /// Its index in each thread's twins (`IStr::to_php_str`).
    id: u32,
}

/// The program-memory strings, by their bytes.
struct Pool {
    by_bytes: crate::fast::HashMap<&'static [u8], &'static IStr>,
    /// Bytes held (the strings' and about their overhead).
    size: usize,
}

static POOL: Mutex<Option<Pool>> = Mutex::new(None);

/// What the pool may hold, in bytes (PHP's opcache.interned_strings_buffer
/// is 8 MB by default, for names as well; past it PHP too makes literals
/// per request). Bounded so that code compiled again and again (eval,
/// generated files) cannot grow program memory without end.
const POOL_LIMIT: usize = 64 << 20;

/// What one pooled string costs besides its bytes (the IStr, the map's
/// entry), counted against the limit.
const POOL_ENTRY: usize = 64;

/// The strings the pool has room for when it is first made.
const POOL_FIRST: usize = 1 << 14;

thread_local! {
    /// Each program-memory string as this thread's ordinary string, made
    /// the first time one is wanted (an array key, an owned handle) and
    /// kept: by the `IStr`'s id. Taking one is an `Rc` count, not a copy.
    static TWINS: RefCell<Vec<Option<PhpStr>>> = const { RefCell::new(Vec::new()) };
}

impl IStr {
    /// The program-memory string of `bytes`: the one already made, or a
    /// new one while the pool has room. None past the bound.
    pub fn intern(bytes: &[u8]) -> Option<&'static IStr> {
        let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
        // Sized for an application's literals from the start (a WordPress
        // page interns over ten thousand): the map grew through 13
        // rehashes otherwise.
        let pool = pool.get_or_insert_with(|| Pool { by_bytes: crate::fast::HashMap::with_capacity_and_hasher(POOL_FIRST, Default::default()), size: 0 });
        if let Some(s) = pool.by_bytes.get(bytes) {
            return Some(s);
        }
        let size = bytes.len() + POOL_ENTRY;
        if pool.size + size > POOL_LIMIT {
            return None;
        }
        let id = u32::try_from(pool.by_bytes.len()).ok()?;
        let bytes = if bytes.len() <= INLINE {
            let mut buf = [0; INLINE];
            buf[..bytes.len()].copy_from_slice(bytes);
            Bytes::Inline { len: bytes.len() as u8, buf }
        } else {
            Bytes::Heap(bytes.to_vec())
        };
        let s: &'static IStr = Box::leak(Box::new(IStr { head: StrHead::of(bytes), id }));
        s.head.hash();
        pool.by_bytes.insert(s.as_bytes(), s);
        pool.size += size;
        Some(s)
    }

    /// The program-memory string of `bytes` if one was made (none is made here).
    pub fn lookup(bytes: &[u8]) -> Option<&'static IStr> {
        let pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
        pool.as_ref()?.by_bytes.get(bytes).copied()
    }

    /// The string as an ordinary one, owned: this thread's twin of it
    /// (one `Rc` count; made once per thread).
    pub fn to_php_str(&'static self) -> PhpStr {
        let twin = TWINS.try_with(|twins| {
            let mut twins = twins.borrow_mut();
            let i = self.id as usize;
            if let Some(Some(s)) = twins.get(i) {
                return s.clone();
            }
            if twins.len() <= i {
                twins.resize(i + 1, None);
            }
            let s = PhpStr::copy_of(self.as_bytes());
            s.hash.store(self.head.hash(), Ordering::Relaxed);
            twins[i] = Some(s.clone());
            s
        });
        // (While the thread is torn down: a copy.)
        twin.unwrap_or_else(|_| PhpStr::copy_of(self.as_bytes()))
    }
}

/// A value's string, to read: the string itself when the value is one
/// (any handle, a literal's included, with no handle made), else the
/// string it converts to. Reads as its head (`as_bytes`, `len`, `hash`).
pub enum StrView<'a> {
    Borrowed(&'a StrHead),
    Owned(PhpStr),
}

impl Deref for StrView<'_> {
    type Target = StrHead;

    #[inline(always)]
    fn deref(&self) -> &StrHead {
        match self {
            StrView::Borrowed(s) => s,
            StrView::Owned(s) => s,
        }
    }
}

impl Deref for IStr {
    type Target = StrHead;

    #[inline(always)]
    fn deref(&self) -> &StrHead {
        &self.head
    }
}

impl fmt::Debug for IStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.head.fmt(f)
    }
}

impl PartialEq for StrHead {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other) || (self.hash() == other.hash() && self.as_bytes() == other.as_bytes())
    }
}

impl Eq for StrHead {}

impl fmt::Debug for StrHead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", String::from_utf8_lossy(self.as_bytes()))
    }
}

type Inner = StrHead;

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

thread_local! {
    /// The buffer strings are built in (PhpStr::scratch), between builds.
    static SCRATCH: Cell<Vec<u8>> = const { Cell::new(Vec::new()) };
}

/// The biggest scratch buffer kept for the next build.
const SCRATCH_KEEP: usize = 64 * 1024;

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
        PhpStr(Inner::made(Bytes::Heap(bytes)))
    }

    /// A buffer to build a string in (smart_str), with room for `hint`
    /// bytes: the thread's reused one, so building allocates nothing until
    /// the string is made from it ([`PhpStr::from_scratch`]).
    pub fn scratch(hint: usize) -> Vec<u8> {
        let mut buf = SCRATCH.with(|s| s.take());
        buf.clear();
        buf.reserve(hint);
        buf
    }

    /// The string built in `buf` (from [`PhpStr::scratch`]): a short one in
    /// its own allocation (the buffer kept for the next), a longer one in
    /// the buffer itself when it is about its size, else in an exact copy.
    pub fn from_scratch(buf: Vec<u8>) -> PhpStr {
        let keep = |buf: Vec<u8>| {
            if buf.capacity() <= SCRATCH_KEEP {
                SCRATCH.with(|s| s.set(buf));
            }
        };
        if buf.len() <= INLINE || buf.capacity() > 2 * buf.len() + 64 {
            let s = Self::copy_of(&buf);
            keep(buf);
            return s;
        }
        PhpStr::new(buf)
    }

    /// `parts` one after another, in one piece: a short result in the
    /// string's own allocation, a longer one in a buffer of its size.
    pub fn concat_all(parts: &[&[u8]]) -> Self {
        let len: usize = parts.iter().map(|p| p.len()).sum();
        if len <= INLINE {
            let mut buf = [0; INLINE];
            let mut at = 0;
            for p in parts {
                buf[at..at + p.len()].copy_from_slice(p);
                at += p.len();
            }
            return Self::copy_of(&buf[..len]);
        }
        let mut v = Vec::with_capacity(len);
        for p in parts {
            v.extend_from_slice(p);
        }
        Self::new(v)
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
        PhpStr(Inner::made(Bytes::Inline { len: bytes.len() as u8, buf }))
    }

    /// A copy of `bytes`: in the string's own allocation when short, and
    /// no allocation at all for zero or one bytes.
    fn copy_of(bytes: &[u8]) -> Self {
        if bytes.len() > INLINE {
            return PhpStr(Inner::made(Bytes::Heap(bytes.to_vec())));
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
                self.0 = Inner::made(Bytes::Heap(v));
            }
        }
        let inner = Rc::get_mut(&mut self.0).expect("unshared above");
        *inner.hash.get_mut() = 0;
        match &mut inner.bytes {
            Bytes::Heap(v) => v,
            Bytes::Inline { .. } => unreachable!("moved to the heap above"),
        }
    }

    /// Append, as `.=` does (in place when this is the only reference).
    pub fn push_bytes(&mut self, bytes: &[u8]) {
        self.writable(bytes.len()).extend_from_slice(bytes);
    }

    /// Whether two values share one string (no copy was made).
    pub fn ptr_eq(a: &Self, b: &Self) -> bool {
        Rc::ptr_eq(&a.0, &b.0)
    }

    /// How many handles share the string (1: dropping this one frees it).
    #[inline(always)]
    pub fn count(&self) -> usize {
        Rc::strong_count(&self.0)
    }
}

/// A handle reads as its head: `s.as_bytes()`, `s.len()`, `s.hash()`.
impl Deref for PhpStr {
    type Target = StrHead;

    #[inline(always)]
    fn deref(&self) -> &StrHead {
        &self.0
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
    let (words, rest) = bytes.as_chunks::<8>();
    for w in words {
        h = (h ^ u64::from_le_bytes(*w)).wrapping_mul(K).rotate_left(29);
    }
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

    /// `short_eq` decides two short strings by their buffers (zero past the
    /// length, whatever built them) and leaves a heap one to the slices;
    /// `first_byte` is 0 for the empty string.
    #[test]
    fn short_strings_compare_by_their_buffers() {
        let built = PhpStr::concat(b"ab", b"c");
        let int = PhpStr::from_int(-12);
        assert_eq!(PhpStr::from("abc").short_eq(&built), Some(true));
        assert_eq!(PhpStr::from("-12").short_eq(&int), Some(true));
        assert_eq!(PhpStr::from("abd").short_eq(&built), Some(false));
        assert_eq!(PhpStr::from("ab").short_eq(&built), Some(false));
        assert_eq!(PhpStr::from("ab\0").short_eq(&PhpStr::from("ab")), Some(false));
        let i = IStr::intern(b"abc").expect("room in the pool");
        assert_eq!(i.short_eq(&built), Some(true));
        assert_eq!(PhpStr::new(b"abc".to_vec()).short_eq(&built), None);
        assert_eq!((PhpStr::empty().first_byte(), PhpStr::new(Vec::new()).first_byte(), PhpStr::new(b"-1".to_vec()).first_byte()), (0, 0, b'-'));
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
    fn built_strings_take_one_allocation_when_short_and_their_buffer_when_long() {
        let mut b = PhpStr::scratch(4);
        b.extend_from_slice(b"short one");
        let s = PhpStr::from_scratch(b);
        assert!(matches!(s.0.bytes, Bytes::Inline { .. }));
        assert_eq!(s.as_bytes(), b"short one");
        // The buffer came back: the next build starts with its room.
        assert!(PhpStr::scratch(0).capacity() >= 9);
        let mut b = PhpStr::scratch(40);
        b.extend_from_slice(&[b'x'; 40]);
        let ptr = b.as_ptr();
        let long = PhpStr::from_scratch(b);
        // About its size: the string is the buffer itself, no copy.
        assert_eq!((long.len(), long.as_bytes().as_ptr()), (40, ptr));
        assert_eq!(PhpStr::concat_all(&[b"\0", b"Class", b"\0", b"p"]).as_bytes(), b"\0Class\0p");
        assert!(matches!(PhpStr::concat_all(&[b"ab", b"cd"]).0.bytes, Bytes::Inline { .. }));
        assert_eq!(PhpStr::concat_all(&[&[b'y'; 30], b"z"]).len(), 31);
        assert!(PhpStr::ptr_eq(&PhpStr::concat_all(&[]), &PhpStr::empty()));
    }

    #[test]
    fn program_memory_strings_are_made_once_with_their_hash_and_one_twin_per_thread() {
        let bytes = b"a literal in program memory, longer than inline";
        assert!(IStr::lookup(bytes).is_none());
        let s = IStr::intern(bytes).expect("room");
        assert!(std::ptr::eq(s, IStr::intern(bytes).expect("room")));
        assert!(std::ptr::eq(s, IStr::lookup(bytes).expect("made")));
        assert_eq!(s.as_bytes(), bytes);
        // Hashed when made, and as an ordinary string of the same bytes.
        assert_eq!(s.hash.load(Ordering::Relaxed), PhpStr::from(&bytes[..]).hash());
        // The thread's twin is shared, not copied each time.
        let (a, b) = (s.to_php_str(), s.to_php_str());
        assert!(PhpStr::ptr_eq(&a, &b));
        assert_eq!(a.as_bytes(), bytes);
        // Another thread has its own.
        let other = std::thread::spawn(move || s.to_php_str().as_bytes().as_ptr() as usize).join().expect("ran");
        assert_ne!(other, a.as_bytes().as_ptr() as usize);
        let short = IStr::intern(b"ab").expect("room");
        assert!(matches!(short.bytes, Bytes::Inline { .. }));
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
