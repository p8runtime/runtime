#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! php-src's `.phpt` test runner: parses each test, runs it against a php
//! binary (p8) in its own process group, and compares the output with what
//! the test expects (the `php-test` binary, `src/main.rs`).

pub mod executor;
pub mod matcher;
pub mod parser;
pub mod results;
pub mod schedule;
pub mod suite;

pub use executor::{PhptExecutor, TestResult};
pub use matcher::{ExpectationType, match_output};
pub use parser::{PhptSections, PhptTest};
pub use results::TestResults;
