use elm_effect::{Ctx, Exit, ForkOpts, Mask, Runtime, StartMode};
use std::panic::{AssertUnwindSafe, catch_unwind};

#[test]
fn inherit_mask_starts_scoped_and_detached_children_masked() {
    // Regression: both entries tested `mask == Mask::On`, so `Inherit` from a
    // masked parent silently meant `Off`.
    let rt = Runtime::new();
    let scope = rt.create_scope();
    let h = rt.fork_detach_with(
        move |ctx: &Ctx| -> Exit<bool, String> {
            let opts = ForkOpts {
                start: StartMode::Immediate,
                mask: Mask::Inherit,
            };
            let scoped = ctx.masked(|| {
                let child = ctx.fork_scoped(
                    &scope,
                    |child| Exit::<bool, String>::succeed(child.is_masked()),
                    opts,
                );
                matches!(child.join(), Exit::Success(true))
            });
            let detached = ctx.masked(|| {
                let child = ctx.fork_detach(
                    |child| Exit::<bool, String>::succeed(child.is_masked()),
                    opts,
                );
                matches!(child.join(), Exit::Success(true))
            });
            Exit::succeed(scoped && detached)
        },
        ForkOpts::immediate(),
    );
    assert_eq!(
        h.join(),
        Exit::Success(true),
        "an inherited mask must start the child masked"
    );
}

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
