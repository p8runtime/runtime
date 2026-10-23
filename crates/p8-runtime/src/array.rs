//! PHP arrays: ordered maps with int and string keys. A list (keys 0..n-1
//! in order) is stored packed as a plain vector; anything else is an
//! ordered hash with tombstones, so unset is O(1) as in PHP and order is
//! kept. Arrays are values: `Rc<PhpArray>` is shared until written
//! (`Rc::make_mut`). Key rules follow zend_hash.c / zend_operators.c.

use crate::string::PhpStr;
use crate::value::Value;
use crate::number::dval_to_lval;
use hashbrown::HashTable;
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
            Value::Ref(cell) => return Key::from_value(&cell.borrow()),
        })
    }

    /// The key as a value (what foreach and array_keys give).
    pub fn to_value(&self) -> Value {
        match self {
            Key::Int(i) => Value::Int(*i),
            Key::Str(s) => Value::Str(s.clone()),
        }
    }

    fn hash(&self) -> u64 {
        match self {
            // Fibonacci hashing spreads sequential ints across buckets
            Key::Int(i) => (*i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15),
            Key::Str(s) => s.hash(),
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
    /// Keys 0..len-1 in order.
    Packed(Vec<Value>),
    Hashed(Hashed),
}

/// An insertion-ordered hash: entries in order (None where unset), and a
/// table of entry indices by key hash.
#[derive(Debug, Clone, Default)]
struct Hashed {
    entries: Vec<Option<(Key, Value)>>,
    index: HashTable<usize>,
    live: usize,
}

impl Hashed {
    fn find(&self, key: &Key) -> Option<usize> {
        let entries = &self.entries;
        self.index
            .find(key.hash(), |&i| entries[i].as_ref().is_some_and(|(k, _)| k == key))
            .copied()
    }

    fn insert_new(&mut self, key: Key, value: Value) {
        let i = self.entries.len();
        let hash = key.hash();
        self.entries.push(Some((key, value)));
        let entries = &self.entries;
        self.index.insert_unique(hash, i, |&j| {
            entries[j].as_ref().map_or(0, |(k, _)| k.hash())
        });
        self.live += 1;
    }

    fn remove(&mut self, key: &Key) -> Option<Value> {
        let hash = key.hash();
        let entries = &self.entries;
        let found = self
            .index
            .find_entry(hash, |&i| entries[i].as_ref().is_some_and(|(k, _)| k == key))
            .ok()?;
        let (i, _) = found.remove();
        self.live -= 1;
        let (_, value) = self.entries[i].take()?;
        // Compact when more than half the entries are holes, as PHP rehashes.
        if self.entries.len() > 8 && self.live * 2 < self.entries.len() {
            self.compact();
        }
        Some(value)
    }

    fn compact(&mut self) {
        let entries: Vec<(Key, Value)> = std::mem::take(&mut self.entries).into_iter().flatten().collect();
        *self = Hashed::default();
        for (k, v) in entries {
            self.insert_new(k, v);
        }
    }
}

#[derive(Debug, Clone)]
pub struct PhpArray {
    storage: Storage,
    /// The key `$a[] = v` uses next; None until an int key is used
    /// (PHP 8.3+: after only negative keys it is the largest plus one).
    next_free: Option<i64>,
}

impl Default for PhpArray {
    fn default() -> Self {
        Self::new()
    }
}

impl PhpArray {
    pub fn new() -> Self {
        PhpArray { storage: Storage::Packed(Vec::new()), next_free: None }
    }

    /// A list of values (keys 0..n-1).
    pub fn from_list(values: Vec<Value>) -> Self {
        let next_free = (!values.is_empty()).then_some(values.len() as i64);
        PhpArray { storage: Storage::Packed(values), next_free }
    }

    pub fn len(&self) -> usize {
        match &self.storage {
            Storage::Packed(v) => v.len(),
            Storage::Hashed(h) => h.live,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the keys are exactly 0..n-1 in order (array_is_list).
    pub fn is_list(&self) -> bool {
        match &self.storage {
            Storage::Packed(_) => true,
            Storage::Hashed(_) => self.iter().enumerate().all(|(i, (k, _))| k == Key::Int(i as i64)),
        }
    }

    pub fn get(&self, key: &Key) -> Option<&Value> {
        match &self.storage {
            Storage::Packed(v) => match key {
                Key::Int(i) if *i >= 0 => v.get(*i as usize),
                _ => None,
            },
            Storage::Hashed(h) => h.find(key).and_then(|i| h.entries[i].as_ref().map(|(_, v)| v)),
        }
    }

    /// `get` for an int key.
    #[inline]
    pub fn get_int(&self, i: i64) -> Option<&Value> {
        match &self.storage {
            Storage::Packed(v) => usize::try_from(i).ok().and_then(|i| v.get(i)),
            Storage::Hashed(h) => h.find(&Key::Int(i)).and_then(|i| h.entries[i].as_ref().map(|(_, v)| v)),
        }
    }

    /// `get_mut` for an int key.
    #[inline]
    pub fn get_int_mut(&mut self, i: i64) -> Option<&mut Value> {
        match &mut self.storage {
            Storage::Packed(v) => usize::try_from(i).ok().and_then(|i| v.get_mut(i)),
            Storage::Hashed(h) => {
                let i = h.find(&Key::Int(i))?;
                h.entries[i].as_mut().map(|(_, v)| v)
            }
        }
    }

    pub fn get_mut(&mut self, key: &Key) -> Option<&mut Value> {
        match &mut self.storage {
            Storage::Packed(v) => match key {
                Key::Int(i) if *i >= 0 => v.get_mut(*i as usize),
                _ => None,
            },
            Storage::Hashed(h) => {
                let i = h.find(key)?;
                h.entries[i].as_mut().map(|(_, v)| v)
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
            Storage::Packed(v) => match key {
                Key::Int(i) if i >= 0 && (i as usize) < v.len() => v[i as usize] = value,
                Key::Int(i) if i >= 0 && i as usize == v.len() => v.push(value),
                key => {
                    self.make_hashed();
                    self.insert(key, value);
                }
            },
            Storage::Hashed(h) => match h.find(&key) {
                Some(i) => {
                    if let Some((_, v)) = h.entries[i].as_mut() {
                        *v = value;
                    }
                }
                None => h.insert_new(key, value),
            },
        }
    }

    /// `$a[] = value`; Err when the next key is taken.
    pub fn push(&mut self, value: Value) -> Result<(), NextKeyOccupied> {
        let key = self.next_key().ok_or(NextKeyOccupied)?;
        self.insert(Key::Int(key), value);
        Ok(())
    }

    /// unset($a[key])
    pub fn remove(&mut self, key: &Key) -> Option<Value> {
        if let Storage::Packed(v) = &self.storage {
            match key {
                Key::Int(i) if *i >= 0 && (*i as usize) < v.len() => self.make_hashed(),
                _ => return None,
            }
        }
        match &mut self.storage {
            Storage::Hashed(h) => h.remove(key),
            Storage::Packed(_) => None,
        }
    }

    fn make_hashed(&mut self) {
        if let Storage::Packed(v) = &mut self.storage {
            let mut h = Hashed::default();
            for (i, value) in std::mem::take(v).into_iter().enumerate() {
                h.insert_new(Key::Int(i as i64), value);
            }
            self.storage = Storage::Hashed(h);
        }
    }

    /// The first entry at or after position `pos` in iteration order, with
    /// its position (foreach walks an array by position).
    pub fn entry_at(&self, pos: usize) -> Option<(usize, Key, &Value)> {
        match &self.storage {
            Storage::Packed(v) => v.get(pos).map(|x| (pos, Key::Int(pos as i64), x)),
            Storage::Hashed(h) => h.entries.get(pos..)?.iter().enumerate().find_map(|(i, e)| {
                e.as_ref().map(|(k, v)| (pos + i, k.clone(), v))
            }),
        }
    }

    /// Entries in order.
    pub fn iter(&self) -> impl Iterator<Item = (Key, &Value)> {
        let (packed, hashed) = match &self.storage {
            Storage::Packed(v) => (Some(v), None),
            Storage::Hashed(h) => (None, Some(h)),
        };
        let packed = packed.into_iter().flat_map(|v| v.iter().enumerate().map(|(i, x)| (Key::Int(i as i64), x)));
        let hashed = hashed
            .into_iter()
            .flat_map(|h| h.entries.iter().flatten().map(|(k, v)| (k.clone(), v)));
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
    fn copies_share_until_written() {
        let a = std::rc::Rc::new(PhpArray::from_list(vec![int(1)]));
        let mut b = a.clone();
        std::rc::Rc::make_mut(&mut b).push(int(2)).unwrap();
        assert_eq!((a.len(), b.len()), (1, 2));
    }
}
