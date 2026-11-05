//! Runs a parsed .phpt test the way php-src's run-tests.php does: each
//! section is written next to the test as `<name>.php` / `.skip.php` /
//! `.clean.php` and executed by the `php` binary in a child process, so a
//! crash or hang fails one test instead of the whole run.
//!
//! Each run is its own process group, watched while it runs: past its
//! memory or process limit, or its timeout, the whole group is killed, and
//! whatever it left running in the background is killed when it exits. A
//! runaway test (unbounded allocation, a command that never ends) froze the
//! build machine on 23 Oct before this.

use crate::phpt::matcher::{ExpectationType, match_output};
use crate::phpt::parser::PhptTest;
use std::collections::HashSet;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub enum TestResult {
    Passed,
    Failed { expected: String, actual: String },
    /// Failed, but the test is marked `--XFAIL--` in php-src.
    ExpectedFailure { reason: String },
    Skipped { reason: String },
    Error { error: String },
}

/// Output kept per test run; anything past this is discarded.
const MAX_OUTPUT: usize = 1 << 20;

/// The settings php-src's run-tests.php gives every test (its
/// `$ini_overwrites`, as of the target's release, 8.6); a test's --INI--
/// section comes after and wins.
const RUN_TESTS_INI: &[(&str, &str)] = &[
    ("output_handler", ""),
    ("open_basedir", ""),
    ("disable_functions", ""),
    ("output_buffering", "Off"),
    // run-tests.php passes E_ALL as the PHP running it has it: 30719 (8.4+).
    ("error_reporting", "30719"),
    // (8.5+) a test that wants fatal errors' traces asks for them.
    ("fatal_error_backtraces", "Off"),
    ("display_errors", "1"),
    ("display_startup_errors", "1"),
    ("error_include_args", "0"),
    ("log_errors", "0"),
    ("html_errors", "0"),
    ("track_errors", "0"),
    // (report_memleaks=1 is gone: the directive is deprecated since 8.5.)
    ("report_zend_debug", "0"),
    ("docref_root", ""),
    ("docref_ext", ".html"),
    ("error_prepend_string", ""),
    ("error_append_string", ""),
    ("auto_prepend_file", ""),
    ("auto_append_file", ""),
    ("ignore_repeated_errors", "0"),
    ("precision", "14"),
    ("serialize_precision", "-1"),
    ("memory_limit", "128M"),
    ("zend.assertions", "1"),
    ("zend.exception_ignore_args", "0"),
    ("zend.exception_string_param_max_len", "15"),
    ("short_open_tag", "0"),
    ("date.timezone", "UTC"),
];

/// What one test run (its php process and everything it starts) may use.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub timeout: Duration,
    /// Resident memory of the whole process group, in bytes.
    pub memory: u64,
    /// Processes in the group at once.
    pub processes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { timeout: Duration::from_secs(10), memory: 1 << 30, processes: 32 }
    }
}

/// How often a running test's memory and processes are checked.
const WATCH_EVERY: Duration = Duration::from_millis(200);

pub struct PhptExecutor {
    php: PathBuf,
    limits: Limits,
    loaded_extensions: HashSet<String>,
}

struct Output {
    text: String,
    /// Why the run was killed, if it was.
    killed: Option<String>,
}

impl PhptExecutor {
    pub fn new(php: PathBuf, limits: Limits) -> Self {
        // Absolute: each test runs in its own directory, and tests start it again.
        let php = std::fs::canonicalize(&php).unwrap_or(php);
        Self {
            php,
            limits,
            loaded_extensions: HashSet::new(),
        }
    }

    /// Ask the binary which of `names` it has loaded (one `extension_loaded`
    /// probe), so tests needing missing extensions are skipped, not failed.
    pub fn probe_extensions<'a>(
        &mut self,
        names: impl IntoIterator<Item = &'a str>,
        scratch: &Path,
    ) -> Result<(), String> {
        let names: Vec<&str> = names.into_iter().map(Self::module_name).collect();
        let mut code = String::from("<?php\necho 'Core', \"\\n\";\n");
        for name in &names {
            code.push_str(&format!("if (extension_loaded('{}')) echo '{}', \"\\n\";\n", name, name));
        }
        let probe = scratch.join("php-test-extensions.php");
        // Every run loads Core: an empty answer means the probe did not
        // finish (a loaded machine), and would skip every --EXTENSIONS-- test.
        for _ in 0..3 {
            let out = self.run_section(&PhptTest::default(), &probe, &code)?;
            if out.killed.is_none() && !out.text.trim().is_empty() {
                self.loaded_extensions = out.text.lines().map(str::to_string).collect();
                return Ok(());
            }
        }
        Err("the extension probe produced no output".into())
    }

    /// The module name an --EXTENSIONS-- entry stands for: run-tests.php
    /// loads `opcache` as the Zend extension "Zend OPcache".
    fn module_name(extension: &str) -> &str {
        match extension {
            "opcache" => "Zend OPcache",
            other => other,
        }
    }

    pub fn run_test(&self, test: &PhptTest) -> TestResult {
        // php-src's open_basedir suite counts on open_basedir to stop it:
        // without it, it chmods, unlinks and writes outside its directory
        // (it made php-src/tests unreadable).
        if test.sections.file.contains("open_basedir.inc") && test.sections.ini.iter().any(|(k, v)| k.trim() == "open_basedir" && !v.trim().is_empty()) {
            return TestResult::Skipped { reason: "open_basedir is not enforced yet".to_string() };
        }
        if let Some(section) = test.sections.unsupported.first() {
            return TestResult::Skipped {
                reason: format!("needs --{}--", section),
            };
        }
        if let Some(ext) = test
            .sections
            .extensions
            .iter()
            .find(|e| !self.loaded_extensions.contains(Self::module_name(e)))
        {
            return TestResult::Skipped {
                reason: format!("extension {} not loaded", ext),
            };
        }
        let base = match std::path::absolute(&test.file_path) {
            Ok(path) => path.with_extension(""),
            Err(e) => return TestResult::Error { error: e.to_string() },
        };

        if let Some(code) = &test.sections.skipif {
            let path = sibling(&base, "skip.php");
            match self.run_section(test, &path, code) {
                Ok(out) => {
                    let trimmed = out.text.trim_start();
                    if trimmed.len() >= 4 && trimmed[..4].eq_ignore_ascii_case("skip") {
                        return TestResult::Skipped {
                            reason: trimmed[4..].trim().to_string(),
                        };
                    }
                }
                Err(e) => return TestResult::Error { error: e },
            }
        }

        let path = sibling(&base, "php");
        let result = match self.run_section(test, &path, &test.sections.file) {
            Ok(Output { text, killed: Some(why) }) => TestResult::Failed {
                expected: "(completion)".into(),
                actual: format!("{why}\n{text}"),
            },
            Ok(out) => self.judge(test, out.text),
            Err(e) => TestResult::Error { error: e },
        };

        if let Some(code) = &test.sections.clean {
            let _ = self.run_section(test, &sibling(&base, "clean.php"), code);
        }
        result
    }

    fn judge(&self, test: &PhptTest, actual: String) -> TestResult {
        let s = &test.sections;
        let expected = if let Some(e) = &s.expect {
            ExpectationType::Exact(e.clone())
        } else if let Some(e) = &s.expectf {
            ExpectationType::Format(e.clone())
        } else if let Some(e) = &s.expectregex {
            ExpectationType::Regex(e.clone())
        } else {
            return TestResult::Error {
                error: "no EXPECT/EXPECTF/EXPECTREGEX section".into(),
            };
        };
        if match_output(&actual, expected.clone()) {
            return TestResult::Passed;
        }
        if let Some(reason) = &s.xfail {
            return TestResult::ExpectedFailure {
                reason: reason.clone(),
            };
        }
        let expected = match expected {
            ExpectationType::Exact(s) | ExpectationType::Format(s) | ExpectationType::Regex(s) => s,
        };
        TestResult::Failed { expected, actual }
    }

    /// Write `code` to `path`, run it, delete it; stdout and stderr are merged
    /// in order, like a terminal would show them.
    fn run_section(&self, test: &PhptTest, path: &Path, code: &str) -> Result<Output, String> {
        std::fs::write(path, code).map_err(|e| format!("write {}: {}", path.display(), e))?;
        let out = self.spawn(test, path);
        let _ = std::fs::remove_file(path);
        out
    }

    fn spawn(&self, test: &PhptTest, path: &Path) -> Result<Output, String> {
        let (mut reader, writer) = std::io::pipe().map_err(|e| e.to_string())?;
        let mut cmd = Command::new(&self.php);
        // Its own process group, so everything it starts can be watched and killed.
        cmd.process_group(0);
        let dir = path.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
        let section = test.sections.ini.iter().map(|(k, v)| (k.as_str(), v.as_str()));
        for (name, value) in RUN_TESTS_INI.iter().copied().chain(section) {
            cmd.arg("-d").arg(format!("{}={}", name, value.replace("{PWD}", &dir)));
        }
        cmd.arg(path)
            .stdin(Stdio::null())
            .stdout(writer.try_clone().map_err(|e| e.to_string())?)
            .stderr(writer);
        if let Some(dir) = path.parent() {
            cmd.current_dir(dir);
        }
        if let Some(args) = &test.sections.args {
            cmd.args(args.split_whitespace());
        }
        // As run-tests.php does: tests that start PHP again find it here.
        let php = self.php.to_string_lossy();
        cmd.env("TEST_PHP_EXECUTABLE", &*php)
            .env("TEST_PHP_EXECUTABLE_ESCAPED", format!("'{}'", php.replace('\'', "'\\''")));
        for (key, value) in &test.sections.env {
            cmd.env(key, value);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("spawn {}: {}", self.php.display(), e))?;
        // `cmd` holds the write ends; drop it so EOF arrives when the child exits.
        drop(cmd);

        // Keep at most MAX_OUTPUT bytes but keep draining, so a runaway test
        // can't fill memory or disk; the timeout still ends it.
        let collector = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let mut chunk = [0u8; 64 * 1024];
            while let Ok(n) = reader.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                let room = MAX_OUTPUT.saturating_sub(buf.len());
                buf.extend_from_slice(&chunk[..n.min(room)]);
            }
            buf
        });
        let group = child.id();
        let start = Instant::now();
        let mut watched = start;
        let mut killed = None;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {}
                Err(e) => {
                    kill_group(group);
                    let _ = child.wait();
                    return Err(e.to_string());
                }
            }
            if start.elapsed() >= self.limits.timeout {
                killed = Some(format!("timed out after {:?}", self.limits.timeout));
            } else if watched.elapsed() >= WATCH_EVERY {
                watched = Instant::now();
                killed = self.over_limit(group);
            }
            if killed.is_some() {
                kill_group(group);
                let _ = child.wait();
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        // Whatever it left running in the background goes too; until it
        // does, the output pipe stays open.
        kill_group(group);
        let bytes = collector.join().unwrap_or_default();
        Ok(Output {
            text: String::from_utf8_lossy(&bytes).into_owned(),
            killed,
        })
    }

    /// Why the process group `group` must be stopped, if it is over a limit.
    fn over_limit(&self, group: u32) -> Option<String> {
        let (processes, memory) = group_usage(group)?;
        if memory > self.limits.memory {
            return Some(format!("killed: {} MB resident, over the {} MB limit", memory >> 20, self.limits.memory >> 20));
        }
        if processes > self.limits.processes {
            return Some(format!("killed: {processes} processes, over the limit of {}", self.limits.processes));
        }
        None
    }
}

/// The processes in a process group and their resident memory in bytes,
/// as `ps` reports them.
fn group_usage(group: u32) -> Option<(usize, u64)> {
    let out = Command::new("ps").args(["-A", "-o", "pgid=,rss="]).stderr(Stdio::null()).output().ok()?;
    parse_usage(&String::from_utf8_lossy(&out.stdout), group)
}

/// `ps -o pgid=,rss=` lines (rss in KB) summed for `group`.
fn parse_usage(ps: &str, group: u32) -> Option<(usize, u64)> {
    let mut processes = 0;
    let mut memory = 0u64;
    for line in ps.lines() {
        let mut cols = line.split_whitespace();
        let (Some(pgid), Some(rss)) = (cols.next(), cols.next()) else { continue };
        if pgid.parse() == Ok(group) {
            processes += 1;
            memory += rss.parse::<u64>().unwrap_or(0) * 1024;
        }
    }
    Some((processes, memory))
}

/// SIGKILL every process in the group a test run started (its group id is
/// its php process's pid).
fn kill_group(group: u32) {
    let _ = Command::new("kill")
        .args(["-KILL", "--", &format!("-{group}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn sibling(base: &Path, ext: &str) -> PathBuf {
    let mut name = base.as_os_str().to_owned();
    name.push(".");
    name.push(ext);
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_sums_only_the_group() {
        let ps = "    1  26344\n  700   1000\n  700   2048\n  701  99999\n  bad line\n";
        assert_eq!(parse_usage(ps, 700), Some((2, 3048 * 1024)));
        assert_eq!(parse_usage(ps, 9), Some((0, 0)));
    }
}
