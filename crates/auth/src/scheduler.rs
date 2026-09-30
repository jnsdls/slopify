use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::Mutex;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

pub type Task = Box<dyn FnOnce() + Send>;

/// The clock and timers the token store runs on: one background thread in the app, fake time in
/// tests.
pub trait Scheduler: Send + Sync {
    fn now(&self) -> Instant;
    /// Runs `task` once `delay` has passed. There is no cancel: the store ignores stale tasks.
    fn after(&self, delay: Duration, task: Task);
}

/// Runs every task on one worker thread, which exits when the scheduler is dropped.
pub struct ThreadScheduler {
    tx: Mutex<Sender<(Instant, Task)>>,
}

impl ThreadScheduler {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<(Instant, Task)>();
        thread::Builder::new()
            .name("slopify-auth-timer".into())
            .spawn(move || {
                // Ordered by deadline, then by arrival so equal deadlines run in order.
                let mut queue: BinaryHeap<Reverse<(Instant, u64)>> = BinaryHeap::new();
                let mut tasks = std::collections::HashMap::<u64, Task>::new();
                let mut seq = 0u64;
                loop {
                    let next = queue.peek().map(|Reverse((at, _))| *at);
                    let received = match next {
                        None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
                        Some(at) => rx.recv_timeout(at.saturating_duration_since(Instant::now())),
                    };
                    match received {
                        Ok((at, task)) => {
                            seq += 1;
                            queue.push(Reverse((at, seq)));
                            tasks.insert(seq, task);
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            while let Some(Reverse((at, id))) = queue.peek().copied() {
                                if at > Instant::now() {
                                    break;
                                }
                                queue.pop();
                                if let Some(task) = tasks.remove(&id) {
                                    task();
                                }
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            })
            .expect("spawn the auth timer thread");
        Self { tx: Mutex::new(tx) }
    }
}

impl Default for ThreadScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl Scheduler for ThreadScheduler {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn after(&self, delay: Duration, task: Task) {
        let _ = self.tx.lock().unwrap().send((Instant::now() + delay, task));
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;

    /// Time that only moves when a test says so, like vitest's fake timers.
    pub struct FakeScheduler {
        base: Instant,
        inner: Mutex<Inner>,
    }

    struct Inner {
        elapsed: Duration,
        seq: u64,
        queue: Vec<(Duration, u64, Task)>,
    }

    impl FakeScheduler {
        pub fn new() -> Self {
            Self {
                base: Instant::now(),
                inner: Mutex::new(Inner {
                    elapsed: Duration::ZERO,
                    seq: 0,
                    queue: Vec::new(),
                }),
            }
        }

        /// Moves the clock without firing anything, like `vi.setSystemTime`.
        pub fn set_elapsed(&self, by: Duration) {
            self.inner.lock().unwrap().elapsed += by;
        }

        /// Moves the clock forward, firing each due task at its own deadline, in order.
        pub fn advance(&self, by: Duration) {
            let target = self.inner.lock().unwrap().elapsed + by;
            loop {
                let task = {
                    let mut inner = self.inner.lock().unwrap();
                    let due = inner
                        .queue
                        .iter()
                        .enumerate()
                        .filter(|(_, (at, _, _))| *at <= target)
                        .min_by_key(|(_, (at, seq, _))| (*at, *seq))
                        .map(|(i, _)| i);
                    match due {
                        Some(i) => {
                            let (at, _, task) = inner.queue.remove(i);
                            inner.elapsed = inner.elapsed.max(at);
                            task
                        }
                        None => {
                            inner.elapsed = target;
                            return;
                        }
                    }
                };
                // Run outside the lock: the task usually schedules the next one.
                task();
            }
        }
    }

    impl Scheduler for FakeScheduler {
        fn now(&self) -> Instant {
            self.base + self.inner.lock().unwrap().elapsed
        }

        fn after(&self, delay: Duration, task: Task) {
            let mut inner = self.inner.lock().unwrap();
            inner.seq += 1;
            let (at, seq) = (inner.elapsed + delay, inner.seq);
            inner.queue.push((at, seq, task));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn thread_scheduler_runs_tasks_in_deadline_order() {
        let s = ThreadScheduler::new();
        let (tx, rx) = mpsc::channel();
        for (delay, label) in [(40, "late"), (0, "now"), (20, "soon")] {
            let tx = tx.clone();
            s.after(
                Duration::from_millis(delay),
                Box::new(move || tx.send(label).unwrap()),
            );
        }
        let order: Vec<_> = (0..3)
            .map(|_| rx.recv_timeout(Duration::from_secs(2)).unwrap())
            .collect();
        assert_eq!(order, ["now", "soon", "late"]);
    }
}
