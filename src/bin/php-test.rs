//! Run php-src style .phpt tests against the `php` binary.
//!
//!   php-test path/to/php-src/Zend/tests -r --by-dir 2 --list results.txt
#![forbid(unsafe_code)] // memory safety is the point of this runtime; see SECURITY.md

use clap::Parser;
use php_rs::phpt::executor::TestResult;
use php_rs::phpt::results::TestResults;
use php_rs::phpt::{PhptExecutor, PhptTest};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use walkdir::WalkDir;

#[derive(Parser)]
#[command(name = "php-test", about = "Run .phpt test files against php-rs")]
struct Cli {
    /// .phpt file or directory
    path: PathBuf,

    /// Recurse into subdirectories
    #[arg(short = 'r', long)]
    recursive: bool,

    /// Only tests whose path contains this text
    #[arg(short = 'f', long)]
    filter: Option<String>,

    /// The php binary under test (default: `php` next to this binary)
    #[arg(long)]
    php: Option<PathBuf>,

    /// Per-test timeout in seconds
    #[arg(long, default_value_t = 10)]
    timeout: u64,

    /// Parallel test processes (default: available cores)
    #[arg(short = 'j', long)]
    jobs: Option<usize>,

    /// Print every test, not only failures
    #[arg(short = 's', long)]
    show_all: bool,

    /// Print full expected/actual output for failures
    #[arg(short = 'v', long)]
    verbose: bool,

    /// Only print the summary (and --by-dir table)
    #[arg(short = 'q', long)]
    quiet: bool,

    /// Group counts by this many directory levels under PATH
    #[arg(long)]
    by_dir: Option<usize>,

    /// Write `STATUS path` per test to this file
    #[arg(long)]
    list: Option<PathBuf>,

    /// The target set: a file of test paths (relative to PATH, one per
    /// line) that real PHP passes, e.g. tests/php-src/target.txt. Reports
    /// how many of them pass.
    #[arg(long)]
    target: Option<PathBuf>,

    /// Write the paths (relative to PATH) of the target tests that passed
    /// to this file, sorted: the pass list later runs are compared with.
    #[arg(long, requires = "target")]
    passes: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let php = match cli.php.clone() {
        Some(p) => p,
        None => std::env::current_exe()?.with_file_name("php"),
    };
    anyhow::ensure!(php.is_file(), "php binary not found: {}", php.display());
    // Tests run from their own directory, so a relative path would break.
    let php = std::path::absolute(php)?;

    let files = collect_phpt_files(&cli.path, cli.recursive, cli.filter.as_deref());
    anyhow::ensure!(!files.is_empty(), "no .phpt files in {}", cli.path.display());
    eprintln!("Running {} test(s) with {}", files.len(), php.display());

    let tests: Vec<Result<PhptTest, String>> = files
        .iter()
        .map(|f| PhptTest::from_file(f).map_err(|e| e.to_string()))
        .collect();
    let mut executor = PhptExecutor::new(php, Duration::from_secs(cli.timeout));
    let wanted: BTreeSet<&str> = tests
        .iter()
        .flatten()
        .flat_map(|t| t.sections.extensions.iter().map(String::as_str))
        .collect();
    executor
        .probe_extensions(wanted, &std::env::temp_dir())
        .map_err(|e| anyhow::anyhow!("extension probe failed: {}", e))?;

    let jobs = cli
        .jobs
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()));
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<TestResult>>> = files.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..jobs.max(1) {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(test) = tests.get(i) else { break };
                    let result = match test {
                        Ok(test) => executor.run_test(test),
                        Err(e) => TestResult::Error {
                            error: format!("unparseable test: {}", e),
                        },
                    };
                    *slots[i].lock().unwrap() = Some(result);
                }
            });
        }
    });

    let mut results = TestResults::new();
    for (file, slot) in files.into_iter().zip(slots) {
        if let Some(result) = slot.into_inner().unwrap() {
            results.add(file, result);
        }
    }
    if !cli.quiet {
        results.print_details(cli.show_all, cli.verbose);
    }
    if let Some(depth) = cli.by_dir {
        let root = if cli.path.is_dir() { cli.path.as_path() } else { Path::new("") };
        results.print_by_dir(root, depth);
    }
    results.print_summary();
    if let Some(target) = &cli.target {
        let target = std::fs::read_to_string(target)?;
        let target: BTreeSet<&str> = target.lines().filter(|l| !l.is_empty()).collect();
        let root = if cli.path.is_dir() { cli.path.as_path() } else { Path::new("") };
        let passed: BTreeSet<String> = results
            .details
            .iter()
            .filter(|(_, r)| matches!(r, TestResult::Passed))
            .map(|(file, _)| file.strip_prefix(root).unwrap_or(file).to_string_lossy().into_owned())
            .filter(|rel| target.contains(rel.as_str()))
            .collect();
        println!(
            "Target:  {} of {} passed ({:.1}%)",
            passed.len(),
            target.len(),
            100.0 * passed.len() as f64 / target.len().max(1) as f64
        );
        if let Some(passes) = &cli.passes {
            let mut out = String::new();
            for p in &passed {
                out.push_str(p);
                out.push('\n');
            }
            std::fs::write(passes, out)?;
        }
    }
    if let Some(list) = &cli.list {
        results.write_list(&mut std::fs::File::create(list)?)?;
    }
    if results.has_failures() {
        std::process::exit(1);
    }
    Ok(())
}

fn collect_phpt_files(path: &Path, recursive: bool, filter: Option<&str>) -> Vec<PathBuf> {
    let walker = WalkDir::new(path)
        .follow_links(true)
        .max_depth(if recursive { usize::MAX } else { 1 });
    let mut files: Vec<PathBuf> = walker
        .into_iter()
        .filter_map(|e| e.ok())
        .map(|e| e.into_path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "phpt"))
        .filter(|p| filter.is_none_or(|f| p.to_string_lossy().contains(f)))
        .collect();
    files.sort();
    files
}
