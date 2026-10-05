//! OS threads behind a tiny join-handle surface.
//!
//! Thin over [`std::thread`]: spawn named threads, join them back,
//! sleep the current one. No pool, no executor.

/// A handle to a spawned thread: join it exactly once.
pub struct Worker<T> {
    inner: Option<std::thread::JoinHandle<T>>,
}

impl<T> Worker<T> {
    /// The thread's return value, blocking until done.
    /// Panics when the thread panicked, like [`std::thread`] does.
    pub fn join(mut self) -> T {
        self.inner.take().unwrap().join().unwrap()
    }
}

/// Spawn a thread running `work`, named `name` when given.
pub fn spawn<T>(name: Option<&str>, work: impl FnOnce() -> T + Send + 'static) -> Worker<T>
where
    T: Send + 'static,
{
    let builder = match name {
        Some(name) => std::thread::Builder::new().name(name.to_string()),
        None => std::thread::Builder::new(),
    };
    Worker {
        inner: Some(builder.spawn(work).unwrap()),
    }
}

/// Sleep the current thread for `millis` milliseconds.
pub fn sleep_millis(millis: u64) {
    std::thread::sleep(std::time::Duration::from_millis(millis));
}

/// The current thread's id, for debugging and logs.
pub fn current_id() -> String {
    format!("{:?}", std::thread::current().id())
}
