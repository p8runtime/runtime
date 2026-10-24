//! Run php-src style .phpt tests against the `php` binary.
//!
//!   php-test path/to/php-src/Zend/tests -r --by-dir 2 --list results.txt
//!
//! Against p8-test-suite (the target tests and p8's recorded results):
//!
//!   php-test ../php-src --suite ../p8-test-suite --php target/release/php2 [-f zlib] [--only fail]
#![forbid(unsafe_code)] // memory safety is the point of this runtime; see SECURITY.md

use clap::Parser;
use php_rs::phpt::executor::{Limits, TestResult};
use php_rs::phpt::results::TestResults;
use php_rs::phpt::parser::parse_conflicts;
use php_rs::phpt::schedule::Schedule;
use php_rs::phpt::suite::{self, State, Suite};
use php_rs::phpt::{PhptExecutor, PhptTest};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::sync::{Condvar, Mutex};
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

    /// Tests run at once (at most half the cores; the machine is shared)
    #[arg(short = 'j', long, default_value_t = 4)]
    jobs: usize,

    /// Resident memory one test (with everything it starts) may use, in MB
    #[arg(long, default_value_t = 1024)]
    memory_mb: u64,

    /// Processes one test may have running at once
    #[arg(long, default_value_t = 32)]
    max_processes: usize,

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
    /// line) that real PHP passes, e.g. p8-test-suite's target.txt. Reports
    /// how many of them pass.
    #[arg(long)]
    target: Option<PathBuf>,

    /// Write the paths (relative to PATH) of the target tests that passed
    /// to this file, sorted: the pass list later runs are compared with.
    #[arg(long, requires = "target")]
    passes: Option<PathBuf>,

    /// A p8-test-suite checkout. PATH is then the php-src checkout at the
    /// suite's pinned commit; only target tests run, and the run is compared
    /// with the suite's recorded results. A test that passed there and does
    /// not now fails the run. The php-src checkout is reset before and after
    /// (tests write into it).
    #[arg(long, conflicts_with_all = ["target", "passes", "recursive", "by_dir"])]
    suite: Option<PathBuf>,

    /// With --suite: only the tests the suite records in this state
    #[arg(long, requires = "suite", value_parser = ["pass", "fail", "skip"])]
    only: Option<String>,

    /// With --suite: write this run to the suite's results.txt and run.json.
    /// Needs every target test (no -f/--only) and a clean p8 checkout, whose
    /// HEAD the run is recorded against.
    #[arg(long, requires = "suite", conflicts_with_all = ["filter", "only"])]
    write: bool,

    /// With --suite: for each test that does not pass, write what it expected
    /// and what p8 printed, as <DIR>/<test path>.exp and .out
    #[arg(long, requires = "suite")]
    outputs: Option<PathBuf>,
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

    if let Some(dir) = &cli.suite {
        return run_suite(&cli, php, dir);
    }

    let files = collect_phpt_files(&cli.path, cli.recursive, cli.filter.as_deref());
    anyhow::ensure!(!files.is_empty(), "no .phpt files in {}", cli.path.display());
    eprintln!("Running {} test(s) with {}", files.len(), php.display());

    let ran = run_all(&cli, php, &files)?;
    let mut results = TestResults::new();
    for (file, result) in files.into_iter().zip(ran) {
        results.add(file, result);
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

/// php-src keeps the shared PDO tests once, in ext/pdo/tests, and runs-tests
/// copies them per driver; a target path like ext/pdo_sqlite/tests/x.phpt
/// has no file of its own.
const MISSING: &str = "not in the php-src checkout";

fn run_suite(cli: &Cli, php: PathBuf, dir: &Path) -> anyhow::Result<()> {
    let suite = Suite::load(dir).map_err(|e| anyhow::anyhow!("{}: {e}", dir.display()))?;
    let head = git(&cli.path, &["rev-parse", "HEAD"])?;
    anyhow::ensure!(
        head == suite.php_commit,
        "{} is at {head}; the suite needs php-src {} ({})",
        cli.path.display(),
        suite.php_tag,
        suite.php_commit
    );
    let p8 = if cli.write {
        let dirty = git(Path::new("."), &["status", "--porcelain", "--untracked-files=no"])?;
        anyhow::ensure!(dirty.is_empty(), "--write records the run against p8's HEAD; commit or stash first:\n{dirty}");
        Some(git(Path::new("."), &["rev-parse", "HEAD"])?)
    } else {
        None
    };

    let only = cli.only.as_deref().and_then(State::parse);
    let picked: Vec<usize> = (0..suite.tests.len())
        .filter(|&i| only.is_none_or(|s| suite.recorded[i] == s))
        .filter(|&i| cli.filter.as_deref().is_none_or(|f| suite.tests[i].contains(f)))
        .collect();
    anyhow::ensure!(!picked.is_empty(), "no target tests match");
    let files: Vec<PathBuf> = picked.iter().map(|&i| cli.path.join(&suite.tests[i])).collect();
    eprintln!("Running {} of {} target test(s) with {}", files.len(), suite.tests.len(), php.display());

    reset_php_src(&cli.path)?;
    let started = chrono::Utc::now();
    let ran = run_all(cli, php, &files);
    reset_php_src(&cli.path)?;
    let ran = ran?;
    eprintln!("took {:.0}s", (chrono::Utc::now() - started).as_seconds_f64());

    if let Some(out) = &cli.outputs {
        for (&i, result) in picked.iter().zip(&ran) {
            let (exp, actual) = match result {
                TestResult::Passed => continue,
                TestResult::Failed { expected, actual } => (expected.as_str(), actual.as_str()),
                TestResult::ExpectedFailure { reason } | TestResult::Skipped { reason } => ("", reason.as_str()),
                TestResult::Error { error } => ("", error.as_str()),
            };
            let base = out.join(&suite.tests[i]);
            std::fs::create_dir_all(base.parent().expect("a test path has a folder"))?;
            std::fs::write(base.with_extension("exp"), exp)?;
            std::fs::write(base.with_extension("out"), actual)?;
        }
    }

    let states: Vec<(usize, State)> = picked.iter().copied().zip(ran.iter().map(State::of)).collect();
    let c = suite::compare(&suite, &states);
    if cli.show_all || cli.verbose {
        let mut details = TestResults::new();
        for (file, result) in files.into_iter().zip(ran) {
            details.add(file, result);
        }
        details.print_details(cli.show_all, cli.verbose);
    }
    let changed: Vec<_> = c.areas.iter().filter(|(_, (before, now, _))| before != now).collect();
    if !changed.is_empty() {
        println!("\n{:<28} {:>7} {:>7} {:>7}", "area", "before", "now", "tests");
        for (area, (before, now, total)) in changed {
            println!("{area:<28} {before:>7} {now:>7} {total:>7}  {:+}", *now as i64 - *before as i64);
        }
    }
    if !c.gained.is_empty() {
        println!("\nGained {}:", c.gained.len());
        for path in c.gained.iter().take(40) {
            println!("  + {path}");
        }
        if c.gained.len() > 40 {
            println!("  … and {} more", c.gained.len() - 40);
        }
    }
    if !c.lost.is_empty() {
        println!("\nLost {}:", c.lost.len());
        for (path, now) in &c.lost {
            println!("  - {path} ({now})");
        }
    }
    let total = suite.tests.len();
    let recorded = suite.recorded.iter().filter(|&&s| s == State::Pass).count();
    let now_total = recorded + c.pass_now - c.pass_before;
    println!(
        "\nRan {} test(s): {} pass (was {}), {} gained, {} lost",
        c.ran,
        c.pass_now,
        c.pass_before,
        c.gained.len(),
        c.lost.len()
    );
    println!(
        "Target: {now_total} of {total} pass ({:.1}%), was {recorded}{}",
        100.0 * now_total as f64 / total as f64,
        if c.ran < total { " (tests not run keep their recorded state)" } else { "" }
    );

    if let Some(p8) = p8 {
        let all: Vec<State> = states.iter().map(|&(_, s)| s).collect();
        suite::write(dir, &suite, &all, &p8, &started.format("%Y-%m-%dT%H:%M:%SZ").to_string())?;
        println!("Wrote {} and run.json for p8 {}", dir.join("results.txt").display(), &p8[..7]);
    }
    if !c.lost.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

/// Puts the php-src checkout back as git has it: tests write files into it.
fn reset_php_src(php_src: &Path) -> anyhow::Result<()> {
    git(php_src, &["checkout", "-q", "--", "."])?;
    git(php_src, &["clean", "-fdq"])?;
    Ok(())
}

fn git(dir: &Path, args: &[&str]) -> anyhow::Result<String> {
    let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).output()?;
    anyhow::ensure!(out.status.success(), "git {} in {}: {}", args.join(" "), dir.display(), String::from_utf8_lossy(&out.stderr).trim());
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Hands a batch back to the schedule when it ends, even by a panic.
struct Finish<'a> {
    schedule: &'a Mutex<Schedule>,
    freed: &'a Condvar,
    batch: &'a [usize],
}

impl Drop for Finish<'_> {
    fn drop(&mut self) {
        let mut s = self.schedule.lock().unwrap_or_else(|e| e.into_inner());
        s.done(self.batch);
        drop(s);
        self.freed.notify_all();
    }
}

/// Runs each file (a missing one is skipped), `jobs` at a time, in order.
fn run_all(cli: &Cli, php: PathBuf, files: &[PathBuf]) -> anyhow::Result<Vec<TestResult>> {
    let tests: Vec<Result<PhptTest, String>> = files
        .iter()
        .map(|f| if f.is_file() { PhptTest::from_file(f).map_err(|e| e.to_string()) } else { Err(MISSING.to_owned()) })
        .collect();
    let limits = Limits {
        timeout: Duration::from_secs(cli.timeout),
        memory: cli.memory_mb << 20,
        processes: cli.max_processes,
    };
    let mut executor = PhptExecutor::new(php, limits);
    let wanted: BTreeSet<&str> = tests
        .iter()
        .flatten()
        .flat_map(|t| t.sections.extensions.iter().map(String::as_str))
        .collect();
    executor
        .probe_extensions(wanted, &std::env::temp_dir())
        .map_err(|e| anyhow::anyhow!("extension probe failed: {}", e))?;

    let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
    let jobs = cli.jobs.clamp(1, (cores / 2).max(1));
    eprintln!("{jobs} at a time, each up to {} MB, {} processes, {}s", cli.memory_mb, cli.max_processes, cli.timeout);
    // A test's own --CONFLICTS-- wins over its folder's CONFLICTS file.
    let mut folder_keys: HashMap<PathBuf, Vec<String>> = HashMap::new();
    let conflicts: Vec<Vec<String>> = files
        .iter()
        .zip(&tests)
        .map(|(file, test)| match test {
            Ok(t) if !t.sections.conflicts.is_empty() => t.sections.conflicts.clone(),
            _ => {
                let dir = file.parent().unwrap_or(Path::new("")).to_path_buf();
                folder_keys
                    .entry(dir)
                    .or_insert_with_key(|dir| std::fs::read_to_string(dir.join("CONFLICTS")).map(|t| parse_conflicts(&t)).unwrap_or_default())
                    .clone()
            }
        })
        .collect();
    let schedule = Mutex::new(Schedule::new(conflicts, jobs));
    let freed = Condvar::new();
    let slots: Vec<Mutex<Option<TestResult>>> = files.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| {
                loop {
                    let batch = {
                        let mut s = schedule.lock().unwrap();
                        loop {
                            if let Some(batch) = s.next() {
                                break batch;
                            }
                            if s.finished() {
                                return;
                            }
                            // What's left waits on a key a running batch holds.
                            s = freed.wait(s).unwrap();
                        }
                    };
                    // A panicking test must not strand the workers waiting on its batch's keys.
                    let finish = Finish { schedule: &schedule, freed: &freed, batch: &batch };
                    for &i in &batch {
                        let result = match &tests[i] {
                            Ok(test) => executor.run_test(test),
                            Err(e) if e == MISSING => TestResult::Skipped { reason: e.clone() },
                            Err(e) => TestResult::Error {
                                error: format!("unparseable test: {}", e),
                            },
                        };
                        *slots[i].lock().unwrap() = Some(result);
                    }
                    drop(finish);
                }
            });
        }
    });

    Ok(slots.into_iter().map(|slot| slot.into_inner().unwrap().expect("every test ran")).collect())
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
