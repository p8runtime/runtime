//! Per-thread counts of hash-table work, for tests that prove a change
//! made the engine do less of it (#411): the work, not the output, is what
//! such a change alters. Counted only in a build with the `counts`
//! feature (the tests' build: `p8`'s dev-dependency turns it on), so an
//! ordinary build pays nothing; reading a count without the feature does
//! not compile, so a test cannot pass by reading zeros.

/// What is counted.
#[derive(Clone, Copy, Debug)]
pub enum Count {
    /// `PhpArray::insert`: a keyed insert, which tests the key and, in a
    /// hash, looks for it first.
    Insert = 0,
    /// A string's hash computed (`StrHead::hash` the first time).
    Hash = 1,
    /// A function or class name, given as bytes, resolved to its symbol
    /// while running (the interpreter's `sym_of`: a hash of the name and a
    /// lookup).
    NameLookup = 2,
    /// A string made in an allocation of its own (`PhpStr`'s handle
    /// allocated): what #412's changes stop doing per call.
    StrMade = 3,
    /// An array let go of by its last value holding it (its free, PHP's
    /// `zend_array_destroy` from `rc_dtor_func`).
    ArrayFreed = 4,
}

#[cfg(feature = "counts")]
thread_local! {
    static COUNTS: [std::cell::Cell<u64>; 5] = const { [const { std::cell::Cell::new(0) }; 5] };
}

/// One more of `c` on this thread (nothing without the feature).
#[inline(always)]
pub fn bump(c: Count) {
    #[cfg(feature = "counts")]
    COUNTS.with(|n| n[c as usize].set(n[c as usize].get() + 1));
    #[cfg(not(feature = "counts"))]
    let _ = c;
}

/// How many of `c` this thread has done.
#[cfg(feature = "counts")]
pub fn get(c: Count) -> u64 {
    COUNTS.with(|n| n[c as usize].get())
}
