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
}

#[cfg(feature = "counts")]
thread_local! {
    static COUNTS: [std::cell::Cell<u64>; 2] = const { [const { std::cell::Cell::new(0) }; 2] };
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
