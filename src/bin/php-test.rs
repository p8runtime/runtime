use clap::Parser;
use php_rs::phpt::{PhptExecutor, PhptTest};
use php_rs::phpt::results::TestResults;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Parser)]
#[command(name = "php-test")]
#[command(about = "Run .phpt test files for php-rs", long_about = None)]
struct Cli {
    /// Path to .phpt file or directory
    #[arg(name = "PATH")]
    path: PathBuf,

    /// Recursively search for .phpt files in directories
    #[arg(short = 'r', long)]
    recursive: bool,

    /// Filter tests by name pattern
    #[arg(short = 'f', long)]
    filter: Option<String>,

    /// Show all tests including passed ones
    #[arg(short = 's', long)]
    show_all: bool,

    /// Verbose output (show full diffs)
    #[arg(short = 'v', long)]
    verbose: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let test_files = collect_phpt_files(&cli.path, cli.recursive, cli.filter.as_deref())?;

    if test_files.is_empty() {
        eprintln!("No .phpt files found in {}", cli.path.display());
        return Ok(());
    }

    println!("Running {} test(s) from: {}", test_files.len(), cli.path.display());
    println!();

    let executor = PhptExecutor::new()
        .map_err(|e| anyhow::anyhow!("Failed to create executor: {}", e))?;

    let mut results = TestResults::new();

    for test_file in test_files {
        match PhptTest::from_file(&test_file) {
            Ok(test) => {
                let result = executor.run_test(&test);
                results.add(test_file, result);
            }
            Err(e) => {
                use php_rs::phpt::executor::TestResult;
                results.add(
                    test_file,
                    TestResult::Error {
                        error: format!("Failed to parse test file: {}", e),
                    },
                );
            }
        }
    }

    results.print_summary(cli.show_all, cli.verbose);

    Ok(())
}

fn collect_phpt_files(
    path: &Path,
    recursive: bool,
    filter: Option<&str>,
) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    if path.is_file() {
        if path.extension().and_then(|s| s.to_str()) == Some("phpt") {
            files.push(path.to_path_buf());
        }
    } else if path.is_dir() {
        if recursive {
            for entry in WalkDir::new(path)
                .follow_links(true)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let entry_path = entry.path();
                if entry_path.is_file()
                    && entry_path.extension().and_then(|s| s.to_str()) == Some("phpt")
                {
                    if let Some(filter_pattern) = filter {
                        if entry_path
                            .to_string_lossy()
                            .contains(filter_pattern)
                        {
                            files.push(entry_path.to_path_buf());
                        }
                    } else {
                        files.push(entry_path.to_path_buf());
                    }
                }
            }
        } else {
            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                let entry_path = entry.path();
                if entry_path.is_file()
                    && entry_path.extension().and_then(|s| s.to_str()) == Some("phpt")
                {
                    if let Some(filter_pattern) = filter {
                        if entry_path
                            .to_string_lossy()
                            .contains(filter_pattern)
                        {
                            files.push(entry_path);
                        }
                    } else {
                        files.push(entry_path);
                    }
                }
            }
        }
    }

    // Sort files for consistent ordering
    files.sort();

    Ok(files)
}
