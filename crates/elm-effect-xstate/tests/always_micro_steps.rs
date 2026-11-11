use elm_effect_xstate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum LoopState {
    Start,
    Counting,
    Done,
}

#[derive(Debug, Clone)]
enum LoopEvent {
    Begin,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LoopContext {
    count: u32,
    trace: Vec<String>,
}

#[test]
fn eventless_always_transitions_cascade_until_quiescence() {
    let machine: Machine<LoopState, LoopEvent, LoopContext, ()> =
        Machine::builder(LoopState::Start, LoopContext::default())
            .state(LoopState::Start, |node| {
                let to_counting = node
                    .transition(LoopState::Counting)
                    .on(|e| matches!(e, LoopEvent::Begin))
                    .assign(|ctx, _| ctx.trace.push("start->counting".into()));
                node.on(to_counting)
            })
            .state(LoopState::Counting, |node| {
                let self_loop = node
                    .transition(LoopState::Counting)
                    .reenter()
                    .guard(|ctx, _| ctx.count < 3)
                    .assign(|ctx, _| {
                        ctx.count += 1;
                        ctx.trace.push(format!("increment->{}", ctx.count));
                    });
                let to_done = node
                    .transition(LoopState::Done)
                    .guard(|ctx, _| ctx.count >= 3)
                    .assign(|ctx, _| ctx.trace.push("counting->done".into()));

                node.entry(|ctx, _| ctx.trace.push(format!("enter_count:{}", ctx.count)))
                    .always(self_loop)
                    .always(to_done)
            })
            .final_state(LoopState::Done, |node| {
                node.entry(|ctx, _| ctx.trace.push("enter_done".into()))
            })
            .build();

    let outcome = machine.step(
        &LoopState::Start,
        &LoopContext::default(),
        &LoopEvent::Begin,
    );

    assert_eq!(outcome.current_state, LoopState::Done);
    assert_eq!(outcome.context.count, 3);
    assert_eq!(
        outcome.context.trace,
        vec![
            "start->counting".to_string(),
            "enter_count:0".to_string(),
            "increment->1".to_string(),
            "enter_count:1".to_string(),
            "increment->2".to_string(),
            "enter_count:2".to_string(),
            "increment->3".to_string(),
            "enter_count:3".to_string(),
            "counting->done".to_string(),
            "enter_done".to_string(),
        ]
    );
}
