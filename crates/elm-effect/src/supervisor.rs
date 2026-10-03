//! The supervisor: the one legal `catch_unwind` in the workspace.
//!
//! A panic in a fiber body becomes [`Reason::Die`] at the fiber boundary. It
//! is never a typed error `E`, never crosses into a sibling fiber, and never
//! takes the process down. Nothing else in this workspace catches.
//!
//! (Boundary check, run by hand at each wave-1 close:
//! `grep -rn catch_unwind crates/ --include='*.rs'`.)

use std::backtrace::Backtrace;
use std::panic::AssertUnwindSafe;

use crate::{Cause, Defect, Exit};

/// Run `body` under the supervisor. Panics become `Die` with the panic
/// message and a captured backtrace.
pub fn run<A: Send + 'static, E: Send + 'static>(
    ctx: &crate::Ctx,
    body: impl FnOnce(&crate::Ctx) -> Exit<A, E>,
) -> Exit<A, E> {
    match std::panic::catch_unwind(AssertUnwindSafe(|| body(ctx))) {
        Ok(exit) => exit,
        Err(payload) => Exit::Failure(Cause::die(defect_from_payload(payload))),
    }
}

fn defect_from_payload(payload: Box<dyn std::any::Any + Send>) -> Defect {
    let message = payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "fiber panicked with a non-string payload".to_string());
    Defect {
        message,
        backtrace: Backtrace::capture().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Ctx, Runtime};

    #[test]
    fn panic_payload_renders_as_die() {
        // Direct supervisor check without spawning: static payloads.
        let d = defect_from_payload(Box::new("boom"));
        assert_eq!(d.message, "boom");

        let rt = Runtime::new();
        let h = rt.fork_detach(|_ctx: &Ctx| -> Exit<(), String> {
            panic!("through the supervisor");
        });
        let exit = h.join();
        let cause = match exit {
            Exit::Failure(c) => c,
            Exit::Success(()) => panic!("must not succeed"),
        };
        assert_eq!(cause.defects().len(), 1);
        assert!(
            cause.defects()[0]
                .message
                .contains("through the supervisor")
        );
    }
}
