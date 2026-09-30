//! Deadline-based throttle, debounce and the playback error gate. The caller passes `now` and
//! polls when `deadline()` comes due, so the Player core stays free of timers and tests run on
//! made-up instants.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// The leading call goes through at once; later calls collapse into one trailing call per window.
pub struct Throttle<T> {
    window: Duration,
    last: Option<Instant>,
    due: Option<Instant>,
    pending: Option<T>,
}

impl<T> Throttle<T> {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            last: None,
            due: None,
            pending: None,
        }
    }

    /// Returns the value if it should run now; otherwise it waits for `poll`.
    pub fn call(&mut self, now: Instant, value: T) -> Option<T> {
        let open = self
            .last
            .is_none_or(|last| now.duration_since(last) >= self.window);
        if open && self.due.is_none() {
            self.last = Some(now);
            return Some(value);
        }
        self.pending = Some(value);
        if self.due.is_none() {
            self.due = Some(self.last.map_or(now, |last| (last + self.window).max(now)));
        }
        None
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.due
    }

    /// Returns the trailing call once its deadline has passed.
    pub fn poll(&mut self, now: Instant) -> Option<T> {
        if self.due.is_none_or(|due| due > now) {
            return None;
        }
        self.flush(now)
    }

    /// Returns a pending trailing call now.
    pub fn flush(&mut self, now: Instant) -> Option<T> {
        self.due = None;
        let value = self.pending.take()?;
        self.last = Some(now);
        Some(value)
    }

    /// Drops a pending call and reopens the window.
    pub fn cancel(&mut self) {
        self.due = None;
        self.pending = None;
        self.last = None;
    }
}

/// Runs once, `delay` after the last call.
pub struct Debounce<T> {
    delay: Duration,
    due: Option<Instant>,
    pending: Option<T>,
}

impl<T> Debounce<T> {
    pub fn new(delay: Duration) -> Self {
        Self {
            delay,
            due: None,
            pending: None,
        }
    }

    pub fn call(&mut self, now: Instant, value: T) {
        self.pending = Some(value);
        self.due = Some(now + self.delay);
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.due
    }

    pub fn poll(&mut self, now: Instant) -> Option<T> {
        if self.due.is_none_or(|due| due > now) {
            return None;
        }
        self.flush()
    }

    pub fn flush(&mut self) -> Option<T> {
        self.due = None;
        self.pending.take()
    }
}

/// Counts playback errors and trips once `limit` land within `window`. Stays tripped.
pub struct PlaybackErrorGate {
    limit: usize,
    window: Duration,
    times: VecDeque<Instant>,
    tripped: bool,
}

impl Default for PlaybackErrorGate {
    fn default() -> Self {
        Self::new(3, Duration::from_secs(60))
    }
}

impl PlaybackErrorGate {
    pub fn new(limit: usize, window: Duration) -> Self {
        Self {
            limit,
            window,
            times: VecDeque::new(),
            tripped: false,
        }
    }

    pub fn record(&mut self, now: Instant) -> bool {
        if self.tripped {
            return true;
        }
        self.times.push_back(now);
        while self
            .times
            .front()
            .is_some_and(|t| now.duration_since(*t) > self.window)
        {
            self.times.pop_front();
        }
        if self.times.len() >= self.limit {
            self.tripped = true;
        }
        self.tripped
    }

    pub fn is_tripped(&self) -> bool {
        self.tripped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    #[test]
    fn gate_trips_on_the_third_error_within_a_minute() {
        let t0 = Instant::now();
        let mut gate = PlaybackErrorGate::default();
        assert!(!gate.record(at(t0, 0)));
        assert!(!gate.record(at(t0, 10_000)));
        assert!(gate.record(at(t0, 59_000)));
        assert!(gate.is_tripped());
    }

    #[test]
    fn gate_forgets_errors_older_than_a_minute() {
        let t0 = Instant::now();
        let mut gate = PlaybackErrorGate::default();
        gate.record(at(t0, 0));
        gate.record(at(t0, 10_000));
        assert!(!gate.record(at(t0, 61_000)));
        assert!(gate.record(at(t0, 62_000)));
    }

    #[test]
    fn gate_stays_tripped() {
        let t0 = Instant::now();
        let mut gate = PlaybackErrorGate::new(1, Duration::from_secs(1));
        assert!(gate.record(at(t0, 0)));
        assert!(gate.record(at(t0, 1_000_000)));
    }

    #[test]
    fn throttle_runs_the_first_call_and_collapses_the_rest_into_a_trailing_call() {
        let t0 = Instant::now();
        let mut t = Throttle::new(Duration::from_secs(5));
        assert_eq!(t.call(t0, 1), Some(1));
        assert_eq!(t.call(t0, 2), None);
        assert_eq!(t.call(t0, 3), None);
        assert_eq!(t.deadline(), Some(at(t0, 5000)));
        assert_eq!(t.poll(at(t0, 4999)), None);
        assert_eq!(t.poll(at(t0, 5000)), Some(3));
        assert_eq!(t.deadline(), None);
    }

    #[test]
    fn throttle_lets_a_call_through_once_the_window_has_passed() {
        let t0 = Instant::now();
        let mut t = Throttle::new(Duration::from_secs(5));
        assert_eq!(t.call(t0, 1), Some(1));
        assert_eq!(t.call(at(t0, 5000), 2), Some(2));
    }

    #[test]
    fn throttle_flush_runs_the_pending_call_and_cancel_drops_it_and_reopens_the_window() {
        let t0 = Instant::now();
        let mut t = Throttle::new(Duration::from_secs(5));
        t.call(t0, 1);
        t.call(t0, 2);
        assert_eq!(t.flush(t0), Some(2));
        assert_eq!(t.call(t0, 3), None);
        t.cancel();
        assert_eq!(t.poll(at(t0, 10_000)), None);
        assert_eq!(t.call(at(t0, 10_000), 4), Some(4));
    }

    #[test]
    fn debounce_calls_once_with_the_last_value_after_the_delay() {
        let t0 = Instant::now();
        let mut d = Debounce::new(Duration::from_millis(300));
        d.call(t0, 0.1);
        d.call(at(t0, 200), 0.2);
        assert_eq!(d.poll(at(t0, 499)), None);
        assert_eq!(d.poll(at(t0, 500)), Some(0.2));
        assert_eq!(d.poll(at(t0, 900)), None);
    }

    #[test]
    fn debounce_flush() {
        let t0 = Instant::now();
        let mut d = Debounce::new(Duration::from_millis(300));
        d.call(t0, 1);
        assert_eq!(d.flush(), Some(1));
        assert_eq!(d.flush(), None);
        assert_eq!(d.deadline(), None);
    }
}
