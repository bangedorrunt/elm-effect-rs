//! Example 2: a CLI that uses `Runtime::exit_code`.
//!
//! The exit code comes from the outcome, so success, typed failure, defect,
//! and interrupt each leave their own status. Interpreting the description
//! is the `main` edge; turning the outcome into a process status is
//! `Runtime::exit_code`'s job, not the caller's guesswork.

use elm_effect::{Effect, Exit, ManagedRuntime};

/// The CLI's work, as a description. The task name arrives as a message:
/// a missing one is a typed failure, not a panic.
fn work(task: Option<String>) -> Effect<String, String> {
    match task {
        Some(task) => Effect::sync(move || format!("ran {task}")),
        None => Effect::fail("no task given".to_string()),
    }
}

fn main() {
    let task = std::env::args().nth(1).filter(|a| !a.is_empty());
    let exit = ManagedRuntime::new().run(work(task));
    match &exit {
        Exit::Success(message) => println!("{message}"),
        Exit::Failure(cause) => eprintln!("failure: {cause}"),
    }
    std::process::exit(elm_effect::Runtime::exit_code(&exit));
}
