//! php-src `.phpt` test runner (see `src/bin/php-test.rs`).

pub mod executor;
pub mod matcher;
pub mod parser;
pub mod results;

pub use executor::{PhptExecutor, TestResult};
pub use matcher::{ExpectationType, match_output};
pub use parser::{PhptSections, PhptTest};
pub use results::TestResults;
