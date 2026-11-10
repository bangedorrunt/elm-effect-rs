//! Example: print a topology-only TLA+ module for a chart.
//!
//! `scripts/verify-formal.sh` writes the output to a `.tla` file and runs TLC
//! over it, so the generated spec is proven well-formed rather than only
//! string-matched.
//!
//! Run it: `cargo run -p elm-effect-xstate --example emit_tla`

use elm_effect_xstate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Light {
    Green,
    Yellow,
}

fn main() {
    let machine: Machine<Light, (), (), ()> = Machine::builder(Light::Green, ())
        .state(Light::Green, |node| {
            let next = node.transition(Light::Yellow).on(|_| true).label("TIMER");
            node.on(next)
        })
        .state(Light::Yellow, |node| {
            let next = node.transition(Light::Green).on(|_| true);
            node.on(next)
        })
        .build();

    print!("{}", machine.to_tla_named("GeneratedStatechart"));
}
