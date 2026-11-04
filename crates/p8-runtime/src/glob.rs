//! glob()'s flags (GLOB_*), as PHP 8.6 defines them: its own glob
//! implementation's values (main/php_glob.h), the same on every platform.
//! Until 8.5 they were the C library's (glob.h). glob() and
//! ZipArchive::addGlob() both take them.

pub const ERR: i64 = 0x0004;
pub const MARK: i64 = 0x0008;
pub const NOCHECK: i64 = 0x0010;
pub const NOSORT: i64 = 0x0020;
pub const BRACE: i64 = 0x0080;
pub const NOESCAPE: i64 = 0x1000;
pub const ONLYDIR: i64 = 1 << 30;
