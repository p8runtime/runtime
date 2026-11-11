//! Time p8-test-suite's perf snippets under PHP and under p8.
//!
//!   php-perf --suite ../p8-test-suite --php target/release/p8 --reference docker:php:8.4-cli [--write]
//!
//! Each `perf/NAME.php` times its own work with hrtime and prints one JSON
//! line, `{"check": …, "ns": …}` (perf/_bench.php), so process start-up and
//! a container's do not count. A snippet's time counts only when p8's check
//! is PHP's. Runs alternate PHP and p8; each time is the median of --runs.
//! --write records the run in the suite's perf.json, against p8's HEAD.
#![forbid(unsafe_code)] // memory safety is the point of this runtime; see SECURITY.md

use anyhow::{Context, bail, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Parser)]
#[command(name = "php-perf", about = "Time p8-test-suite's perf snippets under PHP and p8")]
struct Cli {
    /// A p8-test-suite checkout (its perf/ snippets)
    #[arg(long)]
    suite: PathBuf,

    /// The p8 binary under test
    #[arg(long)]
    php: PathBuf,

    /// PHP to compare with: a php binary, or docker:<image>
    #[arg(long)]
    reference: String,

    /// Runs of each snippet under each; the time is their median
    #[arg(long, default_value_t = 5)]
    runs: usize,

    /// Write the result to the suite's perf.json (needs a clean p8 checkout)
    #[arg(long)]
    write: bool,
}

/// One run's output: what the work returned, and how long it took.
struct Sample {
    check: Value,
    ns: u64,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let perf = cli.suite.join("perf");
    let mut names: Vec<String> = std::fs::read_dir(&perf)
        .with_context(|| format!("{}", perf.display()))?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".php") && !n.starts_with('_'))
        .map(|n| n.trim_end_matches(".php").to_string())
        .collect();
    names.sort();
    ensure!(!names.is_empty(), "no snippets in {}", perf.display());
    ensure!(cli.runs > 0, "--runs must be at least 1");
    let p8 = std::path::absolute(&cli.php)?;
    let commit = if cli.write {
        let dirty = git(&["status", "--porcelain", "--untracked-files=no"])?;
        ensure!(dirty.is_empty(), "--write records the run against p8's HEAD; commit first:\n{dirty}");
        git(&["rev-parse", "HEAD"])?
    } else {
        git(&["rev-parse", "HEAD"]).unwrap_or_default()
    };

    let php_version = reference_version(&cli.reference)?;
    let mut php: Vec<Vec<Sample>> = names.iter().map(|_| Vec::new()).collect();
    let mut ours: Vec<Vec<Sample>> = names.iter().map(|_| Vec::new()).collect();
    for run in 1..=cli.runs {
        eprintln!("run {run} of {}", cli.runs);
        for (i, sample) in reference_run(&cli.reference, &perf, &names)?.into_iter().enumerate() {
            php[i].push(sample);
        }
        for (i, name) in names.iter().enumerate() {
            let out = Command::new(&p8).arg(format!("{name}.php")).current_dir(&perf).output()?;
            ours[i].push(sample(name, "p8", &out.stdout).with_context(|| String::from_utf8_lossy(&out.stderr).into_owned())?);
        }
    }

    let mut rows = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let same = ours[i].iter().all(|s| s.check == php[i][0].check) && php[i].iter().all(|s| s.check == php[i][0].check);
        let (php_ns, p8_ns) = (median(&php[i]), median(&ours[i]));
        let ratio = p8_ns as f64 / php_ns as f64;
        println!("{name:10} PHP {:8.1} ms   p8 {:8.1} ms   {}", php_ns as f64 / 1e6, p8_ns as f64 / 1e6, if same { format!("{ratio:.2}x") } else { "different output".into() });
        rows.push(json!({ "name": name, "about": about(&perf.join(format!("{name}.php")))?, "php_ns": php_ns, "p8_ns": p8_ns, "same": same }));
    }
    let counted: Vec<f64> = rows.iter().filter(|r| r["same"] == true).map(|r| r["p8_ns"].as_f64().unwrap_or(0.0) / r["php_ns"].as_f64().unwrap_or(1.0)).collect();
    // The geometric mean: a snippet twice as fast counts as much as one twice as slow.
    let geomean = (!counted.is_empty()).then(|| (counted.iter().map(|r| r.ln()).sum::<f64>() / counted.len() as f64).exp());
    if let Some(g) = geomean {
        println!("p8 takes {g:.2}x PHP's time (geometric mean of {} snippets)", counted.len());
    }
    if cli.write {
        let record = json!({
            "p8": commit,
            "php": php_version,
            "reference": cli.reference,
            "machine": machine()?,
            "runs": cli.runs,
            "ran": now()?,
            "geomean": geomean,
            "snippets": rows,
        });
        let path = cli.suite.join("perf.json");
        std::fs::write(&path, serde_json::to_string_pretty(&record)? + "\n")?;
        eprintln!("wrote {}", path.display());
    }
    Ok(())
}

/// One snippet's JSON line.
fn sample(name: &str, who: &str, stdout: &[u8]) -> anyhow::Result<Sample> {
    let text = String::from_utf8_lossy(stdout);
    let line = text.lines().last().unwrap_or_default();
    let v: Value = serde_json::from_str(line).with_context(|| format!("{name} under {who} printed {text:?}"))?;
    let ns = v["ns"].as_u64().with_context(|| format!("{name} under {who}: no ns in {line}"))?;
    Ok(Sample { check: v["check"].clone(), ns })
}

fn median(samples: &[Sample]) -> u64 {
    let mut ns: Vec<u64> = samples.iter().map(|s| s.ns).collect();
    ns.sort_unstable();
    ns[ns.len() / 2]
}

/// Every snippet once under the reference PHP, in order. In docker, the
/// snippets are copied in (a container may not see this checkout's path).
fn reference_run(reference: &str, perf: &Path, names: &[String]) -> anyhow::Result<Vec<Sample>> {
    match reference.strip_prefix("docker:") {
        Some(image) => {
            let script = names.iter().map(|n| format!("php /p/{n}.php")).collect::<Vec<_>>().join(" && ");
            let tar = Command::new("tar").args(["cf", "-", "."]).current_dir(perf).output()?;
            ensure!(tar.status.success(), "tar of {}", perf.display());
            let mut child = Command::new("docker")
                .args(["run", "--rm", "-i", image, "sh", "-c", &format!("mkdir /p && tar xf - -C /p && {script}")])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;
            child.stdin.take().context("docker stdin")?.write_all(&tar.stdout)?;
            let out = child.wait_with_output()?;
            ensure!(out.status.success(), "docker {image}: {}", String::from_utf8_lossy(&out.stderr));
            let text = String::from_utf8_lossy(&out.stdout);
            let lines: Vec<&str> = text.lines().collect();
            ensure!(lines.len() == names.len(), "docker {image} printed {} lines for {} snippets:\n{text}", lines.len(), names.len());
            names.iter().zip(lines).map(|(n, l)| sample(n, "PHP", l.as_bytes())).collect()
        }
        None => names
            .iter()
            .map(|n| {
                let out = Command::new(reference).arg(format!("{n}.php")).current_dir(perf).output()?;
                sample(n, "PHP", &out.stdout)
            })
            .collect(),
    }
}

/// The reference PHP's version (PHP_VERSION).
fn reference_version(reference: &str) -> anyhow::Result<String> {
    let out = match reference.strip_prefix("docker:") {
        Some(image) => Command::new("docker").args(["run", "--rm", image, "php", "-r", "echo PHP_VERSION;"]).output()?,
        None => Command::new(reference).args(["-r", "echo PHP_VERSION;"]).output()?,
    };
    ensure!(out.status.success(), "{reference}: {}", String::from_utf8_lossy(&out.stderr));
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// What a snippet measures: its first comment, its lines joined.
fn about(file: &Path) -> anyhow::Result<String> {
    let text = std::fs::read_to_string(file)?;
    let comment: Vec<&str> = text.lines().skip_while(|l| !l.starts_with("// ")).map_while(|l| l.strip_prefix("// ")).collect();
    Ok(comment.join(" "))
}

/// Where it ran: the host's name, OS and CPU architecture.
fn machine() -> anyhow::Result<String> {
    let host = Command::new("hostname").arg("-s").output()?;
    let host = String::from_utf8_lossy(&host.stdout).trim().to_lowercase();
    Ok(format!("{host} ({} {})", std::env::consts::OS, std::env::consts::ARCH))
}

fn now() -> anyhow::Result<String> {
    let out = Command::new("date").args(["-u", "+%Y-%m-%dT%H:%M:%SZ"]).output()?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn git(args: &[&str]) -> anyhow::Result<String> {
    let out = Command::new("git").args(args).output()?;
    if !out.status.success() {
        bail!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
