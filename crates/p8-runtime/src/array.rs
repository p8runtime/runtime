//! PHP arrays: ordered maps with int and string keys. A list (keys 0..n-1
//! in order) is stored packed as a plain vector; anything else is an
//! ordered hash with tombstones, so unset is O(1) as in PHP and order is
//! kept. Arrays are values: `Rc<PhpArray>` is shared until written
//! (`Rc::make_mut`). Key rules follow zend_hash.c / zend_operators.c.

use crate::string::PhpStr;
use crate::value::Value;
use crate::number::dval_to_lval;
use std::cmp::Ordering;

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
}

impl Key {
    /// The key a string names: a canonical decimal integer is an int key
    /// (ZEND_HANDLE_NUMERIC_STR: no leading zeros, no "-0", fits in i64).
    pub fn from_string(s: PhpStr) -> Key {
        match canonical_int(s.as_bytes()) {
            Some(i) => Key::Int(i),
            None => Key::Str(s),
        }
    }

    /// The key a value names (null is "", bools and floats are ints).
    pub fn from_value(value: &Value) -> Result<(Key, Option<KeyIssue>), KeyIssue> {
        Ok(match value {
            Value::Int(i) => (Key::Int(*i), None),
            Value::Str(s) => (Key::from_string(s.clone()), None),
            Value::Null | Value::Undef => (Key::Str(PhpStr::empty()), None),
            Value::Bool(b) => (Key::Int(i64::from(*b)), None),
            Value::Float(f) => {
                let issue = (f.fract() != 0.0 || !f.is_finite()).then_some(KeyIssue::LosesPrecision(*f));
                (Key::Int(dval_to_lval(*f)), issue)
            }
            Value::Array(_) => return Err(KeyIssue::Illegal("array")),
            Value::Object(_) => return Err(KeyIssue::Illegal("object")),
            Value::Ref(cell) => return Key::from_value(&cell.borrow()),
        })
    }

    /// The key as a value, without a copy of a string key's handle.
    pub fn into_value(self) -> Value {
        match self {
            Key::Int(i) => Value::Int(i),
            Key::Str(s) => Value::Str(s),
        }
    }

    /// The key as a value (what foreach and array_keys give).
    pub fn to_value(&self) -> Value {
        match self {
            Key::Int(i) => Value::Int(*i),
            Key::Str(s) => Value::Str(s.clone()),
        }
    }
}

/// A canonical decimal integer: "0", "-12", not "012", "-0", "+1", " 1".
fn canonical_int(b: &[u8]) -> Option<i64> {
    let digits = b.strip_prefix(b"-").unwrap_or(b);
    if digits.is_empty() || digits.len() > 20 || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    if digits[0] == b'0' && (digits.len() > 1 || b.len() > 1) {
        return None;
    }
    std::str::from_utf8(b).ok()?.parse().ok()
}

/// `$a[] = v` when the next key would overflow: "Cannot add element to the
/// array as the next element is already occupied".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NextKeyOccupied;

#[derive(Debug, Clone)]
enum Storage {
    /// Int keys in increasing order, each at its own index, as PHP's packed
    /// arrays: `slots[k]` is key `k`, `Value::Undef` where there is no such
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
        match usize::try_from(i).ok().and_then(|i| self.slots.get(i)) {
            Some(Value::Undef) | None => None,
            v => v,
        }
    }

    #[inline]
    fn get_mut(&mut self, i: i64) -> Option<&mut Value> {
        match usize::try_from(i).ok().and_then(|i| self.slots.get_mut(i)) {
            Some(Value::Undef) | None => None,
            v => v,
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
/// A small table (up to SMALL buckets: most arrays, records of a few keys)
/// has no index: a lookup scans its buckets' hashes, and the table is one
/// allocation of 128 or 256 bytes. A larger one has an index of chains
/// (`u32`s: slot heads, then each bucket's next).
///
/// An unset entry stays in place (positions do not move) until the table
/// is compacted. Which entries are unset is kept beside the buckets, not in
/// them: any value, Undef included (an uninitialized typed static property
/// in get_class_vars()), may be an entry's.
#[derive(Debug, Default)]
struct Hashed {
    /// The entries in order (positions are indices), unset ones included.
    buckets: Vec<Bucket>,
    /// Empty for a small table; else `slots` chain heads, then one `next`
    /// per bucket of capacity (UNSET for an unset entry).
    index: Vec<u32>,
    /// How many chain heads (a power of two; 0 for a small table).
    slots: usize,
    /// A small table's unset entries, one bit per position.
    holes: u64,
    live: usize,
}

/// No bucket (the end of a chain, an empty slot).
const NONE: u32 = u32::MAX;
/// An unset entry's `next` in a large table's index.
const UNSET: u32 = u32::MAX - 1;
/// The first table's size.
const MIN_SIZE: usize = 4;
/// Up to this many buckets a table has no index (scanned).
const SMALL: usize = 8;

#[derive(Debug)]
struct Bucket {
    /// The key's hash: an int key itself, a string key's string hash
    /// (PHP's `h`).
    h: u64,
    /// A string key; None for an int key (which `h` is).
    key: Option<PhpStr>,
    value: Value,
}

impl Bucket {
    fn key(&self) -> Key {
        match &self.key {
            Some(s) => Key::Str(s.clone()),
            None => Key::Int(self.h as i64),
        }
    }

    #[inline]
    fn is(&self, h: u64, key: &Key) -> bool {
        self.h == h
            && match (key, &self.key) {
                (Key::Int(_), None) => true,
                (Key::Str(s), Some(k)) => PhpStr::ptr_eq(s, k) || s.as_bytes() == k.as_bytes(),
                _ => false,
            }
    }
}

/// An element as a copy of its array holds it (zend_array_dup), and a
/// property as a clone holds it (zval_add_ref): a reference nothing else
/// holds (its variable gone) is a plain value.
#[inline]
pub(crate) fn dup(v: &Value) -> Value {
    match v {
        Value::Ref(cell) if std::rc::Rc::strong_count(cell) == 1 => cell.borrow().clone(),
        v => v.clone(),
    }
}

impl Clone for Packed {
    fn clone(&self) -> Self {
        Packed { slots: self.slots.iter().map(dup).collect(), holes: self.holes }
    }
}

/// zend_array_dup: the buckets copied into one allocation of their size
/// (and a large table's index copied as it is).
impl Clone for Hashed {
    fn clone(&self) -> Self {
        let buckets = self
            .buckets
            .iter()
            .map(|b| Bucket { h: b.h, key: b.key.clone(), value: dup(&b.value) })
            .collect();
        Hashed { buckets, index: self.index.clone(), slots: self.slots, holes: self.holes, live: self.live }
    }
}

impl Hashed {
    /// Empty, with room for `n` entries before it grows.
    fn with_capacity(n: usize) -> Hashed {
        let mut h = Hashed { buckets: Vec::with_capacity(n.max(MIN_SIZE)), ..Hashed::default() };
        h.reindex(&[]);
        h
    }

    /// Whether the entry at position `i` is set (not unset).
    #[inline]
    fn is_set(&self, i: usize) -> bool {
        if self.slots == 0 { self.holes >> i & 1 == 0 } else { self.index[self.slots + i] != UNSET }
    }

    /// The positions of the unset entries.
    fn unset_positions(&self) -> Vec<usize> {
        (0..self.len()).filter(|&i| !self.is_set(i)).collect()
    }

    /// Room to the buckets' capacity. Positions are their indices.
    fn len(&self) -> usize {
        self.buckets.len()
    }

    /// The hash slot of `h` (Fibonacci hashing: sequential ints spread).
    #[inline]
    fn slot(&self, h: u64) -> usize {
        (h.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> (64 - self.slots.trailing_zeros())) as usize
    }

    /// The position of `key` (its hash `h`). Out of line: the interpreter's
    /// fast loop inlines array reads, and only their packed part belongs
    /// there.
    #[inline(never)]
    fn find(&self, h: u64, key: &Key) -> Option<usize> {
        if self.slots == 0 {
            return self.scan(h, key);
        }
        let mut i = self.index[self.slot(h)];
        while i != NONE {
            let b = &self.buckets[i as usize];
            if b.is(h, key) {
                return Some(i as usize);
            }
            i = self.index[self.slots + i as usize];
        }
        None
    }

    fn link(&mut self, i: usize) {
        let slot = self.slot(self.buckets[i].h);
        self.index[self.slots + i] = self.index[slot];
        self.index[slot] = i as u32;
    }

    /// Build the index for the buckets' capacity (none when small), from
    /// the stored hashes, with the entries at `unset` (ascending) unset.
    fn reindex(&mut self, unset: &[usize]) {
        let capacity = self.buckets.capacity();
        if capacity <= SMALL {
            self.index = Vec::new();
            self.slots = 0;
            self.holes = unset.iter().fold(0, |m, &i| m | 1 << i);
            return;
        }
        assert!(capacity < UNSET as usize, "array too large");
        self.holes = 0;
        self.slots = capacity.next_power_of_two();
        self.index.clear();
        self.index.resize(self.slots + capacity, NONE);
        let mut unset = unset.iter().peekable();
        for i in 0..self.buckets.len() {
            if unset.next_if_eq(&&i).is_some() {
                self.index[self.slots + i] = UNSET;
            } else {
                self.link(i);
            }
        }
    }

    /// A small table's position of `key`: one pass over the stored hashes,
    /// the key compared only where the hash matches. An unset entry has no
    /// string key, so only an int key needs the unset bits.
    #[inline(always)]
    fn scan(&self, h: u64, key: &Key) -> Option<usize> {
        match key {
            Key::Str(s) => self.buckets.iter().position(|b| b.h == h && b.key.as_ref().is_some_and(|k| PhpStr::ptr_eq(s, k) || s.as_bytes() == k.as_bytes())),
            Key::Int(_) => {
                let holes = self.holes;
                self.buckets.iter().enumerate().position(|(i, b)| b.h == h && b.key.is_none() && holes >> i & 1 == 0)
            }
        }
    }

    /// Set `key` to `value`: in place if present, else last in order. One
    /// pass (a small table's scan, or a large one's chain) with the hash
    /// computed once.
    #[inline(never)]
    fn set(&mut self, key: Key, value: Value) {
        let h = hash_of(&key);
        let found = if self.slots == 0 { self.scan(h, &key) } else { self.find(h, &key) };
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
        if self.buckets.len() == self.buckets.capacity() {
            self.grow();
        }
        let i = self.buckets.len();
        self.buckets.push(Bucket { h, key, value });
        if self.slots != 0 {
            self.link(i);
        }
        self.live += 1;
    }

    /// Double the buckets (an index too, once past SMALL).
    #[cold]
    #[inline(never)]
    fn grow(&mut self) {
        let more = self.buckets.capacity().max(MIN_SIZE);
        let unset = self.unset_positions();
        self.buckets.reserve_exact(more);
        self.reindex(&unset);
    }

    #[inline(never)]
    fn remove(&mut self, h: u64, key: &Key) -> Option<Value> {
        let i = if self.slots == 0 {
            self.find(h, key)?
        } else {
            // Unlinked from its chain.
            let slot = self.slot(h);
            let (mut prev, mut i) = (NONE, self.index[slot]);
            while i != NONE && !self.buckets[i as usize].is(h, key) {
                prev = i;
                i = self.index[self.slots + i as usize];
            }
            if i == NONE {
                return None;
            }
            let next = self.index[self.slots + i as usize];
            match prev {
                NONE => self.index[slot] = next,
                p => self.index[self.slots + p as usize] = next,
            }
            self.index[self.slots + i as usize] = UNSET;
            i as usize
        };
        if self.slots == 0 {
            self.holes |= 1 << i;
        }
        let b = &mut self.buckets[i];
        b.key = None;
        let value = std::mem::replace(&mut b.value, Value::Null);
        self.live -= 1;
        // Compact when more than half the entries are unset.
        if self.len() > SMALL && self.live * 2 < self.len() {
            self.compact();
        }
        Some(value)
    }

    /// Close up the unset entries, keeping the order (positions change).
    fn compact(&mut self) {
        let set: Vec<bool> = (0..self.len()).map(|i| self.is_set(i)).collect();
        let mut keep = set.iter();
        self.buckets.retain(|_| keep.next().copied().unwrap_or(false));
        self.reindex(&[]);
    }

    fn get(&self, i: usize) -> Option<&Value> {
        Some(&self.buckets[i].value)
    }

    /// The entries in order, with their positions.
    fn entries(&self) -> impl Iterator<Item = (usize, &Bucket)> {
        self.buckets.iter().enumerate().filter(|(i, _)| self.is_set(*i))
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

    /// An empty array sized for `capacity` keyed entries (array_init_size,
    /// as builtins returning a record make theirs): no growth while it fills.
    pub fn hashed(capacity: usize) -> Self {
        PhpArray { storage: Storage::Hashed(Hashed::with_capacity(capacity)), next_free: None, cursor: 0 }
    }

    /// A list of values (keys 0..n-1).
    pub fn from_list(values: Vec<Value>) -> Self {
        let next_free = (!values.is_empty()).then_some(values.len() as i64);
        PhpArray { storage: Storage::Packed(Packed { slots: values, holes: 0 }), next_free, cursor: 0 }
    }

    pub fn len(&self) -> usize {
        match &self.storage {
            Storage::Packed(p) => p.slots.len() - p.holes,
            Storage::Hashed(h) => h.live,
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

    pub fn get(&self, key: &Key) -> Option<&Value> {
        match &self.storage {
            Storage::Packed(p) => match key {
                Key::Int(i) => p.get(*i),
                _ => None,
            },
            Storage::Hashed(h) => h.find(hash_of(key), key).and_then(|i| h.get(i)),
        }
    }

    /// `get` for an int key.
    #[inline]
    pub fn get_int(&self, i: i64) -> Option<&Value> {
        match &self.storage {
            Storage::Packed(p) => p.get(i),
            Storage::Hashed(h) => h.find(i as u64, &Key::Int(i)).and_then(|i| h.get(i)),
        }
    }

    /// `get_mut` for an int key.
    #[inline]
    pub fn get_int_mut(&mut self, i: i64) -> Option<&mut Value> {
        match &mut self.storage {
            Storage::Packed(p) => p.get_mut(i),
            Storage::Hashed(h) => {
                let i = h.find(i as u64, &Key::Int(i))?;
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
                let i = h.find(hash_of(key), key)?;
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
        if let Key::Int(k) = key {
            self.note_int_key(k);
        }
        match &mut self.storage {
            Storage::Packed(p) => match key {
                Key::Int(i) if p.get(i).is_some() => p.slots[i as usize] = value,
                Key::Int(i) if p.fits(i) => {
                    let gap = i as usize - p.slots.len();
                    p.slots.resize(i as usize, Value::Undef);
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

    /// `$a[] = value`; Err when the next key is taken.
    pub fn push(&mut self, value: Value) -> Result<(), NextKeyOccupied> {
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
                let old = std::mem::replace(&mut p.slots[i as usize], Value::Undef);
                p.holes += 1;
                while matches!(p.slots.last(), Some(Value::Undef)) {
                    p.slots.pop();
                    p.holes -= 1;
                }
                Some(old)
            }
            Storage::Hashed(h) => h.remove(hash_of(key), key),
        }
    }

    fn make_hashed(&mut self) {
        if let Storage::Packed(p) = &mut self.storage {
            // Room for one more: the insert that made it a hash.
            let mut h = Hashed::with_capacity(p.slots.len() - p.holes + 1);
            for (i, value) in std::mem::take(&mut p.slots).into_iter().enumerate() {
                if !matches!(value, Value::Undef) {
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
                (!matches!(x, Value::Undef)).then(|| (pos + i, Key::Int((pos + i) as i64), x))
            }),
            Storage::Hashed(h) => h.buckets.get(pos..)?.iter().enumerate().find_map(|(i, b)| {
                h.is_set(pos + i).then(|| (pos + i, b.key(), &b.value))
            }),
        }
    }

    /// The last entry before position `pos`.
    pub fn entry_before(&self, pos: usize) -> Option<(usize, Key, &Value)> {
        match &self.storage {
            Storage::Packed(p) => p.slots.get(..pos.min(p.slots.len()))?.iter().enumerate().rev().find_map(|(i, x)| {
                (!matches!(x, Value::Undef)).then_some((i, Key::Int(i as i64), x))
            }),
            Storage::Hashed(h) => h.buckets.get(..pos.min(h.len()))?.iter().enumerate().rev().find_map(|(i, b)| {
                h.is_set(i).then(|| (i, b.key(), &b.value))
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
            v.iter().enumerate().filter(|(_, x)| !matches!(x, Value::Undef)).map(|(i, x)| (Key::Int(i as i64), x))
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

    fn int(i: i64) -> Value {
        Value::Int(i)
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
        assert_eq!(Key::from_value(&Value::Float(1.5)), Ok((Key::Int(1), Some(KeyIssue::LosesPrecision(1.5)))));
        assert_eq!(Key::from_value(&Value::Bool(true)), Ok((Key::Int(1), None)));
        assert_eq!(Key::from_value(&Value::Null), Ok((Key::Str(PhpStr::empty()), None)));
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
                let v = if i % 3 == 0 { Value::Undef } else { int(i) };
                a.insert(Key::Str(PhpStr::from(format!("p{i}"))), v);
            }
            let count = |a: &PhpArray| a.iter().count();
            assert_eq!((a.len(), count(&a)), (n as usize, n as usize), "{n}");
            assert!(matches!(a.get(&Key::Str(PhpStr::from("p0"))), Some(Value::Undef)), "{n}");
            let b = a.clone();
            assert_eq!(count(&b), n as usize);
            for i in (1..n).filter(|i| i % 3 != 0) {
                a.remove(&Key::Str(PhpStr::from(format!("p{i}"))));
            }
            let left: Vec<Key> = a.iter().map(|(k, _)| k).collect();
            let undefs: Vec<Key> = (0..n).filter(|i| i % 3 == 0).map(|i| Key::Str(PhpStr::from(format!("p{i}")))).collect();
            assert_eq!(left, undefs, "{n}");
            assert_eq!(a.len(), undefs.len());
            a.insert(Key::Str(PhpStr::from("after")), Value::Undef);
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
}
