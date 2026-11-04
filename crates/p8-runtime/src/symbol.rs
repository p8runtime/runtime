//! Names as numbers: every function, class and constant name is interned
//! once into a [`Sym`], and the name tables are keyed by it, so a lookup
//! hashes one integer and allocates nothing (PHP interns its names and
//! compares pointers).
//!
//! The table ([`Symbols`]) belongs to the [`crate::ext::Registry`], so the
//! compiler and every isolate on every thread share one: a name compiled
//! anywhere has the same symbol everywhere. Code names are interned when
//! they are compiled; a name made at run time (`define($n, …)`,
//! `class_alias`) when it is declared. A lookup of a name from request data
//! (`class_exists($s)`) only finds: a name never interned names nothing.

use crate::fast::HashMap;
use std::sync::{Arc, RwLock};

/// An interned name. Function and class names are interned lowercased
/// (their case does not matter); a constant's as its key (namespace
/// lowercased, its own name as written). Messages use the spelling the
/// code wrote, kept beside the symbol where it is needed.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Sym(u32);

impl Sym {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// The symbol table: names in, symbols out. `Send + Sync`; reads (the
/// lookups of names made at run time) take a shared lock, interning a new
/// name a short exclusive one. The [`known`] names have fixed symbols and
/// are found without the lock.
pub struct Symbols {
    table: RwLock<Table>,
}

/// Names the engine and the builtins look up by themselves (magic
/// methods, the interfaces' methods), lowercase: each has a fixed symbol,
/// its index here.
const KNOWN: &[&[u8]] = &[
    b"__construct", b"__destruct", b"__call", b"__callstatic", b"__get", b"__set", b"__isset", b"__unset",
    b"__tostring", b"__invoke", b"__clone", b"__serialize", b"__unserialize", b"__sleep", b"__wakeup",
    b"__set_state", b"__debuginfo", b"getiterator", b"current", b"key", b"next", b"valid", b"rewind",
    b"offsetget", b"offsetset", b"offsetexists", b"offsetunset", b"count", b"jsonserialize",
];

/// The fixed symbols of the names in `KNOWN`.
pub mod known {
    use super::Sym;
    pub const CONSTRUCT: Sym = Sym(0);
    pub const DESTRUCT: Sym = Sym(1);
    pub const CALL: Sym = Sym(2);
    pub const CALL_STATIC: Sym = Sym(3);
    pub const GET: Sym = Sym(4);
    pub const SET: Sym = Sym(5);
    pub const ISSET: Sym = Sym(6);
    pub const UNSET: Sym = Sym(7);
    pub const TO_STRING: Sym = Sym(8);
    pub const INVOKE: Sym = Sym(9);
    pub const CLONE: Sym = Sym(10);
}

/// The fixed symbol of a `KNOWN` name (as given: lowercase): one match,
/// which compiles to a switch on the length and a compare.
fn known_sym(name: &[u8]) -> Option<Sym> {
    Some(Sym(match name {
        b"__construct" => 0,
        b"__destruct" => 1,
        b"__call" => 2,
        b"__callstatic" => 3,
        b"__get" => 4,
        b"__set" => 5,
        b"__isset" => 6,
        b"__unset" => 7,
        b"__tostring" => 8,
        b"__invoke" => 9,
        b"__clone" => 10,
        b"__serialize" => 11,
        b"__unserialize" => 12,
        b"__sleep" => 13,
        b"__wakeup" => 14,
        b"__set_state" => 15,
        b"__debuginfo" => 16,
        b"getiterator" => 17,
        b"current" => 18,
        b"key" => 19,
        b"next" => 20,
        b"valid" => 21,
        b"rewind" => 22,
        b"offsetget" => 23,
        b"offsetset" => 24,
        b"offsetexists" => 25,
        b"offsetunset" => 26,
        b"count" => 27,
        b"jsonserialize" => 28,
        _ => return None,
    }))
}

impl std::fmt::Debug for Symbols {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Symbols({} names)", self.len())
    }
}

impl Default for Symbols {
    fn default() -> Symbols {
        let symbols = Symbols { table: RwLock::new(Table::default()) };
        for name in KNOWN {
            symbols.intern(name);
        }
        symbols
    }
}

#[derive(Default)]
struct Table {
    index: HashMap<Arc<[u8]>, Sym>,
    names: Vec<Arc<[u8]>>,
}

/// Names up to this long are lowercased on the stack when looked up.
const STACK_NAME: usize = 128;

impl Symbols {
    fn read(&self) -> std::sync::RwLockReadGuard<'_, Table> {
        // A panic while interning leaves the table consistent (an insert
        // is one push and one map insert), so a poisoned lock is usable.
        self.table.read().unwrap_or_else(|e| e.into_inner())
    }

    /// The symbol of `name`, exactly as given.
    pub fn intern(&self, name: &[u8]) -> Sym {
        if let Some(&s) = self.read().index.get(name) {
            return s;
        }
        let mut table = self.table.write().unwrap_or_else(|e| e.into_inner());
        if let Some(&s) = table.index.get(name) {
            return s;
        }
        let s = Sym(u32::try_from(table.names.len()).expect("fewer than 2^32 names"));
        let name: Arc<[u8]> = Arc::from(name);
        table.names.push(Arc::clone(&name));
        table.index.insert(name, s);
        s
    }

    /// The symbol of a function or class name: ASCII-lowercased (PHP's
    /// zend_str_tolower; other bytes are kept).
    pub fn intern_lower(&self, name: &[u8]) -> Sym {
        with_lower(name, name.len(), |lower| self.intern(lower))
    }

    /// The symbol of `name` (exactly as given), if it was interned.
    pub fn find(&self, name: &[u8]) -> Option<Sym> {
        if let Some(s) = known_sym(name) {
            return Some(s);
        }
        self.read().index.get(name).copied()
    }

    /// The symbol of a function or class name in any case, if it was
    /// interned. Allocates nothing for a name up to 128 bytes.
    pub fn find_lower(&self, name: &[u8]) -> Option<Sym> {
        with_lower(name, name.len(), |lower| self.find(lower))
    }

    /// The symbol of a constant's key (see [`crate::ext::const_key`]: its
    /// namespace in any case), if it was interned. Allocates nothing for a
    /// name up to 128 bytes.
    pub fn find_const(&self, name: &[u8]) -> Option<Sym> {
        let namespace = name.iter().rposition(|&c| c == b'\\').unwrap_or(0);
        with_lower(name, namespace, |key| self.find(key))
    }

    /// The name `s` was interned from.
    pub fn name(&self, s: Sym) -> Arc<[u8]> {
        Arc::clone(&self.read().names[s.index()])
    }

    /// How many names are interned.
    pub fn len(&self) -> usize {
        self.read().names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Run `f` on `name` with its first `upto` bytes ASCII-lowercased: the
/// name itself when they have no uppercase letter, else a copy (on the
/// stack when it is short).
fn with_lower<R>(name: &[u8], upto: usize, f: impl FnOnce(&[u8]) -> R) -> R {
    if !name[..upto].iter().any(u8::is_ascii_uppercase) {
        return f(name);
    }
    let mut buf = [0u8; STACK_NAME];
    let mut heap = Vec::new();
    let folded = if name.len() <= STACK_NAME {
        &mut buf[..name.len()]
    } else {
        heap.resize(name.len(), 0);
        &mut heap[..]
    };
    folded.copy_from_slice(name);
    folded[..upto].make_ascii_lowercase();
    f(folded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_are_shared_across_threads() {
        fn shared<T: Send + Sync>() {}
        shared::<Symbols>();
    }

    #[test]
    fn a_name_has_one_symbol_in_any_case() {
        let symbols = Symbols::default();
        let foo = symbols.intern_lower(b"Foo");
        assert_eq!(symbols.intern_lower(b"FOO"), foo);
        assert_eq!(symbols.find_lower(b"fOo"), Some(foo));
        assert_eq!(&*symbols.name(foo), b"foo");
        // Exact interning keeps the case (constants).
        assert_ne!(symbols.intern(b"FOO"), foo);
        // Only ASCII letters fold.
        let e = symbols.intern_lower("ÉTé".as_bytes());
        assert_eq!(&*symbols.name(e), "Été".as_bytes());
        assert_eq!(symbols.find_lower("ÉtÉ".as_bytes()), None);
        assert_eq!(symbols.find_lower("Été".as_bytes()), Some(e));
        // A long name is lowercased off the stack.
        let long = vec![b'A'; 300];
        assert_eq!(symbols.find_lower(&long), None);
        let s = symbols.intern_lower(&long);
        assert_eq!(symbols.find_lower(&vec![b'a'; 300]), Some(s));
    }

    #[test]
    fn a_constant_is_found_with_its_namespace_in_any_case() {
        let symbols = Symbols::default();
        let c = symbols.intern(&crate::ext::const_key(b"App\\Util\\LIMIT"));
        assert_eq!(symbols.find_const(b"APP\\util\\LIMIT"), Some(c));
        assert_eq!(symbols.find_const(b"app\\util\\Limit"), None);
        let g = symbols.intern(b"PHP_EOL");
        assert_eq!(symbols.find_const(b"PHP_EOL"), Some(g));
        assert_eq!(symbols.find_const(b"php_eol"), None);
    }

    #[test]
    fn finding_does_not_intern() {
        let symbols = Symbols::default();
        let known = symbols.len();
        assert_eq!(symbols.find_lower(b"nothing"), None);
        assert_eq!(symbols.len(), known);
    }

    #[test]
    fn known_names_have_their_fixed_symbols() {
        let symbols = Symbols::default();
        for (i, name) in KNOWN.iter().enumerate() {
            assert_eq!(symbols.find(name), Some(Sym(i as u32)));
            assert_eq!(&*symbols.name(Sym(i as u32)), *name);
        }
        assert_eq!(symbols.find_lower(b"__CONSTRUCT"), Some(known::CONSTRUCT));
        assert_eq!(symbols.intern_lower(b"__toString"), known::TO_STRING);
        assert_eq!(symbols.find(b"__callStatic"), None, "find is exact");
        assert!(KNOWN.iter().enumerate().all(|(i, k)| known_sym(k) == Some(Sym(i as u32))));
        for (s, name) in [(known::CONSTRUCT, "__construct"), (known::DESTRUCT, "__destruct"), (known::CALL, "__call"), (known::CALL_STATIC, "__callstatic"), (known::GET, "__get"), (known::SET, "__set"), (known::ISSET, "__isset"), (known::UNSET, "__unset"), (known::TO_STRING, "__tostring"), (known::INVOKE, "__invoke"), (known::CLONE, "__clone")] {
            assert_eq!(KNOWN[s.index()], name.as_bytes());
        }
    }
}
