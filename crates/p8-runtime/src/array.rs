//! PHP arrays: ordered maps with int and string keys. A list (keys 0..n-1
//! in order) is stored packed as a plain vector; anything else is an
//! ordered hash with tombstones, so unset is O(1) as in PHP and order is
//! kept. Arrays are values: `Rc<PhpArray>` is shared until written
//! (`Rc::make_mut`). Key rules follow zend_hash.c / zend_operators.c.

use crate::string::{PhpStr, StrHead};
use crate::value::{Kind, Value};
use crate::number::dval_to_lval;
use std::cell::RefCell;
use std::cmp::Ordering;

/// Freed array buffers of the two sizes most arrays have (8 and 16
/// entries), kept for the next array on this thread (an isolate's): a new
/// small array, or one growing to 16, takes one instead of allocating.
/// What they held is gone before they are kept (nothing outlives its
/// request); at most POOL_MAX of each.
struct Pool<T> {
    eight: Vec<Vec<T>>,
    sixteen: Vec<Vec<T>>,
}

const POOL_MAX: usize = 256;

thread_local! {
    static VALUES: RefCell<Pool<Value>> = const { RefCell::new(Pool { eight: Vec::new(), sixteen: Vec::new() }) };
    static BUCKETS: RefCell<Pool<Bucket>> = const { RefCell::new(Pool { eight: Vec::new(), sixteen: Vec::new() }) };
    /// Arrays copied on this thread (zend_array_dup's count): every copy
    /// allocates, so counting one is noise beside it.
    static COPIES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How many arrays this thread has copied (a separation before a write,
/// or an explicit copy): what tests of copy-on-write assert on.
pub fn copies() -> u64 {
    COPIES.with(std::cell::Cell::get)
}

/// The pool size for room of `n` (None: not pooled).
#[inline]
fn class(n: usize) -> Option<usize> {
    match n {
        0..=8 => Some(8),
        9..=16 => Some(16),
        _ => None,
    }
}

/// An empty buffer with room for at least `n`: a kept one when `n` is a
/// pooled size, else a new one.
#[inline]
fn take<T>(pool: &'static std::thread::LocalKey<RefCell<Pool<T>>>, n: usize) -> Vec<T> {
    let Some(size) = class(n) else { return Vec::with_capacity(n) };
    let kept = pool
        .try_with(|p| p.try_borrow_mut().ok().and_then(|mut p| if size == 8 { p.eight.pop() } else { p.sixteen.pop() }))
        .ok()
        .flatten();
    kept.unwrap_or_else(|| Vec::with_capacity(size))
}

/// Keep `v` for a later array when it is a pooled size (emptied first:
/// its values go now, as they would with the buffer). Always inline: the
/// array's free ([`free`]) is its hot caller, and a call there was a
/// second prologue per array.
#[inline(always)]
fn give<T>(pool: &'static std::thread::LocalKey<RefCell<Pool<T>>>, mut v: Vec<T>) {
    let size = v.capacity();
    if size != 8 && size != 16 {
        return;
    }
    // (Dropping the values may free other arrays, which give theirs: the
    // pool is not borrowed meanwhile.)
    v.clear();
    let _ = pool.try_with(|p| {
        if let Ok(mut p) = p.try_borrow_mut() {
            let list = if size == 8 { &mut p.eight } else { &mut p.sixteen };
            if list.len() < POOL_MAX {
                list.push(v);
            }
        }
    });
}

/// A values buffer with room for `n` (an object's properties, a frame's
/// extra arguments): from the pool when `n` is a pooled size.
pub fn values_buffer(n: usize) -> Vec<Value> {
    if n == 0 { Vec::new() } else { take(&VALUES, n) }
}

/// Keep a values buffer that is done with (see [`values_buffer`]).
pub fn keep_values_buffer(v: Vec<Value>) {
    give(&VALUES, v);
}

impl Drop for Packed {
    fn drop(&mut self) {
        give(&VALUES, std::mem::take(&mut self.slots));
    }
}

impl Drop for Hashed {
    fn drop(&mut self) {
        give(&BUCKETS, std::mem::take(&mut self.buckets));
    }
}

/// The last handle of an array goes (PHP's `rc_dtor_func` reaching
/// `zend_array_destroy`): in this one function, the array taken out of
/// its `Rc`, its values released in one walk of its buffer and the
/// buffer kept for the next array ([`give`]); an empty array only frees
/// its box. Letting the `Rc` drop instead went through four more
/// out-of-line layers (`Rc::drop_slow`, the `Storage` glue, `Packed`'s or
/// `Hashed`'s `Drop`, each with its own prologue: ~100 instructions an
/// array before the first value). The values go in the same order and at
/// the same point as there, so destructors are queued as they were; only
/// the array's own box is freed before its values rather than after.
#[inline(never)]
pub(crate) fn free(a: std::rc::Rc<PhpArray>) {
    // (No weak handles to arrays exist; with one, `into_inner` still gives
    // the array when this was the last strong handle.)
    let Some(mut array) = std::rc::Rc::into_inner(a) else { return };
    match &mut array.storage {
        Storage::Packed(p) => give(&VALUES, std::mem::take(&mut p.slots)),
        Storage::Hashed(h) => give(&BUCKETS, std::mem::take(&mut h.buckets)),
    }
    // What is left holds nothing (an empty Vec): its `Drop` would only
    // give back an empty buffer.
    std::mem::forget(array);
}


/// An array key: PHP normalizes "123" to 123, so a key is an int or a
/// string that is not a canonical integer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Int(i64),
    Str(PhpStr),
}

/// What normalizing a value into a key reported, for the engine to show.
#[derive(Debug, Clone, PartialEq)]
pub enum KeyIssue {
    /// A float with a fractional part (8.1 deprecation).
    LosesPrecision(f64),
    /// Not usable as a key (array, object): "Illegal offset type".
    Illegal(&'static str),
    /// Null (or undefined) as a key: the empty string, deprecated since 8.5
    /// where PHP reads, writes or tests an offset (not for unset()).
    NullOffset,
}

/// What PHP 8.5+ says when null is an array offset.
pub const NULL_OFFSET_DEPRECATION: &str = "Using null as an array offset is deprecated, use an empty string instead";

impl Key {
    /// The key a string names: a canonical decimal integer is an int key
    /// (ZEND_HANDLE_NUMERIC_STR: no leading zeros, no "-0", fits in i64).
    ///
    /// Out of line: the interpreter's fast loop builds keys from strings
    /// (array literals), and the test inlined there costs the loop's other
    /// paths registers.
    #[inline(never)]
    pub fn from_string(s: PhpStr) -> Key {
        match canonical_int(&s) {
            Some(i) => Key::Int(i),
            None => Key::Str(s),
        }
    }

    /// The key a value names (null is "", bools and floats are ints).
    pub fn from_value(value: &Value) -> Result<(Key, Option<KeyIssue>), KeyIssue> {
        Ok(match value.kind() {
            Kind::Int => (Key::Int(value.as_int().expect("an int")), None),
            Kind::Str => (Key::from_string(value.cloned_str().expect("a string")), None),
            Kind::Null | Kind::Undef => (Key::Str(PhpStr::empty()), Some(KeyIssue::NullOffset)),
            Kind::Bool => (Key::Int(i64::from(value.as_bool().expect("a bool"))), None),
            Kind::Float => {
                let f = value.as_float().expect("a float");
                // Not an int as it is: a fraction, NaN/INF, or out of the
                // int range (zend_dval_to_lval_safe's deprecation).
                let fits = (-9.223_372_036_854_776e18..9.223_372_036_854_776e18).contains(&f);
                let issue = (f.fract() != 0.0 || !fits).then_some(KeyIssue::LosesPrecision(f));
                (Key::Int(dval_to_lval(f)), issue)
            }
            Kind::Array => return Err(KeyIssue::Illegal("array")),
            Kind::Object => return Err(KeyIssue::Illegal("object")),
            Kind::Ref => return Key::from_value(&value.as_reference().expect("a reference").borrow()),
        })
    }

    /// The key as a value, without a copy of a string key's handle.
    pub fn into_value(self) -> Value {
        match self {
            Key::Int(i) => Value::int(i),
            Key::Str(s) => Value::str(s),
        }
    }

    /// The key as a value (what foreach and array_keys give).
    pub fn to_value(&self) -> Value {
        match self {
            Key::Int(i) => Value::int(*i),
            Key::Str(s) => Value::str(s.clone()),
        }
    }
}

/// What one position of an array holds ([`PhpArray::value_here`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Here<T> {
    /// An entry.
    Entry(T),
    /// An unset entry's place: the walk goes on past it.
    Hole,
    /// Past the last position.
    End,
}

/// What one position of an array holds, with the key
/// ([`PhpArray::entry_here`]). The key is read in place: an int, or the
/// string the array holds (borrowed, so reading it copies no handle).
/// One level, each case made where the array finds it: the reader's
/// `match` is a jump from there, with no kind to work out in between.
#[derive(Debug, Clone, Copy)]
pub enum EntryHere<'a> {
    /// An entry with an int key.
    Int(i64, &'a Value),
    /// An entry with a string key.
    Str(&'a PhpStr, &'a Value),
    /// An unset entry's place: the walk goes on past it.
    Hole,
    /// Past the last position.
    End,
}

/// The keys of a record ([`PhpArray::record`]), each hashed now: made once
/// (a thread-local of the builtin's) and shared by every record built from
/// them. Distinct strings that are not integers (asserted).
pub fn record_keys<const N: usize>(names: [&str; N]) -> [PhpStr; N] {
    for (i, n) in names.iter().enumerate() {
        assert!(!names[..i].contains(n), "record keys are distinct");
    }
    names.map(|n| {
        assert!(int_key(n.as_bytes()).is_none(), "a record key is a string key");
        let s = PhpStr::from(n);
        s.hash();
        s
    })
}

/// A canonical decimal integer: "0", "-12", not "012", "-0", "+1", " 1".
///
/// ZEND_HANDLE_NUMERIC_STR: the first byte is tested inline, and only a
/// key starting with a digit, or with '-' and a digit, goes on to
/// [`canonical_int_rest`]. Most string keys start with a letter or '_'
/// (above '9'), so a lookup by one costs two compares here.
#[inline(always)]
fn canonical_int(s: &StrHead) -> Option<i64> {
    let c = s.first_byte();
    if c > b'9' || (c < b'0' && c != b'-') {
        return None;
    }
    canonical_int_rest(s.as_bytes())
}

/// The int key a string's bytes name, if any: [`Key::from_string`]'s test,
/// for a compiler that has bytes, not a string (no string made).
pub fn int_key(b: &[u8]) -> Option<i64> {
    match b.first() {
        Some(&c) if c.is_ascii_digit() || c == b'-' => canonical_int_rest(b),
        _ => None,
    }
}

/// `canonical_int` past its first byte (a digit or '-'): as
/// _zend_handle_numeric_str_ex, digit by digit, no leading zero, no "-0",
/// and in the i64 range.
#[inline(never)]
fn canonical_int_rest(b: &[u8]) -> Option<i64> {
    let neg = b[0] == b'-';
    let digits = &b[usize::from(neg)..];
    // (MAX_LENGTH_OF_LONG - 1: 19 digits; i64::MIN has 19 as well.)
    match digits {
        [] => return None,
        [b'0', ..] if b.len() > 1 => return None,
        _ if digits.len() > 19 => return None,
        _ => {}
    }
    let mut n: u64 = 0;
    for &d in digits {
        if !d.is_ascii_digit() {
            return None;
        }
        // (19 digits fit in a u64: no overflow here.)
        n = n * 10 + u64::from(d - b'0');
    }
    if neg {
        // Down to i64::MIN, whose magnitude is i64::MAX + 1.
        (n <= i64::MAX as u64 + 1).then(|| (n as i64).wrapping_neg())
    } else {
        i64::try_from(n).ok()
    }
}

/// `$a[] = v` when the next key would overflow: "Cannot add element to the
/// array as the next element is already occupied".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NextKeyOccupied;

#[derive(Debug, Clone)]
enum Storage {
    /// Int keys in increasing order, each at its own index, as PHP's packed
    /// arrays: `slots[k]` is key `k`, undefined where there is no such
    /// key (a hole: `[1 => x]`, or an unset). A key that would break the
    /// order (filling a hole), a string key, or a far-off int key turns
    /// the array into a hash.
    Packed(Packed),
    Hashed(Hashed),
}

#[derive(Debug, Default)]
struct Packed {
    slots: Vec<Value>,
    /// How many slots are holes.
    holes: usize,
}

impl Packed {
    #[inline]
    fn get(&self, i: i64) -> Option<&Value> {
        usize::try_from(i).ok().and_then(|i| self.slots.get(i)).filter(|v| !v.is_undef())
    }

    #[inline]
    fn get_mut(&mut self, i: i64) -> Option<&mut Value> {
        usize::try_from(i).ok().and_then(|i| self.slots.get_mut(i)).filter(|v| !v.is_undef())
    }

    /// A buffer to put key `i` (past the end) in where the buffer is
    /// none, or the eight-slot one and `i` is below sixteen: a kept buffer
    /// of the pool's size (the values moved to it). Anything larger grows
    /// as the vector does.
    #[inline]
    fn room_for(&mut self, i: usize) {
        if self.slots.capacity() == 0 {
            self.slots = take(&VALUES, MIN_PACKED.max(i + 1));
        } else if i >= self.slots.capacity() && self.slots.capacity() == 8 && i < 16 {
            // From eight to sixteen: a kept buffer, the values moved.
            let mut more = take(&VALUES, 16);
            more.append(&mut self.slots);
            give(&VALUES, std::mem::replace(&mut self.slots, more));
        }
    }

    /// Whether key `i`, past the end, still fits a packed array: no more
    /// holes than values (or a handful), as PHP's packed arrays allow.
    fn fits(&self, i: i64) -> bool {
        let Ok(i) = usize::try_from(i) else { return false };
        i >= self.slots.len() && (i < 8 || i < 2 * (self.slots.len() - self.holes + 1))
    }
}

/// An insertion-ordered hash shaped as PHP's (zend_types.h, zend_hash.c):
/// buckets in insertion order, each holding its key's hash, the key and
/// the value (32 bytes, as a zend Bucket). A lookup compares the stored
/// hash before the key, so a stored key is never hashed again, and growing
/// rebuilds the index from the stored hashes.
///
/// The table is ONE allocation, as PHP's arData is: its size (a power of
/// two) in buckets, the first `used` the entries in order (positions are
/// indices), the rest empty room. The index lives in the buckets too: each
/// bucket's `head` is the chain head of hash slot `i` (one slot per
/// bucket), its `next` the next entry of its own chain, as PHP keeps the
/// hash slots in front of the buckets and `next` in the zval. 40 bytes a
/// bucket either way (PHP: 32, and two u32 slots).
///
/// A small table (up to SMALL buckets: most arrays, records of a few keys)
/// has no chains: a lookup scans its buckets' hashes.
///
/// An unset entry stays in place (positions do not move) until the table
/// is compacted; its `next` is UNSET. (Not its value: any value, Undef
/// included (an uninitialized typed static property in
/// get_class_vars()), may be an entry's.)
#[derive(Debug, Default)]
struct Hashed {
    /// The table: entries, then room (see above).
    buckets: Vec<Bucket>,
    /// How many positions are taken (entries, unset ones included).
    used: usize,
    live: usize,
}

/// No bucket (the end of a chain, an empty slot).
const NONE: u32 = u32::MAX;
/// An unset entry's `next`.
const UNSET: u32 = u32::MAX - 1;
/// The first table's size: eight buckets, as zend_hash's minimum (a table
/// of a few keys never grows, so is one allocation).
const MIN_SIZE: usize = 8;
/// A packed array's first room once it is pushed to (PHP's minimum too):
/// one allocation for a list of up to eight, not a second at five.
const MIN_PACKED: usize = 8;
/// Up to this many buckets a table has no chains (scanned).
const SMALL: usize = 8;

#[derive(Debug)]
struct Bucket {
    /// The key's hash: an int key itself, a string key's string hash
    /// (PHP's `h`).
    h: u64,
    /// A string key; None for an int key (which `h` is).
    key: Option<PhpStr>,
    value: Value,
    /// The next entry in this entry's chain (NONE: the last; UNSET: the
    /// entry is unset).
    next: u32,
    /// The first entry of hash slot `i`'s chain, `i` this bucket's
    /// position (a large table's).
    head: u32,
}

impl Bucket {
    /// Room for an entry.
    fn empty() -> Bucket {
        Bucket { h: 0, key: None, value: Value::NULL, next: NONE, head: NONE }
    }
}

impl Bucket {
    fn key(&self) -> Key {
        match &self.key {
            Some(s) => Key::Str(s.clone()),
            None => Key::Int(self.h as i64),
        }
    }

    #[inline]
    fn is(&self, h: u64, key: Probe<'_>) -> bool {
        self.h == h
            && match (key, &self.key) {
                (Probe::Int, None) => true,
                (Probe::Str(s), Some(k)) => std::ptr::eq(s, &**k) || s.as_bytes() == k.as_bytes(),
                _ => false,
            }
    }
}

/// A key looked for, borrowed: an int key (its hash `h` is the int), or a
/// string key's head, whatever handle holds it (a program-memory string
/// is looked up with no handle made).
#[derive(Clone, Copy)]
enum Probe<'a> {
    Int,
    Str(&'a StrHead),
}

/// Whether a probe `s` is the stored key `k`, their hashes equal, decided
/// with no call where it can be: the same head (zend_hash_find_bucket's
/// `p->key == key`), or two short strings compared as words. None when
/// the bytes must be compared as slices (a heap string on either side).
///
/// A literal is rarely the same head as the key it finds: an array built
/// from literals holds the thread's handle of each (its twin), not the
/// program-memory head the lookup has, so the short compare is the
/// common case for literal keys, not a fallback.
#[inline(always)]
fn same_key(s: &StrHead, k: &StrHead) -> Option<bool> {
    if std::ptr::eq(s, k) {
        return Some(true);
    }
    s.short_eq(k)
}

impl Key {
    #[inline(always)]
    fn probe(&self) -> Probe<'_> {
        match self {
            Key::Int(_) => Probe::Int,
            Key::Str(s) => Probe::Str(s),
        }
    }
}

/// An element as a copy of its array holds it (zend_array_dup), and a
/// property as a clone holds it (zval_add_ref): a reference nothing else
/// holds (its variable gone) is a plain value.
///
/// Inline, as zend_array_dup's element copy is: an array's copy runs it
/// once per element, and a call per element cost as much as the copy
/// itself. Only a reference goes out of line ([`dup_ref`]).
#[inline(always)]
pub(crate) fn dup(v: &Value) -> Value {
    match v.as_reference() {
        Some(cell) => dup_ref(v, cell),
        None => v.clone(),
    }
}

/// [`dup`] of a reference `v` to `cell`.
#[inline(never)]
fn dup_ref(v: &Value, cell: &std::rc::Rc<std::cell::RefCell<Value>>) -> Value {
    if std::rc::Rc::strong_count(cell) == 1 { cell.borrow().clone() } else { v.clone() }
}

impl Clone for Packed {
    fn clone(&self) -> Self {
        // A small list's copy in a kept buffer when there is one.
        let mut slots = match class(self.slots.len()) {
            Some(_) if !self.slots.is_empty() => take(&VALUES, self.slots.len()),
            _ => Vec::with_capacity(self.slots.len()),
        };
        slots.extend(self.slots.iter().map(dup));
        Packed { slots, holes: self.holes }
    }
}

/// zend_array_dup: the table copied into one allocation of its size,
/// chains and all. Only the taken positions are copied entries; the room
/// after them is empty buckets that keep their chain heads (the index of
/// the whole table), as PHP copies nNumUsed buckets and the hash part.
impl Clone for Hashed {
    fn clone(&self) -> Self {
        // (In a kept buffer when its size is a pooled one.)
        let mut buckets = take(&BUCKETS, self.buckets.len());
        let (taken, room) = self.buckets.split_at(self.used);
        buckets.extend(taken.iter().map(|b| Bucket { h: b.h, key: b.key.clone(), value: dup(&b.value), next: b.next, head: b.head }));
        buckets.extend(room.iter().map(|b| Bucket { h: 0, key: None, value: Value::NULL, next: NONE, head: b.head }));
        Hashed { buckets, used: self.used, live: self.live }
    }
}

impl Hashed {
    /// Empty, with room for `n` entries before it grows.
    fn with_capacity(n: usize) -> Hashed {
        let size = n.max(MIN_SIZE).next_power_of_two();
        assert!(size < UNSET as usize, "array too large");
        let mut buckets = take(&BUCKETS, size);
        buckets.resize_with(size, Bucket::empty);
        Hashed { buckets, used: 0, live: 0 }
    }

    /// Whether the table is small (scanned, no chains).
    #[inline]
    fn small(&self) -> bool {
        self.buckets.len() <= SMALL
    }

    /// Positions taken. Positions are the buckets' indices.
    fn len(&self) -> usize {
        self.used
    }

    /// The hash slot of `h` (Fibonacci hashing: sequential ints spread).
    #[inline]
    fn slot(&self, h: u64) -> usize {
        (h.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> (64 - self.buckets.len().trailing_zeros())) as usize
    }

    /// The position of `key` (its hash `h`). Out of line: the interpreter's
    /// fast loop inlines array reads, and only their packed part belongs
    /// there.
    ///
    /// A leaf: it compares hashes, and a string key by [`same_key`] (its
    /// head's address, or a short string's words). The first entry whose
    /// hash matches and which is a heap string is handed to
    /// [`Hashed::find_bytes`], which compares bytes from there on. A
    /// function that calls `bcmp` itself has to save and restore every
    /// register the loop uses around it, in every call: that was a third
    /// of this function's cost.
    #[inline(never)]
    fn find(&self, h: u64, key: Probe<'_>) -> Option<usize> {
        crate::counts::bump(crate::counts::Count::Find);
        if self.small() {
            return self.scan(h, key);
        }
        let mut i = self.buckets[self.slot(h)].head;
        while i != NONE {
            let b = &self.buckets[i as usize];
            if b.h == h {
                match (key, &b.key) {
                    (Probe::Int, None) => return Some(i as usize),
                    (Probe::Str(s), Some(k)) => match same_key(s, k) {
                        Some(true) => return Some(i as usize),
                        Some(false) => {}
                        None => return self.find_bytes(h, s, i),
                    },
                    _ => {}
                }
            }
            i = b.next;
        }
        None
    }

    /// `find` from chain entry `i` on, the string key compared by bytes.
    #[inline(never)]
    fn find_bytes(&self, h: u64, s: &StrHead, mut i: u32) -> Option<usize> {
        while i != NONE {
            let b = &self.buckets[i as usize];
            if b.is(h, Probe::Str(s)) {
                return Some(i as usize);
            }
            i = b.next;
        }
        None
    }

    fn link(&mut self, i: usize) {
        let slot = self.slot(self.buckets[i].h);
        self.buckets[i].next = self.buckets[slot].head;
        self.buckets[slot].head = i as u32;
    }

    /// Make the chains again from the stored hashes (the unset entries
    /// stay unset).
    fn reindex(&mut self) {
        let small = self.small();
        for b in &mut self.buckets {
            b.head = NONE;
        }
        for i in 0..self.used {
            if self.buckets[i].next != UNSET {
                self.buckets[i].next = NONE;
                if !small {
                    self.link(i);
                }
            }
        }
    }

    /// Room for `size` buckets (a power of two): one new allocation, the
    /// entries moved to it at their positions, the chains made again.
    fn resize(&mut self, size: usize) {
        assert!(size < UNSET as usize, "array too large");
        // (Eight to sixteen: a kept buffer, and the eight kept.)
        let mut buckets = take(&BUCKETS, size);
        buckets.extend(self.buckets.drain(..self.used));
        buckets.resize_with(size, Bucket::empty);
        give(&BUCKETS, std::mem::replace(&mut self.buckets, buckets));
        self.reindex();
    }

    /// A small table's position of `key`: one pass over the stored hashes,
    /// the key compared only where the hash matches. An unset entry has no
    /// string key, so only an int key needs the unset mark.
    ///
    /// A leaf, as `find` is: the byte compare of a string key is out of
    /// line (`scan_bytes`), reached from the first entry whose hash
    /// matches and which [`same_key`] cannot decide (a heap string).
    #[inline(always)]
    fn scan(&self, h: u64, key: Probe<'_>) -> Option<usize> {
        let taken = &self.buckets[..self.used];
        match key {
            Probe::Str(s) => {
                for (i, b) in taken.iter().enumerate() {
                    if b.h == h && let Some(k) = &b.key {
                        match same_key(s, k) {
                            Some(true) => return Some(i),
                            Some(false) => {}
                            None => return self.scan_bytes(h, s, i),
                        }
                    }
                }
                None
            }
            Probe::Int => taken.iter().position(|b| b.h == h && b.key.is_none() && b.next != UNSET),
        }
    }

    /// `scan` from position `from` on, the string key compared by bytes.
    #[inline(never)]
    fn scan_bytes(&self, h: u64, s: &StrHead, from: usize) -> Option<usize> {
        let taken = &self.buckets[from..self.used];
        taken.iter().position(|b| b.is(h, Probe::Str(s))).map(|i| from + i)
    }

    /// Set `key` to `value`: in place if present, else last in order. One
    /// pass (a small table's scan, or a large one's chain) with the hash
    /// computed once.
    #[inline(never)]
    fn set(&mut self, key: Key, value: Value) {
        let h = hash_of(&key);
        let found = if self.small() { self.scan(h, key.probe()) } else { self.find(h, key.probe()) };
        match found {
            Some(i) => self.buckets[i].value = value,
            None => {
                let key = match key {
                    Key::Str(s) => Some(s),
                    Key::Int(_) => None,
                };
                self.insert_new(h, key, value);
            }
        }
    }

    /// A new entry, last in order (`key` not present).
    #[inline(always)]
    fn insert_new(&mut self, h: u64, key: Option<PhpStr>, value: Value) {
        if self.used == self.buckets.len() {
            self.grow();
        }
        let i = self.used;
        let b = &mut self.buckets[i];
        b.h = h;
        b.key = key;
        b.value = value;
        b.next = NONE;
        self.used += 1;
        if !self.small() {
            self.link(i);
        }
        self.live += 1;
    }

    /// Double the table (one allocation, as zend_hash_do_resize).
    #[cold]
    #[inline(never)]
    fn grow(&mut self) {
        self.resize(self.buckets.len().max(MIN_SIZE / 2) * 2);
    }

    #[inline(never)]
    fn remove(&mut self, h: u64, key: Probe<'_>) -> Option<Value> {
        let i = if self.small() {
            self.find(h, key)?
        } else {
            // Unlinked from its chain.
            let slot = self.slot(h);
            let (mut prev, mut i) = (NONE, self.buckets[slot].head);
            while i != NONE && !self.buckets[i as usize].is(h, key) {
                prev = i;
                i = self.buckets[i as usize].next;
            }
            if i == NONE {
                return None;
            }
            let next = self.buckets[i as usize].next;
            match prev {
                NONE => self.buckets[slot].head = next,
                p => self.buckets[p as usize].next = next,
            }
            i as usize
        };
        let b = &mut self.buckets[i];
        b.next = UNSET;
        b.key = None;
        let value = std::mem::replace(&mut b.value, Value::NULL);
        self.live -= 1;
        // Compact when more than half the entries are unset.
        if self.len() > SMALL && self.live * 2 < self.len() {
            self.compact();
        }
        Some(value)
    }

    /// Close up the unset entries, keeping the order (positions change),
    /// in place.
    fn compact(&mut self) {
        let mut to = 0;
        for from in 0..self.used {
            if self.buckets[from].next != UNSET {
                self.buckets.swap(to, from);
                to += 1;
            }
        }
        // The positions left are room again (an unset entry holds nothing).
        for b in &mut self.buckets[to..self.used] {
            b.next = NONE;
        }
        self.used = to;
        self.reindex();
    }

    fn get(&self, i: usize) -> Option<&Value> {
        Some(&self.buckets[i].value)
    }

    /// The entries in order, with their positions.
    fn entries(&self) -> impl Iterator<Item = (usize, &Bucket)> {
        self.buckets[..self.used].iter().enumerate().filter(|(_, b)| b.next != UNSET)
    }
}

/// A key's hash as buckets store it.
#[inline]
fn hash_of(key: &Key) -> u64 {
    match key {
        Key::Int(i) => *i as u64,
        Key::Str(s) => s.hash(),
    }
}

#[derive(Debug)]
pub struct PhpArray {
    storage: Storage,
    /// The key `$a[] = v` uses next; None until an int key is used
    /// (PHP 8.3+: after only negative keys it is the largest plus one).
    next_free: Option<i64>,
    /// The internal pointer (current(), next(), reset()): a position.
    cursor: usize,
}

/// A copy (zend_array_dup): an internal pointer past the end starts over
/// at the beginning in the copy.
impl Clone for PhpArray {
    fn clone(&self) -> Self {
        // (An empty array's copy, the first write to a shared `[]`, copies
        // no values: not counted, as PHP makes a new array there.)
        if !self.is_empty() {
            COPIES.with(|c| c.set(c.get() + 1));
        }
        let cursor = if self.cursor < self.end_pos() { self.cursor } else { 0 };
        PhpArray { storage: self.storage.clone(), next_free: self.next_free, cursor }
    }
}

impl Default for PhpArray {
    fn default() -> Self {
        Self::new()
    }
}

impl PhpArray {
    pub fn new() -> Self {
        PhpArray { storage: Storage::Packed(Packed::default()), next_free: None, cursor: 0 }
    }

    /// An empty list with room for `capacity` elements pushed onto it.
    pub fn list_with_capacity(capacity: usize) -> Self {
        let slots = if capacity == 0 { Vec::new() } else { take(&VALUES, capacity) };
        PhpArray { storage: Storage::Packed(Packed { slots, holes: 0 }), next_free: None, cursor: 0 }
    }

    /// An empty array sized for `capacity` keyed entries (array_init_size,
    /// as builtins returning a record make theirs): no growth while it fills.
    pub fn hashed(capacity: usize) -> Self {
        PhpArray { storage: Storage::Hashed(Hashed::with_capacity(capacity)), next_free: None, cursor: 0 }
    }

    /// Whether it is stored packed (int keys by position: HT_IS_PACKED).
    pub fn is_packed(&self) -> bool {
        matches!(self.storage, Storage::Packed(_))
    }

    /// Packed without holes: keys 0..n-1, every one set
    /// (HT_IS_PACKED && HT_IS_WITHOUT_HOLES).
    pub fn is_dense_list(&self) -> bool {
        matches!(&self.storage, Storage::Packed(p) if p.holes == 0)
    }

    /// A list of values (keys 0..n-1).
    pub fn from_list(values: Vec<Value>) -> Self {
        let next_free = (!values.is_empty()).then_some(values.len() as i64);
        PhpArray { storage: Storage::Packed(Packed { slots: values, holes: 0 }), next_free, cursor: 0 }
    }

    /// Room for `additional` more elements, made now (a builtin that
    /// knows how many it adds grows the array once).
    pub fn reserve(&mut self, additional: usize) {
        match &mut self.storage {
            Storage::Packed(p) => p.slots.reserve(additional),
            // The table grown once (one allocation, chains made again).
            Storage::Hashed(h) if h.buckets.len() - h.used < additional => {
                h.resize((h.used + additional).max(MIN_SIZE).next_power_of_two());
            }
            Storage::Hashed(_) => {}
        }
    }

    pub fn len(&self) -> usize {
        match &self.storage {
            Storage::Packed(p) => p.slots.len() - p.holes,
            Storage::Hashed(h) => h.live,
        }
    }

    /// The bytes its table takes: its slots or buckets, room included
    /// (not the values' own allocations, nor the array itself).
    pub fn table_bytes(&self) -> usize {
        match &self.storage {
            Storage::Packed(p) => p.slots.capacity() * std::mem::size_of::<Value>(),
            Storage::Hashed(h) => h.buckets.capacity() * std::mem::size_of::<Bucket>(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the keys are exactly 0..n-1 in order (array_is_list).
    pub fn is_list(&self) -> bool {
        match &self.storage {
            Storage::Packed(p) => p.holes == 0,
            Storage::Hashed(_) => self.iter().enumerate().all(|(i, (k, _))| k == Key::Int(i as i64)),
        }
    }

    /// Out of line, as before the numeric-key test became small: inlined,
    /// LLVM put it into the interpreter's fast loop at five sites, and the
    /// loop's other paths paid for the registers (+9 instructions an
    /// iteration on `$a[2]`).
    #[inline(never)]
    pub fn get(&self, key: &Key) -> Option<&Value> {
        match &self.storage {
            Storage::Packed(p) => match key {
                Key::Int(i) => p.get(*i),
                _ => None,
            },
            Storage::Hashed(h) => h.find(hash_of(key), key.probe()).and_then(|i| h.get(i)),
        }
    }

    /// `get` of the key a string names (`Key::from_string`), with no handle
    /// to it made: what a lookup by a literal (a program-memory string)
    /// or any borrowed string uses.
    #[inline]
    pub fn get_str(&self, s: &StrHead) -> Option<&Value> {
        if let Some(i) = canonical_int(s) {
            return self.get_int(i);
        }
        match &self.storage {
            Storage::Packed(_) => None,
            Storage::Hashed(h) => h.find(s.hash(), Probe::Str(s)).and_then(|i| h.get(i)),
        }
    }

    /// `get_mut` of the key a string names, with no handle to it made
    /// (see [`get_str`](Self::get_str)).
    #[inline]
    pub fn get_str_mut(&mut self, s: &StrHead) -> Option<&mut Value> {
        if let Some(i) = canonical_int(s) {
            return self.get_int_mut(i);
        }
        match &mut self.storage {
            Storage::Packed(_) => None,
            Storage::Hashed(h) => {
                let i = h.find(s.hash(), Probe::Str(s))?;
                Some(&mut h.buckets[i].value)
            }
        }
    }

    /// [`get_str`](Self::get_str) of a string that is not a canonical
    /// integer (a literal key lowering marked `KSTR`), so with no numeric
    /// test: as PHP's handlers look up a CONST dimension the compiler left
    /// a string.
    #[inline]
    pub fn get_str_key(&self, s: &StrHead) -> Option<&Value> {
        debug_assert!(canonical_int(s).is_none(), "get_str_key of an int key");
        match &self.storage {
            Storage::Packed(_) => None,
            Storage::Hashed(h) => h.find(s.hash(), Probe::Str(s)).and_then(|i| h.get(i)),
        }
    }

    /// [`get_str_mut`](Self::get_str_mut) of a string that is not a
    /// canonical integer (see [`get_str_key`](Self::get_str_key)).
    #[inline]
    pub fn get_str_key_mut(&mut self, s: &StrHead) -> Option<&mut Value> {
        debug_assert!(canonical_int(s).is_none(), "get_str_key_mut of an int key");
        match &mut self.storage {
            Storage::Packed(_) => None,
            Storage::Hashed(h) => {
                let i = h.find(s.hash(), Probe::Str(s))?;
                Some(&mut h.buckets[i].value)
            }
        }
    }

    /// `insert` of a string key just looked up and not found (a write's
    /// miss, after [`get_str_mut`](Self::get_str_mut)): appended with no
    /// second lookup, as zend_hash_add_new.
    pub fn insert_absent(&mut self, key: Key, value: Value) {
        debug_assert!(!self.contains_key(&key), "insert_absent of a present key");
        match (&mut self.storage, key) {
            (Storage::Hashed(h), Key::Str(s)) => h.insert_new(s.hash(), Some(s), value),
            (_, key) => self.insert(key, value),
        }
    }

    /// A record a builtin returns (getrusage(), gc_status()): `keys` in
    /// order with `values`, the keys made once (see [`record_keys`]), so
    /// no key is allocated, hashed or looked for here. PHP builds these
    /// with add_assoc_*, which hashes and allocates every key every call.
    pub fn record(keys: &[PhpStr], values: impl IntoIterator<Item = Value>) -> PhpArray {
        let mut h = Hashed::with_capacity(keys.len());
        for (k, v) in keys.iter().zip(values) {
            h.insert_new(k.hash(), Some(k.clone()), v);
        }
        PhpArray { storage: Storage::Hashed(h), next_free: None, cursor: 0 }
    }

    /// `get` for an int key.
    #[inline]
    pub fn get_int(&self, i: i64) -> Option<&Value> {
        match &self.storage {
            Storage::Packed(p) => p.get(i),
            Storage::Hashed(h) => h.find(i as u64, Probe::Int).and_then(|i| h.get(i)),
        }
    }

    /// `get_mut` for an int key.
    #[inline]
    pub fn get_int_mut(&mut self, i: i64) -> Option<&mut Value> {
        match &mut self.storage {
            Storage::Packed(p) => p.get_mut(i),
            Storage::Hashed(h) => {
                let i = h.find(i as u64, Probe::Int)?;
                Some(&mut h.buckets[i].value)
            }
        }
    }

    pub fn get_mut(&mut self, key: &Key) -> Option<&mut Value> {
        match &mut self.storage {
            Storage::Packed(p) => match key {
                Key::Int(i) => p.get_mut(*i),
                _ => None,
            },
            Storage::Hashed(h) => {
                let i = h.find(hash_of(key), key.probe())?;
                Some(&mut h.buckets[i].value)
            }
        }
    }

    pub fn contains_key(&self, key: &Key) -> bool {
        self.get(key).is_some()
    }

    /// The key `$a[] = v` would use, or None when it would overflow
    /// ("Cannot add element to the array as the next element is already occupied").
    pub fn next_key(&self) -> Option<i64> {
        match self.next_free {
            None => Some(0),
            Some(i64::MAX) if self.contains_key(&Key::Int(i64::MAX)) => None,
            Some(n) => Some(n),
        }
    }

    fn note_int_key(&mut self, k: i64) {
        let next = k.saturating_add(1);
        self.next_free = Some(match self.next_free {
            Some(n) if n >= next => n,
            _ => next,
        });
    }

    /// Set `key` to `value` (in place if present, else appended in order).
    pub fn insert(&mut self, key: Key, value: Value) {
        crate::counts::bump(crate::counts::Count::Insert);
        if let Key::Int(k) = key {
            self.note_int_key(k);
        }
        match &mut self.storage {
            Storage::Packed(p) => match key {
                Key::Int(i) if p.get(i).is_some() => p.slots[i as usize] = value,
                Key::Int(i) if p.fits(i) => {
                    let gap = i as usize - p.slots.len();
                    p.room_for(i as usize);
                    p.slots.resize(i as usize, Value::UNDEF);
                    p.holes += gap;
                    p.slots.push(value);
                }
                key => {
                    self.make_hashed();
                    self.insert(key, value);
                }
            },
            Storage::Hashed(h) => h.set(key, value),
        }
    }

    /// The element at `key` to write into, made null first if there is none
    /// (a write through `$a[k][...]`: ZEND_FETCH_DIM_W's slot). One lookup
    /// when it is there.
    pub fn slot_mut(&mut self, key: &Key) -> &mut Value {
        let at = match &self.storage {
            Storage::Packed(p) => match key {
                Key::Int(i) if p.get(*i).is_some() => Some(*i as usize),
                _ => None,
            },
            Storage::Hashed(h) => h.find(hash_of(key), key.probe()),
        };
        let Some(i) = at else {
            self.insert(key.clone(), Value::NULL);
            return self.get_mut(key).expect("just inserted");
        };
        match &mut self.storage {
            Storage::Packed(p) => &mut p.slots[i],
            Storage::Hashed(h) => &mut h.buckets[i].value,
        }
    }

    /// `$a[] = value`; Err when the next key is taken.
    pub fn push(&mut self, value: Value) -> Result<(), NextKeyOccupied> {
        // A list without holes whose next key is its length: the value
        // goes on the end (zend_hash_next_index_insert's packed path,
        // ZEND_HASH_FILL_PACKED). What `insert` of that key does (it fits:
        // no holes), without its key tests.
        if let Storage::Packed(p) = &mut self.storage {
            let n = p.slots.len();
            if p.holes == 0 && self.next_free.unwrap_or(0) == n as i64 {
                if n == p.slots.capacity() {
                    p.room_for(n);
                }
                p.slots.push(value);
                self.next_free = Some(n as i64 + 1);
                return Ok(());
            }
        }
        let key = self.next_key().ok_or(NextKeyOccupied)?;
        self.insert(Key::Int(key), value);
        Ok(())
    }

    /// array_pop: remove the last entry. If its key is the int just below
    /// the next free key, the next free key comes back down to it, as
    /// PHP's array_pop does (so `$a[] = x` reuses it).
    pub fn pop(&mut self) -> Option<Value> {
        let key = self.entry_before(self.end_pos())?.1;
        let value = self.remove(&key)?;
        if let Key::Int(k) = key
            && self.next_free == Some(k.saturating_add(1))
        {
            self.next_free = if k == 0 && self.is_empty() { None } else { Some(k) };
        }
        Some(value)
    }

    /// unset($a[key])
    pub fn remove(&mut self, key: &Key) -> Option<Value> {
        match &mut self.storage {
            // The slot becomes a hole; holes at the end go.
            Storage::Packed(p) => {
                let Key::Int(i) = *key else { return None };
                p.get(i)?;
                let old = std::mem::replace(&mut p.slots[i as usize], Value::UNDEF);
                p.holes += 1;
                while p.slots.last().is_some_and(Value::is_undef) {
                    p.slots.pop();
                    p.holes -= 1;
                }
                Some(old)
            }
            Storage::Hashed(h) => h.remove(hash_of(key), key.probe()),
        }
    }

    fn make_hashed(&mut self) {
        if let Storage::Packed(p) = &mut self.storage {
            // Room for one more: the insert that made it a hash.
            let mut h = Hashed::with_capacity(p.slots.len() - p.holes + 1);
            for (i, value) in std::mem::take(&mut p.slots).into_iter().enumerate() {
                if !value.is_undef() {
                    h.insert_new(i as u64, None, value);
                }
            }
            self.storage = Storage::Hashed(h);
        }
    }

    /// The first entry at or after position `pos` in iteration order, with
    /// its position (foreach walks an array by position).
    pub fn entry_at(&self, pos: usize) -> Option<(usize, Key, &Value)> {
        match &self.storage {
            Storage::Packed(p) => p.slots.get(pos..)?.iter().enumerate().find_map(|(i, x)| {
                (!x.is_undef()).then(|| (pos + i, Key::Int((pos + i) as i64), x))
            }),
            Storage::Hashed(h) => h.buckets[..h.used].get(pos..)?.iter().enumerate().find_map(|(i, b)| {
                (b.next != UNSET).then(|| (pos + i, b.key(), &b.value))
            }),
        }
    }

    /// `entry_at` without the key (no copy of a string key's handle): the
    /// position and value of the first entry at or after `pos`.
    #[inline]
    pub fn value_at(&self, pos: usize) -> Option<(usize, &Value)> {
        match &self.storage {
            Storage::Packed(p) => p.slots.get(pos..)?.iter().enumerate().find_map(|(i, x)| (!x.is_undef()).then_some((pos + i, x))),
            Storage::Hashed(h) => h.buckets[..h.used].get(pos..)?.iter().enumerate().find_map(|(i, b)| (b.next != UNSET).then_some((pos + i, &b.value))),
        }
    }

    /// What position `pos` itself holds, with no walk: the value there, a
    /// hole (an unset entry: the next entry is `value_at`'s to find), or
    /// the end. foreach's step in the interpreter's loop reads this: one
    /// bound and one test, and nothing copied.
    #[inline(always)]
    pub fn value_here(&self, pos: usize) -> Here<&Value> {
        match &self.storage {
            Storage::Packed(p) => match p.slots.get(pos) {
                Some(x) if x.is_undef() => Here::Hole,
                Some(x) => Here::Entry(x),
                None => Here::End,
            },
            Storage::Hashed(h) => match h.buckets.get(pos) {
                Some(b) if pos < h.used => {
                    if b.next == UNSET { Here::Hole } else { Here::Entry(&b.value) }
                }
                _ => Here::End,
            },
        }
    }

    /// `value_here` with the key, read in place (a string key's handle is
    /// not copied: the reader copies it once, into the value the loop's
    /// key variable gets).
    #[inline(always)]
    pub fn entry_here(&self, pos: usize) -> EntryHere<'_> {
        match &self.storage {
            Storage::Packed(p) => match p.slots.get(pos) {
                Some(x) if x.is_undef() => EntryHere::Hole,
                Some(x) => EntryHere::Int(pos as i64, x),
                None => EntryHere::End,
            },
            Storage::Hashed(h) => match h.buckets.get(pos) {
                Some(b) if pos < h.used => match &b.key {
                    _ if b.next == UNSET => EntryHere::Hole,
                    Some(s) => EntryHere::Str(s, &b.value),
                    None => EntryHere::Int(b.h as i64, &b.value),
                },
                _ => EntryHere::End,
            },
        }
    }

    /// The last entry before position `pos`.
    pub fn entry_before(&self, pos: usize) -> Option<(usize, Key, &Value)> {
        match &self.storage {
            Storage::Packed(p) => p.slots.get(..pos.min(p.slots.len()))?.iter().enumerate().rev().find_map(|(i, x)| {
                (!x.is_undef()).then_some((i, Key::Int(i as i64), x))
            }),
            Storage::Hashed(h) => h.buckets.get(..pos.min(h.len()))?.iter().enumerate().rev().find_map(|(i, b)| {
                (b.next != UNSET).then(|| (i, b.key(), &b.value))
            }),
        }
    }

    /// One past the last position.
    pub fn end_pos(&self) -> usize {
        match &self.storage {
            Storage::Packed(p) => p.slots.len(),
            Storage::Hashed(h) => h.len(),
        }
    }

    /// The internal pointer's position.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn set_cursor(&mut self, pos: usize) {
        self.cursor = pos;
    }

    /// Entries in order.
    pub fn iter(&self) -> impl Iterator<Item = (Key, &Value)> {
        let (packed, hashed) = match &self.storage {
            Storage::Packed(p) => (Some(&p.slots), None),
            Storage::Hashed(h) => (None, Some(h)),
        };
        let packed = packed.into_iter().flat_map(|v| {
            v.iter().enumerate().filter(|(_, x)| !x.is_undef()).map(|(i, x)| (Key::Int(i as i64), x))
        });
        let hashed = hashed
            .into_iter()
            .flat_map(|h| h.entries().map(|(_, b)| (b.key(), &b.value)));
        packed.chain(hashed)
    }

    pub fn values(&self) -> impl Iterator<Item = &Value> {
        self.iter().map(|(_, v)| v)
    }

    /// `===`: the same pairs in the same order, identical values.
    pub fn identical(&self, other: &PhpArray) -> bool {
        self.len() == other.len()
            && self.iter().zip(other.iter()).all(|((ka, va), (kb, vb))| ka == kb && va.identical(vb))
    }

    /// zend_compare_arrays: by count, then by the left's keys in order;
    /// None when a key of the left is missing on the right.
    pub fn compare(&self, other: &PhpArray) -> Option<Ordering> {
        match self.len().cmp(&other.len()) {
            Ordering::Equal => {}
            unequal => return Some(unequal),
        }
        for (key, value) in self.iter() {
            let theirs = other.get(&key)?;
            match value.compare(theirs)? {
                Ordering::Equal => {}
                unequal => return Some(unequal),
            }
        }
        Some(Ordering::Equal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::string::IStr;

    fn int(i: i64) -> Value {
        Value::int(i)
    }

    #[test]
    fn keys_normalize_like_php() {
        let k = |s: &str| Key::from_string(PhpStr::from(s));
        assert_eq!(k("123"), Key::Int(123));
        assert_eq!(k("-5"), Key::Int(-5));
        assert_eq!(k("0"), Key::Int(0));
        assert_eq!(k("012"), Key::Str(PhpStr::from("012")));
        assert_eq!(k("-0"), Key::Str(PhpStr::from("-0")));
        assert_eq!(k("9223372036854775808"), Key::Str(PhpStr::from("9223372036854775808")));
        assert_eq!(k("9223372036854775807"), Key::Int(i64::MAX));
        assert_eq!(k("-9223372036854775808"), Key::Int(i64::MIN));
        for s in ["", "-", "-01", "00", "08", "+1", " 1", "1 ", "1a", "a1", "--1", "1.0", "0x1", "\0", "-9223372036854775809", "18446744073709551616", "12345678901234567890"] {
            assert_eq!(k(s), Key::Str(PhpStr::from(s)), "{s:?}");
            assert_eq!(int_key(s.as_bytes()), None, "{s:?}");
        }
        for (s, i) in [("0", 0), ("-12", -12), ("9223372036854775807", i64::MAX), ("-9223372036854775808", i64::MIN)] {
            assert_eq!(int_key(s.as_bytes()), Some(i), "{s:?}");
        }
        // A key on the heap (not a short string) the same.
        assert_eq!(Key::from_string(PhpStr::new(b"42".to_vec())), Key::Int(42));
        assert_eq!(Key::from_string(PhpStr::new(b"042".to_vec())), Key::Str(PhpStr::from("042")));
        assert_eq!(Key::from_value(&Value::float(1.5)), Ok((Key::Int(1), Some(KeyIssue::LosesPrecision(1.5)))));
        assert_eq!(Key::from_value(&Value::TRUE), Ok((Key::Int(1), None)));
        assert_eq!(Key::from_value(&Value::NULL), Ok((Key::Str(PhpStr::empty()), Some(KeyIssue::NullOffset))));
    }

    #[test]
    fn lists_stay_packed_and_order_is_kept() {
        let mut a = PhpArray::new();
        a.push(int(1)).unwrap();
        a.push(int(2)).unwrap();
        assert!(matches!(a.storage, Storage::Packed(_)));
        a.insert(Key::Str(PhpStr::from("x")), int(3));
        a.insert(Key::Int(0), int(9));
        let keys: Vec<Key> = a.iter().map(|(k, _)| k).collect();
        assert_eq!(keys, vec![Key::Int(0), Key::Int(1), Key::Str(PhpStr::from("x"))]);
        assert_eq!(a.get(&Key::Int(0)).map(Value::to_int), Some(9));
    }

    /// A string key is found from another handle of the same bytes (the
    /// byte compare `find` and `scan` hand off), in a scanned table and a
    /// chained one, by every lookup and by `set`; other bytes find nothing.
    #[test]
    fn string_keys_found_from_another_handle() {
        let got = |v: Option<&Value>| v.and_then(Value::as_int);
        for n in [4usize, 40] {
            let mut a = PhpArray::new();
            for i in 0..n {
                a.insert(Key::Str(PhpStr::from(format!("key{i}"))), int(i as i64));
            }
            for i in 0..n {
                let other = PhpStr::from(format!("key{i}"));
                assert_eq!(got(a.get(&Key::Str(other.clone()))), Some(i as i64), "get, {n} keys");
                assert_eq!(got(a.get_str(&other)), Some(i as i64), "get_str, {n} keys");
                assert!(a.get_mut(&Key::Str(other.clone())).is_some(), "get_mut, {n} keys");
                assert_eq!(got(a.get(&Key::Str(PhpStr::from(format!("key{i}x"))))), None, "a miss, {n} keys");
            }
            // A heap handle of a short key's bytes (compared as slices),
            // and a short probe of a key stored on the heap.
            let heap = |s: &str| PhpStr::new(s.as_bytes().to_vec());
            for i in 0..n {
                assert_eq!(got(a.get_str(&heap(&format!("key{i}")))), Some(i as i64), "heap probe, {n} keys");
            }
            a.insert(Key::Str(heap("on the heap")), int(-2));
            assert_eq!(got(a.get_str(&PhpStr::from("on the heap"))), Some(-2));
            assert_eq!(got(a.get_str(&PhpStr::from("on the hea"))), None);
            a.remove(&Key::Str(PhpStr::from("on the heap")));
            // set() finds the entry too: no second one is made.
            a.insert(Key::Str(PhpStr::from("key1")), int(-1));
            assert_eq!(a.len(), n);
            assert_eq!(got(a.get_str(&PhpStr::from("key1"))), Some(-1));
            // A program-memory key (one head): found by its head, and by its bytes.
            let i = IStr::intern(b"istr key").expect("room in the pool");
            a.insert(Key::Str(i.to_php_str()), int(7));
            assert_eq!(got(a.get_str(i)), Some(7));
            assert_eq!(got(a.get_str(&PhpStr::from("istr key"))), Some(7));
        }
    }

    #[test]
    fn next_free_follows_php_8_3() {
        let mut a = PhpArray::new();
        a.insert(Key::Int(-5), int(1));
        a.push(int(2)).unwrap();
        assert!(a.contains_key(&Key::Int(-4)));
        let mut b = PhpArray::from_list(vec![int(1), int(2)]);
        b.remove(&Key::Int(1));
        b.push(int(3)).unwrap();
        // the removed key is not reused
        assert!(b.contains_key(&Key::Int(2)) && !b.contains_key(&Key::Int(1)));
        let mut c = PhpArray::new();
        c.insert(Key::Int(i64::MAX), int(1));
        assert_eq!(c.push(int(2)), Err(NextKeyOccupied));
    }

    #[test]
    fn unset_is_order_preserving_and_compacts() {
        let mut a = PhpArray::new();
        for i in 0..100 {
            a.insert(Key::Str(PhpStr::from(format!("k{i}"))), int(i));
        }
        for i in 0..90 {
            a.remove(&Key::Str(PhpStr::from(format!("k{i}"))));
        }
        let values: Vec<i64> = a.values().map(Value::to_int).collect();
        assert_eq!(values, (90..100).collect::<Vec<_>>());
        assert_eq!(a.len(), 10);
    }

    #[test]
    fn int_keys_in_order_stay_packed_with_holes() {
        // [1 => .., 2 => .., ..]: packed, a hole at 0.
        let mut a = PhpArray::new();
        for i in 1..=5 {
            a.insert(Key::Int(i), int(i * 10));
        }
        assert!(matches!(a.storage, Storage::Packed(_)));
        assert_eq!((a.len(), a.get_int(0).is_none(), a.get_int(3).map(Value::to_int)), (5, true, Some(30)));
        assert!(!a.is_list());
        // unset leaves a hole; order and count follow.
        a.remove(&Key::Int(3));
        let keys: Vec<Key> = a.iter().map(|(k, _)| k).collect();
        assert_eq!(keys, [1, 2, 4, 5].map(Key::Int));
        assert_eq!(a.entry_at(2).map(|(p, k, _)| (p, k)), Some((2, Key::Int(2))));
        assert_eq!(a.entry_at(3).map(|(p, k, _)| (p, k)), Some((4, Key::Int(4))));
        // Filling the hole puts key 3 last, as PHP does: a hash now.
        a.insert(Key::Int(3), int(33));
        assert!(matches!(a.storage, Storage::Hashed(_)));
        let keys: Vec<Key> = a.iter().map(|(k, _)| k).collect();
        assert_eq!(keys, [1, 2, 4, 5, 3].map(Key::Int));
        // A far-off key does not leave a long run of holes.
        let mut b = PhpArray::from_list(vec![int(1)]);
        b.insert(Key::Int(1000), int(2));
        assert!(matches!(b.storage, Storage::Hashed(_)));
    }

    #[test]
    fn an_int_key_equal_to_a_string_keys_hash_is_another_key() {
        // Buckets compare the stored hash first: an int key is its own
        // hash, so it must not match a string whose hash is that int.
        let s = PhpStr::from("collides");
        let as_int = s.hash() as i64;
        let mut a = PhpArray::hashed(0);
        a.insert(Key::Str(s.clone()), int(1));
        a.insert(Key::Int(as_int), int(2));
        assert_eq!(a.len(), 2);
        assert_eq!(a.get(&Key::Str(s.clone())).map(Value::to_int), Some(1));
        assert_eq!(a.get_int(as_int).map(Value::to_int), Some(2));
        a.remove(&Key::Int(as_int));
        assert_eq!((a.len(), a.get(&Key::Str(s)).map(Value::to_int), a.get_int(as_int).is_none()), (1, Some(1), true));
    }

    #[test]
    fn a_hash_grows_unsets_and_compacts_keeping_order_and_chains() {
        let mut a = PhpArray::new();
        for i in 0..1000 {
            a.insert(Key::Str(PhpStr::from(format!("s{i}"))), int(i));
            a.insert(Key::Int(i * 7919), int(-i));
        }
        assert_eq!(a.len(), 2000);
        for i in (0..1000).step_by(3) {
            assert_eq!(a.remove(&Key::Str(PhpStr::from(format!("s{i}")))).map(|v| v.to_int()), Some(i));
            assert_eq!(a.remove(&Key::Int(i * 7919)).map(|v| v.to_int()), Some(-i));
        }
        for i in 0..1000 {
            let kept = i % 3 != 0;
            assert_eq!(a.get(&Key::Str(PhpStr::from(format!("s{i}")))).is_some(), kept, "s{i}");
            assert_eq!(a.get_int(i * 7919).is_some(), kept, "{i}");
        }
        let order: Vec<i64> = a.values().map(Value::to_int).collect();
        let expected: Vec<i64> = (0..1000).filter(|i| i % 3 != 0).flat_map(|i| [i, -i]).collect();
        assert_eq!(order, expected);
        // Unsetting nearly all compacts: positions close up, order stays.
        let keys: Vec<Key> = a.iter().map(|(k, _)| k).collect();
        for k in &keys[..keys.len() - 3] {
            a.remove(k);
        }
        let end = a.end_pos();
        assert!(end <= 8, "{end} positions for 3 entries");
        assert_eq!(a.iter().map(|(k, _)| k).collect::<Vec<_>>(), keys[keys.len() - 3..]);
        a.insert(Key::Str(PhpStr::from("s0")), int(0));
        assert_eq!(a.entry_at(end).map(|(p, k, _)| (p, k)), Some((end, Key::Str(PhpStr::from("s0")))));
    }

    #[test]
    fn a_copy_restarts_a_pointer_past_the_end() {
        // zend_array_dup: nInternalPointer past nNumUsed becomes 0.
        let mut a = PhpArray::from_list(vec![int(1), int(2)]);
        a.set_cursor(a.end_pos());
        assert_eq!(a.clone().cursor(), 0);
        a.set_cursor(1);
        assert_eq!(a.clone().cursor(), 1);
    }

    #[test]
    fn an_undef_value_is_an_entry_like_any_other() {
        // get_class_vars() lists an uninitialized typed static as Undef:
        // the entry must stay, in small and large tables, through growth,
        // copies, unsets around it and compaction.
        for n in [3, 8, 9, 40] {
            let mut a = PhpArray::new();
            for i in 0..n {
                let v = if i % 3 == 0 { Value::UNDEF } else { int(i) };
                a.insert(Key::Str(PhpStr::from(format!("p{i}"))), v);
            }
            let count = |a: &PhpArray| a.iter().count();
            assert_eq!((a.len(), count(&a)), (n as usize, n as usize), "{n}");
            assert!(a.get(&Key::Str(PhpStr::from("p0"))).is_some_and(Value::is_undef), "{n}");
            let b = a.clone();
            assert_eq!(count(&b), n as usize);
            for i in (1..n).filter(|i| i % 3 != 0) {
                a.remove(&Key::Str(PhpStr::from(format!("p{i}"))));
            }
            let left: Vec<Key> = a.iter().map(|(k, _)| k).collect();
            let undefs: Vec<Key> = (0..n).filter(|i| i % 3 == 0).map(|i| Key::Str(PhpStr::from(format!("p{i}")))).collect();
            assert_eq!(left, undefs, "{n}");
            assert_eq!(a.len(), undefs.len());
            a.insert(Key::Str(PhpStr::from("after")), Value::UNDEF);
            assert_eq!(a.iter().last().map(|(k, _)| k), Some(Key::Str(PhpStr::from("after"))));
            assert_eq!(count(&b), n as usize, "the copy is untouched");
        }
    }

    #[test]
    fn copies_share_until_written() {
        let a = std::rc::Rc::new(PhpArray::from_list(vec![int(1)]));
        let mut b = a.clone();
        std::rc::Rc::make_mut(&mut b).push(int(2)).unwrap();
        assert_eq!((a.len(), b.len()), (1, 2));
    }

    #[test]
    fn a_table_is_one_allocation_that_doubles() {
        let mut a = PhpArray::hashed(0);
        let table = |a: &PhpArray| match &a.storage {
            Storage::Hashed(h) => (h.buckets.len(), h.buckets.capacity(), h.used),
            _ => unreachable!(),
        };
        assert_eq!(table(&a), (8, 8, 0));
        for i in 0..9 {
            a.insert(Key::Str(PhpStr::from(format!("k{i}"))), int(i));
            let (size, capacity, used) = table(&a);
            // The buckets are the whole table: its room is exactly its size.
            assert_eq!((size, capacity, used), (if i < 8 { 8 } else { 16 }, size, i as usize + 1));
        }
        // Chained now (past SMALL): every key found, an unset one not.
        a.remove(&Key::Str(PhpStr::from("k3")));
        for i in 0..9 {
            assert_eq!(a.get(&Key::Str(PhpStr::from(format!("k{i}")))).is_some(), i != 3);
        }
        let copy = a.clone();
        assert_eq!(table(&copy), table(&a));
        let keys: Vec<Key> = copy.iter().map(|(k, _)| k).collect();
        assert_eq!(keys.len(), 8);
        assert_eq!(keys[2], Key::Str(PhpStr::from("k2")));
        assert_eq!(keys[3], Key::Str(PhpStr::from("k4")));
    }

    #[test]
    fn freed_small_buffers_are_reused_empty() {
        let buffer = |a: &PhpArray| match &a.storage {
            Storage::Packed(p) => (p.slots.as_ptr() as usize, p.slots.capacity()),
            Storage::Hashed(h) => (h.buckets.as_ptr() as usize, h.buckets.capacity()),
        };
        // A list: its buffer of eight is the next small list's.
        let mut a = PhpArray::new();
        for i in 0..5 {
            a.push(int(i)).unwrap();
        }
        let (at, room) = buffer(&a);
        assert_eq!(room, 8);
        drop(a);
        let b = PhpArray::list_with_capacity(3);
        assert_eq!(buffer(&b), (at, 8));
        assert_eq!(b.len(), 0);
        // Growing to sixteen takes a kept one and keeps the eight.
        let mut c = PhpArray::list_with_capacity(8);
        for i in 0..9 {
            c.push(int(i)).unwrap();
        }
        assert_eq!(buffer(&c).1, 16);
        assert!(c.iter().map(|(_, v)| v.to_int()).eq(0..9));
        // A table the same: the next table's, its old entries gone.
        let mut h = PhpArray::new();
        h.insert(Key::Str(PhpStr::from("k")), int(1));
        let (at, room) = buffer(&h);
        assert_eq!(room, 8);
        drop(h);
        let mut h2 = PhpArray::new();
        h2.insert(Key::Str(PhpStr::from("other")), int(2));
        assert_eq!(buffer(&h2), (at, 8));
        assert_eq!(h2.len(), 1);
        assert!(h2.get(&Key::Str(PhpStr::from("k"))).is_none());
        // A table growing past eight keeps its eight for the next.
        let mut g = PhpArray::new();
        for i in 0..9 {
            g.insert(Key::Str(PhpStr::from(format!("g{i}").as_str())), int(i));
        }
        assert_eq!(buffer(&g).1, 16);
        assert!(g.iter().map(|(_, v)| v.to_int()).eq(0..9));
    }

    #[test]
    fn a_position_holds_an_entry_a_hole_or_nothing() {
        // A list with its middle unset: the hole is said, not walked past.
        let mut a = PhpArray::from_list(vec![int(10), int(20), int(30)]);
        a.remove(&Key::Int(1));
        assert!(matches!(a.storage, Storage::Packed(_)));
        fn of<T, U>(here: Here<T>, f: impl FnOnce(T) -> U) -> Here<U> {
            match here {
                Here::Entry(x) => Here::Entry(f(x)),
                Here::Hole => Here::Hole,
                Here::End => Here::End,
            }
        }
        fn keyed(here: EntryHere<'_>) -> Here<(Key, i64)> {
            match here {
                EntryHere::Int(i, v) => Here::Entry((Key::Int(i), v.to_int())),
                EntryHere::Str(s, v) => Here::Entry((Key::Str(s.clone()), v.to_int())),
                EntryHere::Hole => Here::Hole,
                EntryHere::End => Here::End,
            }
        }
        let values = |a: &PhpArray, n: usize| (0..n).map(|p| of(a.value_here(p), Value::to_int)).collect::<Vec<_>>();
        let keys = |a: &PhpArray, n: usize| (0..n).map(|p| keyed(a.entry_here(p))).collect::<Vec<_>>();
        assert_eq!(values(&a, 5), [Here::Entry(10), Here::Hole, Here::Entry(30), Here::End, Here::End]);
        assert_eq!(keys(&a, 4), [Here::Entry((Key::Int(0), 10)), Here::Hole, Here::Entry((Key::Int(2), 30)), Here::End]);
        assert_eq!(of(a.value_here(usize::MAX), Value::to_int), Here::End);
        // A hashed array: string and int keys, an unset entry in the
        // middle and one at the end (a position still taken), then room.
        let mut h = PhpArray::new();
        h.insert(Key::Str(PhpStr::from("a")), int(1));
        h.insert(Key::Str(PhpStr::from("b")), int(2));
        h.insert(Key::Int(-7), int(3));
        h.insert(Key::Str(PhpStr::from("c")), int(4));
        h.remove(&Key::Str(PhpStr::from("b")));
        h.remove(&Key::Str(PhpStr::from("c")));
        assert!(matches!(h.storage, Storage::Hashed(_)));
        assert_eq!(values(&h, 6), [Here::Entry(1), Here::Hole, Here::Entry(3), Here::Hole, Here::End, Here::End]);
        assert_eq!(
            keys(&h, 5),
            [Here::Entry((Key::Str(PhpStr::from("a")), 1)), Here::Hole, Here::Entry((Key::Int(-7), 3)), Here::Hole, Here::End]
        );
        // The key read in place is the array's own string, no handle
        // of it made by the read.
        let EntryHere::Str(k, _) = h.entry_here(0) else { panic!("a string key") };
        let before = k.count();
        let EntryHere::Str(again, _) = h.entry_here(0) else { panic!("a string key") };
        assert_eq!((again.count(), again.as_bytes()), (before, &b"a"[..]));
        assert!(matches!((PhpArray::new().value_here(0), PhpArray::new().entry_here(0)), (Here::End, EntryHere::End)));
    }
}
