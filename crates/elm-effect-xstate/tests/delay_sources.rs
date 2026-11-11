use elm_effect::Duration;
use elm_effect_xstate::prelude::*;

#[test]
fn a_named_delay_expression_resolves_from_schema() {
    let machine = DynamicMachine::from_json_str(
        r#"{
            "initial":"Waiting",
            "delays": {"short": 25},
            "states": {
                "Waiting": {"after": {"short": "Done"}},
                "Done": {}
            }
        }"#,
    )
    .expect("named delay schema");

    let entered = machine.enter(machine.initial_context());
    assert_eq!(
        entered.timers,
        [TimerIntent::Start {
            state: "Waiting".to_string(),
            duration: Duration::from_millis(25)
        }]
    );
}

#[test]
fn a_delay_function_recomputes_the_duration_from_context() {
    let machine = DynamicMachine::builder(
        serde_json::from_str(
            r#"{
                "initial":"Waiting",
                "context": {"delay_ms": 40},
                "states": {
                    "Waiting": {"after": {"timeout": "Done"}},
                    "Done": {}
                }
            }"#,
        )
        .expect("config"),
    )
    .delay("timeout", |context, _event| {
        Some(Duration::from_millis(
            context.data.get("delay_ms")?.as_u64()?,
        ))
    })
    .build();

    let entered = machine.enter(machine.initial_context());
    assert_eq!(
        entered.timers,
        [TimerIntent::Start {
            state: "Waiting".to_string(),
            duration: Duration::from_millis(40)
        }]
    );

    let state = MachineState {
        configuration: entered.configuration,
        history: entered.history,
        context: entered.context,
        armed: entered.armed,
    };
    let fired = machine.step_timer(
        &state,
        &TimerFired::new("Waiting".to_string(), Duration::from_millis(40)),
        &DynamicEvent::new("xstate.after.40.Waiting"),
    );
    assert_eq!(fired.current_state, "Done");
}

#[test]
fn a_delay_function_sees_the_entry_event() {
    let machine = DynamicMachine::builder(
        serde_json::from_str(
            r#"{
                "initial":"Waiting",
                "states": {
                    "Waiting": {"after": {"timeout": "Done"}},
                    "Done": {}
                }
            }"#,
        )
        .expect("config"),
    )
    .delay("timeout", |_context, event| {
        event
            .payload
            .get("wait_ms")
            .and_then(serde_json::Value::as_u64)
            .map(Duration::from_millis)
    })
    .build();

    let started = machine.start(
        machine.initial_context(),
        &DynamicEvent::with_payload("GO", serde_json::json!({ "wait_ms": 75 })),
    );
    assert_eq!(
        started.timers,
        [TimerIntent::Start {
            state: "Waiting".to_string(),
            duration: Duration::from_millis(75)
        }]
    );
}

#[test]
fn a_fired_timer_uses_the_duration_armed_at_entry() {
    let machine = DynamicMachine::builder(
        serde_json::from_str(
            r#"{
                "initial":"Waiting",
                "context": {"delay_ms": 40},
                "states": {
                    "Waiting": {"after": {"timeout": "Done"}},
                    "Done": {}
                }
            }"#,
        )
        .expect("config"),
    )
    .delay("timeout", |context, _event| {
        Some(Duration::from_millis(
            context.data.get("delay_ms")?.as_u64()?,
        ))
    })
    .build();

    let entered = machine.enter(machine.initial_context());
    let mut state = MachineState {
        configuration: entered.configuration,
        history: entered.history,
        context: entered.context,
        armed: entered.armed,
    };
    state.context.data["delay_ms"] = serde_json::json!(900);

    let fired = machine.step_timer(
        &state,
        &TimerFired::new("Waiting".to_string(), Duration::from_millis(40)),
        &DynamicEvent::new("xstate.after.40.Waiting"),
    );
    assert_eq!(fired.current_state, "Done");
}
