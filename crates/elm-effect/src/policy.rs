//! `policy::hedge`: one duplicate for idempotent reads.
//!
//! Start the call; start one duplicate after a caller-supplied delay if the
//! first is still running; take the first success; cancel (interrupt + join)
//! the loser. The duplicate runs only when the first is slow: a fast first
//! returns before the delay elapses and the duplicate closure never runs.
//!
//! Idempotent reads only. Reads have no observable side effects, so running
//! the call twice is safe; never put this in the default stack, where an
//! arbitrary effect could run twice.
//!
//! Failure handling: a failure only wins when the other racer also fails
//! (the first cause is returned then). A lone failure keeps waiting for the
//! other racer, which may still succeed.

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::mpsc;

use crate::{ClockImpl, Ctx, Duration, Exit, ForkOpts, Runtime};

/// Which racer answered first.
enum Done<A, E> {
    First(Exit<A, E>),
    Dup(Exit<A, E>),
}

/// Race `first` against one delayed duplicate.
///
/// Blocks like [`crate::timeout`]: with a virtual clock, drive time from
/// another thread (wait for the delay sleep to park, then `adjust` past it).
pub fn hedge<A, E>(
    rt: &Runtime,
    clock: Arc<dyn ClockImpl>,
    delay: Duration,
    first: impl FnOnce(&Ctx) -> Exit<A, E> + Send + 'static,
    duplicate: impl FnOnce(&Ctx) -> Exit<A, E> + Send + 'static,
) -> Exit<A, E>
where
    A: Send + 'static,
    E: Send + 'static,
{
    let (tx, rx) = mpsc::channel::<Done<A, E>>();
    let immediate = ForkOpts::immediate();

    let tx_first = tx.clone();
    let first_h = rt.fork_detach_with(
        move |ctx| {
            let exit = first(ctx);
            let _ = tx_first.send(Done::First(exit));
            Exit::<(), Infallible>::succeed(())
        },
        immediate,
    );

    // Give the first call the whole delay to finish alone. A fast first is
    // already in the channel when this returns, so the duplicate never runs.
    // (No interrupt source on the calling thread; virtual time is driven by
    // whoever holds the clock.)
    clock.sleep(delay, &|| false);
    if let Ok(Done::First(exit)) = rx.try_recv() {
        first_h.join();
        return exit;
    }

    let tx_dup = tx;
    let dup_h = rt.fork_detach_with(
        move |ctx| {
            let exit = duplicate(ctx);
            let _ = tx_dup.send(Done::Dup(exit));
            Exit::<(), Infallible>::succeed(())
        },
        immediate,
    );

    let (mut first_done, mut dup_done) = (false, false);
    let mut first_failure: Option<Exit<A, E>> = None;
    loop {
        match rx.recv().expect("a racer always answers") {
            Done::First(exit) => {
                first_done = true;
                match exit {
                    Exit::Success(_) => {
                        dup_h.interrupt();
                        dup_h.join();
                        first_h.join();
                        return exit;
                    }
                    failed => {
                        if dup_done {
                            // Both failed: the first cause wins.
                            first_h.join();
                            dup_h.join();
                            return failed;
                        }
                        first_failure = Some(failed);
                    }
                }
            }
            Done::Dup(exit) => {
                dup_done = true;
                match exit {
                    Exit::Success(_) => {
                        first_h.interrupt();
                        first_h.join();
                        dup_h.join();
                        return exit;
                    }
                    Exit::Failure(_) => {
                        if first_done {
                            // The first racer failed earlier (a success would
                            // have returned already): prefer the first cause.
                            first_h.join();
                            dup_h.join();
                            return first_failure
                                .take()
                                .expect("first failed before the duplicate");
                        }
                        // Otherwise the first is still running and may succeed.
                    }
                }
            }
        }
    }
}
