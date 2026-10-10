//! Example 1: a library function that returns an `Effect`.
//!
//! The point is the return type. A caller gets a *description* they can
//! run, retry, compose, or hand to a test without work happening. Nothing
//! has executed when `greet` returns.

use elm_effect::Effect;

/// Build the greeting effect. Runs nothing: the caller decides when, or
/// whether, to interpret it.
pub fn greet(name: &str) -> Effect<String, String> {
    let name = name.to_string();
    Effect::sync(move || format!("hello, {name}"))
}

/// A caller that runs it at the application edge. `examples/` is one of the
/// places a description may be interpreted, so `ManagedRuntime` lives here.
pub fn main() {
    let rt = elm_effect::ManagedRuntime::new();
    let exit = rt.run(greet("elm"));
    match exit {
        elm_effect::Exit::Success(message) => println!("{message}"),
        elm_effect::Exit::Failure(cause) => eprintln!("failed: {cause}"),
    }
}
