//! p8-test-suite (github.com/addiscorp/p8-test-suite): the php-src tests real
//! PHP passes (`target.txt`), p8's last recorded state for each
//! (`results.txt`, `run.json`), and the php-src release they come from
//! (`php-src.json`). `php-test --suite` runs against it, compares a run with
//! the recorded results, and can write a run back in the same format.

use crate::executor::TestResult;
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

/// A target test's state, as `results.txt` records it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    /// p8's output matches the test's expectation.
    Pass,
    /// p8 ran the test and the output differs, it errored, or it timed out.
    Fail,
    /// p8 did not run it (its --SKIPIF-- skipped it, or the file is missing).
    Skip,
}

impl State {
    pub fn of(result: &TestResult) -> State {
        match result {
            TestResult::Passed => State::Pass,
            TestResult::Failed { .. } | TestResult::ExpectedFailure { .. } | TestResult::Error { .. } => State::Fail,
            TestResult::Skipped { .. } => State::Skip,
        }
    }

    fn word(self) -> &'static str {
        match self {
            State::Pass => "pass",
            State::Fail => "fail",
            State::Skip => "skip",
        }
    }

    pub fn parse(word: &str) -> Option<State> {
        match word {
            "pass" => Some(State::Pass),
            "fail" => Some(State::Fail),
            "skip" => Some(State::Skip),
            _ => None,
        }
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.word())
    }
}

/// A checkout of p8-test-suite, read and checked.
pub struct Suite {
    /// Target test paths, relative to php-src, in `target.txt` order.
    pub tests: Vec<String>,
    /// The recorded state of each test, in the same order.
    pub recorded: Vec<State>,
    /// The php-src release: its tag and commit.
    pub php_tag: String,
    pub php_commit: String,
    /// The PHP version the target was made with (`run.json`'s `php`).
    pub php: String,
}

impl Suite {
    pub fn load(dir: &Path) -> Result<Suite, String> {
        let read = |name: &str| std::fs::read_to_string(dir.join(name)).map_err(|e| format!("{}: {e}", dir.join(name).display()));
        let target = read("target.txt")?;
        let results = read("results.txt")?;
        let pin: serde_json::Value = serde_json::from_str(&read("php-src.json")?).map_err(|e| format!("php-src.json: {e}"))?;
        let run: serde_json::Value = serde_json::from_str(&read("run.json")?).map_err(|e| format!("run.json: {e}"))?;
        let field = |v: &serde_json::Value, file: &str, key: &str| {
            v[key].as_str().map(str::to_owned).ok_or_else(|| format!("{file}: no \"{key}\""))
        };
        let (tests, recorded) = parse_results(&target, &results)?;
        Ok(Suite {
            tests,
            recorded,
            php_tag: field(&pin, "php-src.json", "tag")?,
            php_commit: field(&pin, "php-src.json", "commit")?,
            php: field(&run, "run.json", "php")?,
        })
    }
}

/// `results.txt` must hold one line per `target.txt` line, in the same order.
pub fn parse_results(target: &str, results: &str) -> Result<(Vec<String>, Vec<State>), String> {
    let tests: Vec<String> = target.lines().filter(|l| !l.is_empty()).map(str::to_owned).collect();
    let lines: Vec<&str> = results.lines().filter(|l| !l.is_empty()).collect();
    if lines.len() != tests.len() {
        return Err(format!("results.txt has {} lines, target.txt {}", lines.len(), tests.len()));
    }
    let mut recorded = Vec::with_capacity(tests.len());
    for (i, (line, test)) in lines.iter().zip(&tests).enumerate() {
        let (word, path) = line.split_once(' ').ok_or_else(|| format!("results.txt line {}: \"{line}\"", i + 1))?;
        let state = State::parse(word).ok_or_else(|| format!("results.txt line {}: bad state \"{word}\"", i + 1))?;
        if path != test {
            return Err(format!("results.txt line {}: \"{path}\" where target.txt has \"{test}\"", i + 1));
        }
        recorded.push(state);
    }
    Ok((tests, recorded))
}

/// The area a test is reported under, as the site draws it: Zend's
/// subfolders, `tests/<dir>`, each extension, standard's subfolders, `sapi/<name>`.
pub fn area(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        ["Zend", _, _] => "Zend".to_owned(),
        ["Zend", _, dir, ..] => format!("Zend/{dir}"),
        ["ext", "standard", _, dir, _, ..] => format!("standard/{dir}"),
        ["ext", name, ..] | ["tests", name, ..] | ["sapi", name, ..] => (*name).to_owned(),
        _ => parts[0].to_owned(),
    }
}

/// How a run compares with the recorded results, over the tests it ran.
#[derive(Default)]
pub struct Comparison {
    /// Tests that pass now and did not before.
    pub gained: Vec<String>,
    /// Tests that passed before and do not now.
    pub lost: Vec<(String, State)>,
    /// Per area: (passing before, passing now, tests run).
    pub areas: BTreeMap<String, (usize, usize, usize)>,
    pub ran: usize,
    pub pass_before: usize,
    pub pass_now: usize,
}

/// `run` holds (index into the suite, state now) for each test that ran.
pub fn compare(suite: &Suite, run: &[(usize, State)]) -> Comparison {
    let mut c = Comparison::default();
    for &(i, now) in run {
        let before = suite.recorded[i];
        let path = &suite.tests[i];
        let entry = c.areas.entry(area(path)).or_default();
        entry.2 += 1;
        c.ran += 1;
        if before == State::Pass {
            entry.0 += 1;
            c.pass_before += 1;
        }
        if now == State::Pass {
            entry.1 += 1;
            c.pass_now += 1;
        }
        match (before, now) {
            (State::Pass, State::Pass) => {}
            (State::Pass, now) => c.lost.push((path.clone(), now)),
            (_, State::Pass) => c.gained.push(path.clone()),
            _ => {}
        }
    }
    c
}

/// Writes `results.txt` and `run.json` for a run over every target test.
pub fn write(dir: &Path, suite: &Suite, states: &[State], p8_commit: &str, ran: &str) -> std::io::Result<()> {
    assert_eq!(states.len(), suite.tests.len(), "a suite write needs every target test");
    let mut results = String::new();
    for (state, test) in states.iter().zip(&suite.tests) {
        results.push_str(state.word());
        results.push(' ');
        results.push_str(test);
        results.push('\n');
    }
    let count = |s: State| states.iter().filter(|&&x| x == s).count();
    let run = serde_json::json!({
        "php": suite.php,
        "p8": p8_commit,
        "ran": ran,
        "tests": states.len(),
        "pass": count(State::Pass),
        "fail": count(State::Fail),
        "skip": count(State::Skip),
    });
    std::fs::write(dir.join("results.txt"), results)?;
    std::fs::write(dir.join("run.json"), format!("{}\n", serde_json::to_string_pretty(&run).expect("json")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suite(tests: &[&str], recorded: &[State]) -> Suite {
        Suite {
            tests: tests.iter().map(|t| t.to_string()).collect(),
            recorded: recorded.to_vec(),
            php_tag: "php-8.4.27".into(),
            php_commit: "0".into(),
            php: "8.4.27".into(),
        }
    }

    #[test]
    fn results_line_up_with_the_target() {
        let (tests, states) = parse_results("a.phpt\nb.phpt\n", "pass a.phpt\nskip b.phpt\n").unwrap();
        assert_eq!(tests, ["a.phpt", "b.phpt"]);
        assert_eq!(states, [State::Pass, State::Skip]);
        assert!(parse_results("a.phpt\nb.phpt\n", "pass a.phpt\n").unwrap_err().contains("1 lines, target.txt 2"));
        assert!(parse_results("a.phpt\n", "pass b.phpt\n").unwrap_err().contains("where target.txt has"));
        assert!(parse_results("a.phpt\n", "flaky a.phpt\n").unwrap_err().contains("bad state"));
    }

    #[test]
    fn areas_follow_php_src_folders() {
        assert_eq!(area("Zend/tests/001.phpt"), "Zend");
        assert_eq!(area("Zend/tests/enum/a.phpt"), "Zend/enum");
        assert_eq!(area("Zend/tests/enum/deep/a.phpt"), "Zend/enum");
        assert_eq!(area("ext/standard/tests/array/a.phpt"), "standard/array");
        assert_eq!(area("ext/standard/tests/a.phpt"), "standard");
        assert_eq!(area("ext/zlib/tests/a.phpt"), "zlib");
        assert_eq!(area("tests/lang/a.phpt"), "lang");
        assert_eq!(area("sapi/cli/tests/a.phpt"), "cli");
    }

    #[test]
    fn a_run_is_compared_with_the_recorded_results() {
        let s = suite(
            &["ext/zlib/tests/a.phpt", "ext/zlib/tests/b.phpt", "tests/lang/c.phpt", "tests/lang/d.phpt"],
            &[State::Pass, State::Fail, State::Pass, State::Skip],
        );
        // d did not run: it counts nowhere.
        let c = compare(&s, &[(0, State::Fail), (1, State::Pass), (2, State::Pass)]);
        assert_eq!(c.gained, ["ext/zlib/tests/b.phpt"]);
        assert_eq!(c.lost, [("ext/zlib/tests/a.phpt".to_string(), State::Fail)]);
        assert_eq!((c.ran, c.pass_before, c.pass_now), (3, 2, 2));
        assert_eq!(c.areas["zlib"], (1, 1, 2));
        assert_eq!(c.areas["lang"], (1, 1, 1));
    }

    #[test]
    fn states_come_from_test_results() {
        assert_eq!(State::of(&TestResult::Passed), State::Pass);
        let failed = TestResult::Failed { expected: "a".into(), actual: "b".into() };
        assert_eq!(State::of(&failed), State::Fail);
        assert_eq!(State::of(&TestResult::ExpectedFailure { reason: String::new() }), State::Fail);
        assert_eq!(State::of(&TestResult::Error { error: String::new() }), State::Fail);
        assert_eq!(State::of(&TestResult::Skipped { reason: String::new() }), State::Skip);
    }

    #[test]
    fn a_written_run_reads_back() {
        let dir = std::env::current_dir().unwrap().join(".tmp/suite-write-test");
        std::fs::create_dir_all(&dir).unwrap();
        let s = suite(&["a.phpt", "b.phpt", "c.phpt"], &[State::Skip; 3]);
        write(&dir, &s, &[State::Pass, State::Fail, State::Skip], "abc", "2026-10-24T00:00:00Z").unwrap();
        let results = std::fs::read_to_string(dir.join("results.txt")).unwrap();
        assert_eq!(results, "pass a.phpt\nfail b.phpt\nskip c.phpt\n");
        let (_, states) = parse_results("a.phpt\nb.phpt\nc.phpt\n", &results).unwrap();
        assert_eq!(states, [State::Pass, State::Fail, State::Skip]);
        let run: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("run.json")).unwrap()).unwrap();
        assert_eq!((run["tests"].as_u64(), run["pass"].as_u64(), run["fail"].as_u64(), run["skip"].as_u64()), (Some(3), Some(1), Some(1), Some(1)));
        assert_eq!(run["p8"], "abc");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
