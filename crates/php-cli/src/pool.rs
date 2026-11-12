//! `p8 serve`'s workers (APS 8): threads, each with state of its own (an
//! isolate is made on the thread that runs it), taking jobs from one queue.
//! The first idle worker takes the next job, so a job never waits behind a
//! busy worker while another is idle; when every worker is busy, it goes to
//! the first one to finish. A job that panics costs its worker's state:
//! that state is dropped and made again, and the worker goes on.

use std::sync::{Arc, Mutex, mpsc};

/// A worker's native stack: as deep as the CLI's (deep PHP recursion).
const STACK: usize = 64 * 1024 * 1024;

/// Start `n` workers: each makes its state with `make`, then runs every job
/// it takes from `jobs` with `run`, until `jobs` has no sender left.
pub(crate) fn start<J, S>(n: usize, jobs: mpsc::Receiver<J>, make: impl Fn() -> S + Send + Sync + 'static, run: impl Fn(&mut S, J) + Send + Sync + 'static) -> Vec<std::thread::JoinHandle<()>>
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
                    let mut state = make();
                    loop {
                        // Taken while holding the queue: one idle worker
                        // waits on it, the others wait their turn.
                        let job = match jobs.lock().unwrap_or_else(|e| e.into_inner()).recv() {
                            Ok(job) => job,
                            Err(_) => return,
                        };
                        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&mut state, job))).is_err() {
                            // What the job left of the state cannot be
                            // trusted: a new one replaces it.
                            state = make();
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
}
