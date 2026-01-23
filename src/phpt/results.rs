use crate::phpt::executor::TestResult;
use std::path::PathBuf;

pub struct TestResults {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub errors: usize,
    pub details: Vec<TestDetail>,
}

pub struct TestDetail {
    pub file: PathBuf,
    pub result: TestResult,
}

impl TestResults {
    pub fn new() -> Self {
        Self {
            total: 0,
            passed: 0,
            failed: 0,
            skipped: 0,
            errors: 0,
            details: Vec::new(),
        }
    }

    pub fn add(&mut self, file: PathBuf, result: TestResult) {
        self.total += 1;

        match &result {
            TestResult::Passed => self.passed += 1,
            TestResult::Failed { .. } => self.failed += 1,
            TestResult::Skipped { .. } => self.skipped += 1,
            TestResult::Error { .. } => self.errors += 1,
        }

        self.details.push(TestDetail { file, result });
    }

    pub fn print_summary(&self, show_all: bool, verbose: bool) {
        println!("\nTest Results:");
        println!("{}", "=".repeat(80));

        for detail in &self.details {
            let file_display = detail.file.display();
            match &detail.result {
                TestResult::Passed => {
                    if show_all {
                        println!("✓ PASS {}", file_display);
                    }
                }
                TestResult::Failed { expected, actual } => {
                    println!("✗ FAIL {}", file_display);
                    if verbose {
                        println!("  Expected:");
                        for line in expected.lines() {
                            println!("    {}", line);
                        }
                        println!("  Actual:");
                        for line in actual.lines() {
                            println!("    {}", line);
                        }
                        println!();
                    } else {
                        println!("  Expected: {}", truncate(expected, 60));
                        println!("  Actual:   {}", truncate(actual, 60));
                    }
                }
                TestResult::Skipped { reason } => {
                    println!("⊘ SKIP {} ({})", file_display, reason);
                }
                TestResult::Error { error } => {
                    println!("✗ ERROR {}", file_display);
                    if verbose {
                        println!("  Error:");
                        for line in error.lines() {
                            println!("    {}", line);
                        }
                        println!();
                    } else {
                        println!("  Error: {}", truncate(error, 60));
                    }
                }
            }
        }

        println!("{}", "=".repeat(80));
        println!("Summary:");
        println!("  Total:   {}", self.total);
        println!("  Passed:  {}", self.passed);
        println!("  Failed:  {}", self.failed);
        println!("  Skipped: {}", self.skipped);
        println!("  Errors:  {}", self.errors);

        if self.failed > 0 || self.errors > 0 {
            println!("\nSome tests failed or had errors.");
            std::process::exit(1);
        }
    }
}

impl Default for TestResults {
    fn default() -> Self {
        Self::new()
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    let s = s.replace('\n', "\\n");
    if s.len() <= max_len {
        s
    } else {
        format!("{}...", &s[..max_len])
    }
}
