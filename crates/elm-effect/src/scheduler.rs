//! `Scheduler` port (`Scheduler.ts` at the pin): cooperative task queue.
//!
//! Tasks run FIFO until idle. After every [`max_ops`](Scheduler::max_ops)
//! operations the scheduler records one yield: the tag's
//! `MaxOpsBeforeYield` knob, which bounds how long a burst hogs the thread
//! before cooperating. Our fibers already yield through the runtime; this
//! is the explicit, testable value-level spelling.
//!
//! Residuals: plugging this into the fiber driver, `PreventSchedulerYield`
//! context, and mixed clock sleep integration.

use std::collections::VecDeque;

/// Cooperative FIFO task scheduler.
#[derive(Default)]
pub struct Scheduler {
    max_ops: usize,
    queue: VecDeque<Box<dyn FnOnce() + Send>>,
    ops_since_yield: usize,
    yields: usize,
}

impl Scheduler {
    /// Empty scheduler yielding every `max_ops` operations (`0` disables
    /// yield accounting but still drains the queue).
    pub fn new(max_ops: usize) -> Self {
        Self {
            max_ops,
            queue: VecDeque::new(),
            ops_since_yield: 0,
            yields: 0,
        }
    }

    /// Queue one task.
    pub fn schedule(&mut self, task: impl FnOnce() + Send + 'static) {
        self.queue.push_back(Box::new(task));
    }

    /// Queued task count.
    pub fn pending(&self) -> usize {
        self.queue.len()
    }

    /// Operations per yield.
    pub fn max_ops(&self) -> usize {
        self.max_ops
    }

    /// Yields recorded so far.
    pub fn yields(&self) -> usize {
        self.yields
    }

    /// Drain the queue FIFO, returning operations run. Counts one yield per
    /// `max_ops` operations (any remainder counts once it ran).
    pub fn run_until_idle(&mut self) -> usize {
        let mut ops = 0;
        while let Some(task) = self.queue.pop_front() {
            task();
            ops += 1;
            if self.max_ops > 0 {
                self.ops_since_yield += 1;
                if self.ops_since_yield >= self.max_ops {
                    self.ops_since_yield = 0;
                    self.yields += 1;
                }
            }
        }
        ops
    }
}
