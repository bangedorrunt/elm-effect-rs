//! `cli/Completions.generate` dispatch witnesses for
//! `packages/effect/test/cli/completions/completions.test.ts` (Completions
//! section). Each `#[test]` names the upstream line it proves.

mod completions_fixtures;

use completions_fixtures::simple;
use elm_effect_unstable::cli_completions::{Shell, generate};

#[test]
fn dispatches_to_bash_generator() {
    // :790
    let script = generate("greet", Shell::Bash, &simple());
    assert!(script.contains("complete -F _greet greet"));
}

#[test]
fn dispatches_to_zsh_generator() {
    // :796
    let script = generate("greet", Shell::Zsh, &simple());
    assert!(script.contains("#compdef greet"));
}

#[test]
fn dispatches_to_fish_generator() {
    // :802
    let script = generate("greet", Shell::Fish, &simple());
    assert!(script.contains("complete -c greet"));
}
