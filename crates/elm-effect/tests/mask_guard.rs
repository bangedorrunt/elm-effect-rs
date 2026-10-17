use elm_effect::{Ctx, Exit, ForkOpts, Runtime};
use std::panic::{AssertUnwindSafe, catch_unwind};

#[test]
fn masked_restores_depth_on_panic_and_stays_interruptible() {
    let rt = Runtime::new();
    let h = rt.fork_detach_with(
        move |ctx: &Ctx| -> Exit<(), String> {
            let r = catch_unwind(AssertUnwindSafe(|| {
                ctx.masked(|| {
                    panic!("boom");
                })
            }));
            assert!(r.is_err(), "masked body must propagate the panic");
            assert!(
                !ctx.is_masked(),
                "mask depth must return to 0 after a panic in masked"
            );
            assert!(
                !ctx.is_interrupted(),
                "fiber stays interruptible after the guard restores depth"
            );
            Exit::succeed(())
        },
        ForkOpts::immediate(),
    );
    assert_eq!(h.join(), Exit::Success(()));
}
