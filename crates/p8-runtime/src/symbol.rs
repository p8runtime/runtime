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
/// name a short exclusive one.
#[derive(Default)]
pub struct Symbols {
    table: RwLock<Table>,
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
        with_lower(name, |lower| self.intern(lower))
    }

    /// The symbol of `name` (exactly as given), if it was interned.
    pub fn find(&self, name: &[u8]) -> Option<Sym> {
        self.read().index.get(name).copied()
    }

    /// The symbol of a function or class name in any case, if it was
    /// interned. Allocates nothing for a name up to 128 bytes.
    pub fn find_lower(&self, name: &[u8]) -> Option<Sym> {
        with_lower(name, |lower| self.find(lower))
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

/// Run `f` on `name` ASCII-lowercased: the name itself when it has no
/// uppercase letter, else a copy (on the stack when it is short).
fn with_lower<R>(name: &[u8], f: impl FnOnce(&[u8]) -> R) -> R {
    if !name.iter().any(u8::is_ascii_uppercase) {
        return f(name);
    }
    if name.len() <= STACK_NAME {
        let mut buf = [0u8; STACK_NAME];
        let lower = &mut buf[..name.len()];
        lower.copy_from_slice(name);
        lower.make_ascii_lowercase();
        return f(lower);
    }
    f(&name.to_ascii_lowercase())
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
    fn finding_does_not_intern() {
        let symbols = Symbols::default();
        assert_eq!(symbols.find_lower(b"nothing"), None);
        assert!(symbols.is_empty());
    }
}
