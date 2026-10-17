use elm_effect::{Concurrency, Ctx, Exit, ForkOpts, Runtime};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

#[test]
fn shutdown_with_deadline_names_stragglers_instead_of_hanging() {
    let rt = Runtime::new();
    let started = Arc::new(AtomicBool::new(false));
    let probe = Arc::clone(&started);
    let h = rt.fork_detach_with(
        move |_: &Ctx| -> Exit<(), String> {
            probe.store(true, Ordering::SeqCst);
            std::thread::park(); // never observes interruption: unkillable by construction
            Exit::succeed(())
        },
        ForkOpts::immediate(),
    );
    while !started.load(Ordering::SeqCst) {
        std::thread::park_timeout(Duration::from_millis(1));
    }
    let stragglers = rt.shutdown_with_deadline(elm_effect::Duration::from_millis(200));
    assert_eq!(stragglers, vec![h.id()]);
    // NOTE: no join — the straggler never finishes, and joining would hang.
    // The parked thread dies with the test process; the runtime is detached.
}

#[test]
fn bounded_all_times_out_on_hung_body() {
    let rt = Runtime::new();
    let (block_tx, block_rx) = std::sync::mpsc::channel::<()>();
    let bodies: Vec<elm_effect::Body<i32, String>> = vec![
        Box::new(|_: &Ctx| Exit::Success(1)),
        Box::new(move |_: &Ctx| {
            let _ = block_rx.recv(); // ignores interruption: hung past every bulkhead
            Exit::Success(2)
        }),
    ];
    let exit = elm_effect::all_with_deadline(
        &rt,
        bodies,
        Concurrency::Bounded(2),
        elm_effect::Duration::from_millis(200),
    );
    match exit {
        Exit::Failure(cause) => {
            assert!(!cause.defects().is_empty(), "timeout must surface a defect");
            assert!(
                cause.defects()[0].message.contains("timed out"),
                "unexpected defect: {:?}",
                cause.defects()[0].message
            );
        }
        Exit::Success(_) => panic!("a hung body must fail the fan-out, not hang it"),
    }
    drop(block_tx); // release the hung body so the suite exits cleanly
}

#[test]
fn bounded_partition_times_out_on_hung_body() {
    let rt = Runtime::new();
    let (block_tx, block_rx) = std::sync::mpsc::channel::<()>();
    let bodies: Vec<elm_effect::Body<u32, String>> = vec![
        Box::new(|_: &Ctx| Exit::Success(1)),
        Box::new(move |_: &Ctx| {
            let _ = block_rx.recv(); // ignores interruption: hung past every bulkhead
            Exit::Success(2)
        }),
    ];
    let exit = elm_effect::partition_with_deadline(
        &rt,
        bodies,
        Concurrency::Bounded(2),
        elm_effect::Duration::from_millis(200),
    );
    match exit {
        Exit::Failure(cause) => {
            assert!(!cause.defects().is_empty(), "timeout must surface a defect");
            assert!(
                cause.defects()[0].message.contains("timed out"),
                "unexpected defect: {:?}",
                cause.defects()[0].message
            );
        }
        Exit::Success(_) => panic!("a hung body must fail the fan-out, not hang it"),
    }
    drop(block_tx); // release the hung body so the suite exits cleanly
}
