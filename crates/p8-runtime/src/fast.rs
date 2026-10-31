//! The hash maps p8's own tables use: class, function, method, constant
//! and property names, the compiler's symbol tables. Their keys come from
//! code, not request data, so they hash with foldhash (as PHP's tables
//! hash with DJBX33A) rather than SipHash, which costs several times more
//! per name and showed as about a tenth of a Laravel test run's
//! instructions. A PHP array's own keys hash in `array` (FNV-1a).

pub type BuildHasher = foldhash::fast::RandomState;
pub type HashMap<K, V> = std::collections::HashMap<K, V, BuildHasher>;
pub type HashSet<K> = std::collections::HashSet<K, BuildHasher>;
pub type IndexMap<K, V> = indexmap::IndexMap<K, V, BuildHasher>;
