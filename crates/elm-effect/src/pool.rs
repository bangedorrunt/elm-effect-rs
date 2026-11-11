//! A bounded pool of reusable values.
//!
//! [`Pool::acquire`] blocks while every slot is checked out. The returned
//! [`Pooled`] guard puts the value back when it drops. A value that failed
//! must be invalidated instead, so a broken resource never re-enters
//! circulation.

use std::sync::{Arc, Condvar, Mutex};

/// What happens to values beyond the idle bound when released.
#[derive(Clone, Debug)]
pub struct IdlePolicy {
    /// Idle values kept at most.
    pub max_idle: usize,
    /// Idle values older than this are dropped on acquire.
    pub max_idle_time: Option<std::time::Duration>,
}

impl Default for IdlePolicy {
    fn default() -> Self {
        Self {
            max_idle: 8,
            max_idle_time: None,
        }
    }
}

/// A bounded pool of reusable values. `acquire` blocks while exhausted;
/// dropping the guard returns the value, or releases it when idle bounds
/// say so.
pub struct Pool<T> {
    inner: Mutex<PoolInner<T>>,
    available: Condvar,
    make: Arc<dyn Fn() -> T + Send + Sync>,
    max: usize,
    idle: IdlePolicy,
}

struct PoolInner<T> {
    idle: Vec<(T, std::time::Instant)>,
    total: usize,
}

impl<T> Pool<T>
where
    T: Send + 'static,
{
    /// A pool making values with `make`, holding at most `max` total.
    pub fn new(max: usize, idle: IdlePolicy, make: impl Fn() -> T + Send + Sync + 'static) -> Self {
        Self {
            inner: Mutex::new(PoolInner {
                idle: Vec::new(),
                total: 0,
            }),
            available: Condvar::new(),
            make: Arc::new(make),
            max: max.max(1),
            idle,
        }
    }

    /// Take a value, blocking while exhausted. Idle values older than the
    /// idle timeout are released instead of reused.
    pub fn acquire(&self) -> Pooled<'_, T> {
        let mut guard = self.inner.lock().unwrap();
        loop {
            while let Some((value, at)) = guard.idle.pop() {
                if let Some(timeout) = self.idle.max_idle_time
                    && at.elapsed() >= timeout
                {
                    guard.total -= 1;
                    continue;
                }
                return Pooled {
                    value: Some(value),
                    pool: self,
                    returning: true,
                };
            }
            if guard.total < self.max {
                guard.total += 1;
                drop(guard);
                let value = (self.make)();
                return Pooled {
                    value: Some(value),
                    pool: self,
                    returning: true,
                };
            }
            guard = self.available.wait(guard).unwrap();
        }
    }

    /// Take a value the caller makes, handing the slot back when making
    /// fails.
    ///
    /// A failed attempt must not shrink the pool: the slot is counted while
    /// the attempt runs and given back on `Err`, which is what lets a retry
    /// loop keep trying without starving itself. An idle value is reused
    /// first; the pool's own `make` is not used on this path.
    pub fn acquire_with<E>(
        &self,
        create: impl FnOnce() -> Result<T, E>,
    ) -> Result<Pooled<'_, T>, E> {
        let mut guard = self.inner.lock().unwrap();
        loop {
            while let Some((value, at)) = guard.idle.pop() {
                if let Some(timeout) = self.idle.max_idle_time
                    && at.elapsed() >= timeout
                {
                    guard.total -= 1;
                    continue;
                }
                return Ok(Pooled {
                    value: Some(value),
                    pool: self,
                    returning: true,
                });
            }
            if guard.total < self.max {
                guard.total += 1;
                drop(guard);
                return match create() {
                    Ok(value) => Ok(Pooled {
                        value: Some(value),
                        pool: self,
                        returning: true,
                    }),
                    Err(error) => {
                        let mut guard = self.inner.lock().unwrap();
                        guard.total -= 1;
                        self.available.notify_one();
                        Err(error)
                    }
                };
            }
            guard = self.available.wait(guard).unwrap();
        }
    }

    /// Values currently checked out.
    pub fn in_use(&self) -> usize {
        let guard = self.inner.lock().unwrap();
        guard.total - guard.idle.len()
    }

    /// Values currently idle.
    pub fn idle_count(&self) -> usize {
        self.inner.lock().unwrap().idle.len()
    }

    fn release(&self, value: T) {
        let mut guard = self.inner.lock().unwrap();
        if guard.idle.len() < self.idle.max_idle {
            guard.idle.push((value, std::time::Instant::now()));
        } else {
            guard.total -= 1;
        }
        self.available.notify_one();
    }

    /// Drop a value without returning it to circulation: the total
    /// shrinks and one waiter (if any) wakes to make a replacement.
    fn discard(&self, value: T) {
        drop(value);
        let mut guard = self.inner.lock().unwrap();
        guard.total -= 1;
        self.available.notify_one();
    }

    /// Fill idle storage up to `count` values (bounded by `max` total).
    /// Mirrors pool preallocation: later acquires reuse without making.
    pub fn preallocate(&self, count: usize) {
        loop {
            let room = {
                let guard = self.inner.lock().unwrap();
                guard.idle.len() < count && guard.total < self.max
            };
            if !room {
                return;
            }
            let value = (self.make)();
            let mut guard = self.inner.lock().unwrap();
            if guard.idle.len() < count && guard.total < self.max {
                guard.total += 1;
                guard.idle.push((value, std::time::Instant::now()));
            } else {
                drop(guard);
                drop(value);
                return;
            }
            self.available.notify_one();
        }
    }

    /// Take a value out of shared circulation permanently: dropping the
    /// guard shrinks the pool instead of returning the value.
    pub fn reserve(&self) -> Pooled<'_, T> {
        let mut guard = self.acquire();
        guard.returning = false;
        guard
    }
}

/// A checked-out pool value. Dropping returns it (or releases it when the
/// idle bound says so).
pub struct Pooled<'a, T: Send + 'static> {
    value: Option<T>,
    pool: &'a Pool<T>,
    returning: bool,
}

impl<T: Send + 'static> std::ops::Deref for Pooled<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.value.as_ref().expect("held")
    }
}

impl<T: Send + 'static> std::ops::DerefMut for Pooled<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.value.as_mut().expect("held")
    }
}

impl<T: Send + 'static> Pooled<'_, T> {
    /// Reject a checked-out value: drops it without returning it to the
    /// pool (a broken item never circulates again).
    pub fn invalidate(mut self) {
        if let Some(value) = self.value.take() {
            self.pool.discard(value);
        }
    }
}

impl<T: Send + 'static> Drop for Pooled<'_, T> {
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            if self.returning {
                self.pool.release(value);
            } else {
                self.pool.discard(value);
            }
        }
    }
}
