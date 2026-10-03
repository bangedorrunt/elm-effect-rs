//! Law L8: time enters through the Clock service, never the wall.
//!
//! [`Duration`] is millisecond time. [`ClockImpl`] is the service behind the
//! [`Clock`](crate::Clock) tag: [`SystemClock`] parks threads on real
//! deadlines, [`TestClock`] parks fibers on virtual time advanced only by
//! [`TestClock::adjust`]. A sleep observes interruption through a poll
//! closure, so bodies pass `&|| ctx.is_interrupted()`.
//!
//! The 1ms poll quantum inside sleeps bounds wall latency only: completions
//! happen exclusively via `adjust` (virtual) or deadlines (real), so test
//! outcomes are virtual-time-deterministic.

use std::sync::{Arc, Condvar, Mutex};

/// Millisecond time. Durations over `u64::MAX` millis saturate at construction
/// only if built from parts that overflow in debug (plain multiplication).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Duration {
    millis: u64,
}

impl Duration {
    pub const ZERO: Self = Self { millis: 0 };

    pub const fn from_millis(millis: u64) -> Self {
        Self { millis }
    }

    pub const fn from_secs(secs: u64) -> Self {
        Self {
            millis: secs * 1000,
        }
    }

    pub const fn from_minutes(minutes: u64) -> Self {
        Self {
            millis: minutes * 60 * 1000,
        }
    }

    pub fn as_millis(self) -> u64 {
        self.millis
    }

    pub fn is_zero(self) -> bool {
        self.millis == 0
    }
}

/// How a sleep ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SleepOutcome {
    /// The deadline passed.
    Elapsed,
    /// Interruption was requested first.
    Interrupted,
}

/// The clock service implementation. Object-safe so tests share one clock
/// behind `Arc` while bodies hold the trait object.
pub trait ClockImpl: Send + Sync + 'static {
    /// Park until `duration` elapses or `interrupted()` turns true, whichever
    /// comes first. Never advances virtual time itself.
    fn sleep(&self, duration: Duration, interrupted: &dyn Fn() -> bool) -> SleepOutcome;
    /// Current time in millis (virtual for tests, wall for system).
    fn now_millis(&self) -> u64;
}

/// A service tag for the clock: provide any [`ClockImpl`].
pub struct Clock;

impl crate::ServiceTag for Clock {
    type Service = Arc<dyn ClockImpl>;
    const ID: &'static str = "elm-effect/clock";
}

/// Wall-clock implementation: real deadlines, interruptible.
#[derive(Clone, Copy, Debug)]
pub struct SystemClock;

impl ClockImpl for SystemClock {
    fn sleep(&self, duration: Duration, interrupted: &dyn Fn() -> bool) -> SleepOutcome {
        if duration.is_zero() {
            return SleepOutcome::Elapsed;
        }
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(duration.as_millis());
        loop {
            if interrupted() {
                return SleepOutcome::Interrupted;
            }
            if std::time::Instant::now() >= deadline {
                return SleepOutcome::Elapsed;
            }
            std::thread::park_timeout(std::time::Duration::from_millis(1));
        }
    }

    fn now_millis(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

struct Sleeper {
    deadline_ms: u64,
    woken: Arc<(Mutex<bool>, Condvar)>,
}

/// Virtual clock: time advances only through [`TestClock::adjust`]. Sleepers
/// park on per-sleeper latches woken by `adjust`; interruption is observed
/// through a 1ms poll quantum (latency only, never completion).
pub struct TestClock {
    now_ms: Mutex<u64>,
    sleepers: Mutex<Vec<Sleeper>>,
}

impl TestClock {
    pub fn new() -> Self {
        Self {
            now_ms: Mutex::new(0),
            sleepers: Mutex::new(Vec::new()),
        }
    }

    /// Advance virtual time, waking every sleeper whose deadline passed.
    /// Never wakes the future: sleepers scheduled after this call see the new
    /// time (the adjust-first test pins this).
    pub fn adjust(&self, duration: Duration) {
        let now = {
            let mut guard = self.now_ms.lock().unwrap();
            *guard = guard.saturating_add(duration.as_millis());
            *guard
        };
        let sleepers = self.sleepers.lock().unwrap();
        for sleeper in sleepers.iter() {
            if sleeper.deadline_ms <= now {
                let (lock, cvar) = &*sleeper.woken;
                *lock.lock().unwrap() = true;
                cvar.notify_one();
            }
        }
    }

    /// Sleepers currently parked. Test aid: asserts scheduling happened (or
    /// not) before driving time.
    pub fn pending_sleepers(&self) -> usize {
        self.sleepers.lock().unwrap().len()
    }

    fn remove_sleeper(&self, woken: &Arc<(Mutex<bool>, Condvar)>) {
        self.sleepers
            .lock()
            .unwrap()
            .retain(|s| !Arc::ptr_eq(&s.woken, woken));
    }
}

impl Default for TestClock {
    fn default() -> Self {
        Self::new()
    }
}

impl ClockImpl for TestClock {
    fn sleep(&self, duration: Duration, interrupted: &dyn Fn() -> bool) -> SleepOutcome {
        let now = self.now_millis();
        let deadline = now.saturating_add(duration.as_millis());
        if deadline <= now {
            return SleepOutcome::Elapsed;
        }
        let woken: Arc<(Mutex<bool>, Condvar)> = Arc::new((Mutex::new(false), Condvar::new()));
        self.sleepers.lock().unwrap().push(Sleeper {
            deadline_ms: deadline,
            woken: Arc::clone(&woken),
        });
        loop {
            if interrupted() {
                self.remove_sleeper(&woken);
                return SleepOutcome::Interrupted;
            }
            if self.now_millis() >= deadline {
                self.remove_sleeper(&woken);
                return SleepOutcome::Elapsed;
            }
            let (lock, cvar) = &*woken;
            let guard = lock.lock().unwrap();
            if *guard {
                continue;
            }
            let _ = cvar
                .wait_timeout(guard, std::time::Duration::from_millis(1))
                .unwrap();
        }
    }

    fn now_millis(&self) -> u64 {
        *self.now_ms.lock().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn duration_orders_and_constructs() {
        assert!(Duration::from_minutes(1) > Duration::from_secs(59));
        assert_eq!(Duration::from_secs(2).as_millis(), 2000);
        assert!(Duration::ZERO.is_zero());
        assert!(!Duration::from_millis(1).is_zero());
    }

    #[test]
    fn zero_sleep_completes_without_parking() {
        let clock = TestClock::new();
        let out = clock.sleep(Duration::ZERO, &|| false);
        assert_eq!(out, SleepOutcome::Elapsed);
        assert_eq!(clock.pending_sleepers(), 0);
    }

    #[test]
    fn adjust_wakes_parked_sleeper() {
        let clock = Arc::new(TestClock::new());
        let worker = clock.clone();
        let done = Arc::new(AtomicBool::new(false));
        let probe = done.clone();
        let t = std::thread::spawn(move || {
            let out = worker.sleep(Duration::from_minutes(1), &|| false);
            probe.store(true, Ordering::SeqCst);
            out
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while clock.pending_sleepers() < 1 {
            assert!(std::time::Instant::now() < deadline);
            std::thread::park_timeout(std::time::Duration::from_millis(1));
        }
        clock.adjust(Duration::from_minutes(1));
        assert_eq!(t.join().expect("sleeper"), SleepOutcome::Elapsed);
        assert!(done.load(Ordering::SeqCst));
        assert_eq!(clock.pending_sleepers(), 0);
    }
}
