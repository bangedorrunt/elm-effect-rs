//! `FiberRef`: inheritance, isolation, and scoped restore.

use std::convert::Infallible;

use elm_effect::{Effect, Exit, FiberRef, Runtime};

#[test]
fn get_set_update_and_modify_share_one_fiber_local_value() {
    let cell = FiberRef::make(1i32);
    let update_cell = cell.clone();
    let modify_cell = cell.clone();

    let program: Effect<(i32, i32, i32, i32), Infallible, ()> = cell
        .get()
        .flat_map(move |default| {
            let cell = update_cell.clone();
            cell.update(|n| n + 1)
                .flat_map(move |_| cell.get().map(move |updated| (default, updated)))
        })
        .flat_map(move |(default, updated)| {
            let cell = modify_cell.clone();
            cell.modify(|n| (n * 10, n + 1)).flat_map(move |modified| {
                cell.get()
                    .map(move |after| (default, updated, modified, after))
            })
        });

    assert_eq!(Runtime::run(program), Exit::Success((1, 2, 20, 3)));
}

#[test]
fn locally_restores_after_success_and_typed_failure() {
    let cell = FiberRef::make(1i32);
    let success_cell = cell.clone();
    let success = cell
        .locally(7, success_cell.get())
        .flat_map(move |inside| success_cell.get().map(move |after| (inside, after)));

    assert_eq!(Runtime::run(success), Exit::Success((7, 1)));

    let failure_cell = FiberRef::make(1i32);
    let failure_reader = failure_cell.clone();
    let failure = failure_cell
        .locally(7, Effect::<i32, String>::fail("boom".to_string()).exit())
        .flat_map(move |exit| {
            assert!(matches!(exit, Exit::Failure(_)));
            failure_reader.get()
        });

    assert_eq!(Runtime::run(failure), Exit::Success(1));
}

#[test]
fn child_inherits_at_fork_and_siblings_keep_their_own_changes() {
    let cell = FiberRef::make(1i32);

    let first_cell = cell.clone();
    let first: Effect<(i32, i32), Infallible, ()> = first_cell.get().flat_map(move |inherited| {
        let cell = first_cell.clone();
        cell.set(10)
            .flat_map(move |_| cell.get().map(move |mine| (inherited, mine)))
    });

    let second_cell = cell.clone();
    let second: Effect<(i32, i32), Infallible, ()> = second_cell.get().flat_map(move |inherited| {
        let cell = second_cell.clone();
        cell.set(20)
            .flat_map(move |_| cell.get().map(move |mine| (inherited, mine)))
    });

    let program = cell.set(2).flat_map(move |_| {
        let first = first.fork_child_immediate();
        let second = second.fork_child_immediate();
        first.flat_map(move |first| {
            second.flat_map(move |second| {
                let first = Effect::sync(move || first.join());
                let second = Effect::sync(move || second.join());
                let parent = cell.clone();
                first.flat_map(move |first| {
                    second.flat_map(move |second| {
                        parent.get().map(move |parent| (first, second, parent))
                    })
                })
            })
        })
    });

    assert_eq!(
        Runtime::run(program),
        Exit::Success((Exit::Success((2, 10)), Exit::Success((2, 20)), 2))
    );
}

#[test]
fn detached_child_inherits_the_spawning_value() {
    let cell = FiberRef::make(0i32);
    let child_cell = cell.clone();
    let child = child_cell.get().fork_detach();

    let program = cell
        .set(5)
        .flat_map(move |_| child.flat_map(move |handle| Effect::sync(move || handle.join())));

    assert_eq!(Runtime::run(program), Exit::Success(Exit::Success(5)));
}
