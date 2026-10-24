//! Which tests run when, as php-src's run-tests.php decides it in parallel
//! mode: a worker takes a batch of consecutive tests (at most 32, fewer when
//! there are few tests per worker) and runs it in order, so neighbouring
//! tests that share files don't race. While a batch holding a conflict key
//! runs, no other test with that key starts; tests with the key `all` run
//! last, one at a time, with nothing beside them.

use std::collections::{HashMap, VecDeque};

const MAX_BATCH: usize = 32;

pub struct Schedule {
    queue: VecDeque<usize>,
    conflicts: Vec<Vec<String>>,
    /// Tests waiting for a key to be released, per key.
    waiting: HashMap<String, Vec<usize>>,
    /// Keys held by running batches.
    active: HashMap<String, usize>,
    /// The `all` tests, run after everything else.
    last: VecDeque<usize>,
    running: usize,
    batch: usize,
}

impl Schedule {
    /// `conflicts[i]` is test i's conflict keys; tests run in index order.
    pub fn new(mut conflicts: Vec<Vec<String>>, workers: usize) -> Schedule {
        // A key listed twice is held once.
        for keys in &mut conflicts {
            keys.sort();
            keys.dedup();
        }
        let (last, queue): (Vec<usize>, Vec<usize>) =
            (0..conflicts.len()).partition(|&i| conflicts[i].iter().any(|k| k == "all"));
        let batch = MAX_BATCH.min(queue.len().div_ceil(workers.max(1)).max(1));
        Schedule {
            queue: queue.into(),
            conflicts,
            waiting: HashMap::new(),
            active: HashMap::new(),
            last: last.into(),
            running: 0,
            batch,
        }
    }

    /// The next batch to run. `None` means nothing can start now: either
    /// everything has been handed out, or what's left waits on running
    /// batches (call again after `done`).
    pub fn next(&mut self) -> Option<Vec<usize>> {
        let mut batch = Vec::new();
        while batch.len() < self.batch {
            let Some(i) = self.queue.pop_front() else { break };
            match self.conflicts[i].iter().find(|k| self.active.contains_key(*k)) {
                Some(key) => self.waiting.entry(key.clone()).or_default().push(i),
                None => {
                    for key in &self.conflicts[i] {
                        *self.active.entry(key.clone()).or_default() += 1;
                    }
                    batch.push(i);
                }
            }
        }
        if batch.is_empty() && self.running == 0 && self.waiting.is_empty() {
            // Only the `all` tests are left, and nothing else is running.
            if let Some(i) = self.last.pop_front() {
                for key in &self.conflicts[i] {
                    *self.active.entry(key.clone()).or_default() += 1;
                }
                batch.push(i);
            }
        }
        if batch.is_empty() {
            return None;
        }
        self.running += 1;
        Some(batch)
    }

    /// A batch from `next` finished: release its keys.
    pub fn done(&mut self, batch: &[usize]) {
        self.running -= 1;
        for &i in batch {
            for key in &self.conflicts[i] {
                let held = self.active.get_mut(key).expect("held by this batch");
                *held -= 1;
                if *held == 0 {
                    self.active.remove(key);
                    // In order, ahead of what's queued, as run-tests does.
                    for t in self.waiting.remove(key).unwrap_or_default().into_iter().rev() {
                        self.queue.push_front(t);
                    }
                }
            }
        }
    }

    /// Whether every test has been handed out.
    pub fn finished(&self) -> bool {
        self.queue.is_empty() && self.waiting.is_empty() && self.last.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(list: &[&[&str]]) -> Vec<Vec<String>> {
        list.iter().map(|k| k.iter().map(|s| s.to_string()).collect()).collect()
    }

    #[test]
    fn consecutive_tests_share_a_batch() {
        let mut s = Schedule::new(vec![vec![]; 10], 4);
        // 10 tests over 4 workers: batches of 3, in order.
        assert_eq!(s.next(), Some(vec![0, 1, 2]));
        assert_eq!(s.next(), Some(vec![3, 4, 5]));
        assert_eq!(s.next(), Some(vec![6, 7, 8]));
        assert_eq!(s.next(), Some(vec![9]));
        assert_eq!(s.next(), None);
        assert!(s.finished());
    }

    #[test]
    fn batches_are_at_most_32() {
        let mut s = Schedule::new(vec![vec![]; 1000], 4);
        assert_eq!(s.next().unwrap().len(), 32);
    }

    #[test]
    fn a_held_key_holds_back_its_other_tests() {
        let mut s = Schedule::new(keys(&[&["db"], &[], &["db"], &[]]), 2);
        // Batches of 2: test 2 waits while test 0 holds "db".
        let first = s.next().unwrap();
        assert_eq!(first, [0, 1]);
        assert_eq!(s.next(), Some(vec![3]));
        assert_eq!(s.next(), None);
        assert!(!s.finished());
        s.done(&first);
        assert_eq!(s.next(), Some(vec![2]));
        assert!(s.finished());
    }

    #[test]
    fn all_tests_run_last_and_alone() {
        let mut s = Schedule::new(keys(&[&["all", "server"], &[], &["all"], &[]]), 1);
        let batch = s.next().unwrap();
        assert_eq!(batch, [1, 3]);
        // Nothing beside a running batch.
        assert_eq!(s.next(), None);
        s.done(&batch);
        let a = s.next().unwrap();
        assert_eq!(a, [0]);
        assert_eq!(s.next(), None);
        s.done(&a);
        assert_eq!(s.next(), Some(vec![2]));
        assert!(s.finished());
    }

    #[test]
    fn a_key_listed_twice_is_released() {
        let mut s = Schedule::new(keys(&[&["db", "db"], &["db"]]), 2);
        let first = s.next().unwrap();
        assert_eq!(first, [0]);
        s.done(&first);
        assert_eq!(s.next(), Some(vec![1]));
    }
}
