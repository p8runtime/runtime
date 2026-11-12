//! `p8 serve`'s workers (APS 8): threads, each with state of its own (an
//! isolate is made on the thread that runs it), taking jobs from one queue.
//! The first idle worker takes the next job, so a job never waits behind a
//! busy worker while another is idle; when every worker is busy, it goes to
//! the first one to finish. A job that panics, or that says its worker is
//! done for (a request past the hard memory limit), costs its worker's
//! state: that state is dropped, all of it, before a new one is made, and
//! the worker goes on with the new one. The other workers never notice.

use std::sync::{Arc, Mutex, mpsc};

/// A worker's native stack: as deep as the CLI's (deep PHP recursion).
const STACK: usize = 64 * 1024 * 1024;

/// Start `n` workers: each makes its state with `make`, then runs every job
/// it takes from `jobs` with `run`, until `jobs` has no sender left. `run`
/// returns true when the state is to be replaced.
pub(crate) fn start<J, S>(n: usize, jobs: mpsc::Receiver<J>, make: impl Fn() -> S + Send + Sync + 'static, run: impl Fn(&mut S, J) -> bool + Send + Sync + 'static) -> Vec<std::thread::JoinHandle<()>>
where
    J: Send + 'static,
{
    let jobs = Arc::new(Mutex::new(jobs));
    let make = Arc::new(make);
    let run = Arc::new(run);
    (0..n)
        .map(|i| {
            let (jobs, make, run) = (Arc::clone(&jobs), Arc::clone(&make), Arc::clone(&run));
            std::thread::Builder::new()
                .name(format!("p8 worker {i}"))
                .stack_size(STACK)
                .spawn(move || {
                    let mut state = Some(make());
                    loop {
                        // Taken while holding the queue: one idle worker
                        // waits on it, the others wait their turn.
                        let job = match jobs.lock().unwrap_or_else(|e| e.into_inner()).recv() {
                            Ok(job) => job,
                            Err(_) => return,
                        };
                        let current = state.as_mut().expect("a worker's state is made before it takes a job");
                        let replace = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(current, job))).unwrap_or(true);
                        if replace {
                            // What the job left of the state cannot be
                            // trusted (or it is done for): it goes first,
                            // its memory with it, then a new one is made.
                            drop(state.take());
                            state = Some(make());
                        }
                    }
                })
                .expect("a worker thread")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, mpsc};

    /// A job that panics: its worker's state is made again and the worker
    /// takes the next jobs; every other job runs, on states that never saw
    /// the failed one.
    #[test]
    fn a_worker_whose_job_panics_is_replaced() {
        static MADE: AtomicUsize = AtomicUsize::new(0);
        let (tx, rx) = mpsc::channel::<u32>();
        let (done, results) = mpsc::channel::<(u32, Vec<u32>)>();
        let done = Arc::new(std::sync::Mutex::new(done));
        let workers = super::start(
            1,
            rx,
            || {
                MADE.fetch_add(1, Ordering::SeqCst);
                Vec::new()
            },
            move |seen: &mut Vec<u32>, job| {
                seen.push(job);
                if job == 3 {
                    panic!("job 3 fails");
                }
                done.lock().expect("results").send((job, seen.clone())).expect("send");
                false
            },
        );
        for job in 1..=5 {
            tx.send(job).expect("queue");
        }
        drop(tx);
        for w in workers {
            w.join().expect("the worker ends when the queue does");
        }
        let got: Vec<(u32, Vec<u32>)> = results.try_iter().collect();
        assert_eq!(got, [(1, vec![1]), (2, vec![1, 2]), (4, vec![4]), (5, vec![4, 5])]);
        assert_eq!(MADE.load(Ordering::SeqCst), 2, "made once, and once more after the panic");
    }

    /// A job that says its worker is done for: the state it ran on is
    /// dropped before the next is made (the old state's memory goes first),
    /// and the next job runs on the new one.
    #[test]
    fn a_worker_whose_job_says_so_is_replaced_dropping_the_old_state_first() {
        static LIVE: AtomicUsize = AtomicUsize::new(0);
        static MOST: AtomicUsize = AtomicUsize::new(0);
        struct State(Vec<u32>);
        impl Drop for State {
            fn drop(&mut self) {
                LIVE.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let (tx, rx) = mpsc::channel::<u32>();
        let (done, results) = mpsc::channel::<(u32, Vec<u32>)>();
        let done = Arc::new(std::sync::Mutex::new(done));
        let workers = super::start(
            1,
            rx,
            || {
                let live = LIVE.fetch_add(1, Ordering::SeqCst) + 1;
                MOST.fetch_max(live, Ordering::SeqCst);
                State(Vec::new())
            },
            move |state: &mut State, job| {
                state.0.push(job);
                done.lock().expect("results").send((job, state.0.clone())).expect("send");
                job == 2
            },
        );
        for job in 1..=4 {
            tx.send(job).expect("queue");
        }
        drop(tx);
        for w in workers {
            w.join().expect("the worker ends when the queue does");
        }
        let got: Vec<(u32, Vec<u32>)> = results.try_iter().collect();
        assert_eq!(got, [(1, vec![1]), (2, vec![1, 2]), (3, vec![3]), (4, vec![3, 4])]);
        assert_eq!(MOST.load(Ordering::SeqCst), 1, "never two states at once: the old one went before the new one was made");
    }
}
