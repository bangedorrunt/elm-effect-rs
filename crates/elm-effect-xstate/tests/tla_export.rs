use elm_effect_xstate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Light {
    Green,
    Yellow,
}

#[derive(Debug, Clone)]
enum Tick {
    Timer,
}

#[test]
fn to_tla_emits_a_topology_module() {
    let machine: Machine<Light, Tick, (), ()> = Machine::builder(Light::Green, ())
        .state(Light::Green, |node| {
            let next = node
                .transition(Light::Yellow)
                .on(|event| matches!(event, Tick::Timer))
                .label("TIMER");
            node.on(next)
        })
        .state(Light::Yellow, |node| {
            let next = node
                .transition(Light::Green)
                .on(|event| matches!(event, Tick::Timer));
            node.on(next)
        })
        .build();

    let tla = machine.to_tla_named("Traffic");

    assert!(tla.starts_with("---- MODULE Traffic ----\n"));
    assert!(tla.contains("EXTENDS Naturals"));
    assert!(tla.contains("VARIABLES state"));
    assert!(tla.contains("States == {\"Green\", \"Yellow\"}"));
    assert!(tla.contains("Init == state = \"Green\""));
    assert!(tla.contains("state' = \"Yellow\""));
    assert!(tla.contains("state' = \"Green\""));
    assert!(tla.contains("Next ==\n    \\/ state = \"Green\""));
    assert!(tla.ends_with("====\n"));
    assert_eq!(machine.to_tla(), machine.to_tla_named("Statechart"));

    let outcome = machine.step(&Light::Green, &(), &Tick::Timer);
    assert_eq!(outcome.state(), Some(&Light::Yellow));
}
