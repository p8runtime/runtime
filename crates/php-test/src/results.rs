use crate::executor::TestResult;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Default, Clone, Copy)]
pub struct Counts {
    pub passed: usize,
    pub failed: usize,
    pub xfailed: usize,
    pub skipped: usize,
    pub errors: usize,
}

impl Counts {
    fn add(&mut self, result: &TestResult) {
        match result {
            TestResult::Passed => self.passed += 1,
            TestResult::Failed { .. } => self.failed += 1,
            TestResult::ExpectedFailure { .. } => self.xfailed += 1,
            TestResult::Skipped { .. } => self.skipped += 1,
            TestResult::Error { .. } => self.errors += 1,
        }
    }

    pub fn total(&self) -> usize {
        self.passed + self.failed + self.xfailed + self.skipped + self.errors
    }

    /// Pass rate over tests that ran (skips excluded), as php-src reports it.
    pub fn pass_rate(&self) -> f64 {
        let ran = self.total() - self.skipped;
        if ran == 0 {
            0.0
        } else {
            100.0 * self.passed as f64 / ran as f64
        }
    }
}

pub struct TestResults {
    pub counts: Counts,
    pub details: Vec<(PathBuf, TestResult)>,
}

impl TestResults {
    pub fn new() -> Self {
        Self {
            counts: Counts::default(),
            details: Vec::new(),
        }
    }

    pub fn add(&mut self, file: PathBuf, result: TestResult) {
        self.counts.add(&result);
        self.details.push((file, result));
    }

    pub fn has_failures(&self) -> bool {
        self.counts.failed > 0 || self.counts.errors > 0
    }

    /// One line per test (`PASS path`), for diffing two runs.
    pub fn write_list(&self, out: &mut impl Write) -> std::io::Result<()> {
        for (file, result) in &self.details {
            let tag = match result {
                TestResult::Passed => "PASS",
                TestResult::Failed { .. } => "FAIL",
                TestResult::ExpectedFailure { .. } => "XFAIL",
                TestResult::Skipped { .. } => "SKIP",
                TestResult::Error { .. } => "ERROR",
            };
            writeln!(out, "{} {}", tag, file.display())?;
        }
        Ok(())
    }

    pub fn print_details(&self, show_all: bool, verbose: bool) {
        for (file, result) in &self.details {
            let file = file.display();
            match result {
                TestResult::Passed if show_all => println!("PASS  {}", file),
                TestResult::Passed => {}
                TestResult::Failed { expected, actual } => {
                    println!("FAIL  {}", file);
                    if verbose {
                        println!(
                            "  --- expected\n{}\n  --- actual\n{}\n",
                            truncate_block(expected, 4096),
                            truncate_block(actual, 4096)
                        );
                    } else {
                        println!("  expected: {}", truncate(expected, 72));
                        println!("  actual:   {}", truncate(actual, 72));
                    }
                }
                TestResult::ExpectedFailure { .. } if show_all => println!("XFAIL {}", file),
                TestResult::ExpectedFailure { .. } => {}
                TestResult::Skipped { reason } if show_all => println!("SKIP  {} ({})", file, reason),
                TestResult::Skipped { .. } => {}
                TestResult::Error { error } => println!("ERROR {}\n  {}", file, truncate(error, 72)),
            }
        }
    }

    /// Counts grouped by the first `depth` path components under `root`.
    pub fn print_by_dir(&self, root: &Path, depth: usize) {
        let mut groups: BTreeMap<String, Counts> = BTreeMap::new();
        for (file, result) in &self.details {
            let rel = file.strip_prefix(root).unwrap_or(file);
            let parent = rel.parent().unwrap_or(Path::new(""));
            let key: Vec<_> = parent
                .components()
                .take(depth)
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            groups.entry(key.join("/")).or_default().add(result);
        }
        println!(
            "{:<40} {:>6} {:>6} {:>6} {:>6} {:>6}  {:>6}",
            "directory", "total", "pass", "fail", "skip", "error", "pass%"
        );
        for (dir, c) in &groups {
            println!(
                "{:<40} {:>6} {:>6} {:>6} {:>6} {:>6}  {:>5.1}%",
                if dir.is_empty() { "." } else { dir },
                c.total(),
                c.passed,
                c.failed + c.xfailed,
                c.skipped,
                c.errors,
                c.pass_rate()
            );
        }
    }

    pub fn print_summary(&self) {
        let c = &self.counts;
        println!("{}", "=".repeat(60));
        println!("Total:   {}", c.total());
        println!("Passed:  {}", c.passed);
        println!("Failed:  {}", c.failed);
        println!("XFAIL:   {}", c.xfailed);
        println!("Skipped: {}", c.skipped);
        println!("Errors:  {}", c.errors);
        println!("Pass rate (of tests run): {:.1}%", c.pass_rate());
    }
}

impl Default for TestResults {
    fn default() -> Self {
        Self::new()
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    let s = s.replace('\n', "\\n");
    match s.char_indices().nth(max_len) {
        Some((i, _)) => format!("{}...", &s[..i]),
        None => s,
    }
}

fn truncate_block(s: &str, max_len: usize) -> &str {
    match s.char_indices().nth(max_len) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}
