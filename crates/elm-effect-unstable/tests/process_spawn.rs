//! Process spawning: command forms, handles, exit codes, stdio.

#[path = "../src/process.rs"]
#[allow(dead_code)]
mod process;

use process::{command_array, command_template, split_template};

#[test]
fn template_form_creates_standard_command() {
    let cmd = command_template("echo hello world");
    assert_eq!(cmd.program(), "echo");
    assert_eq!(cmd.args(), &["hello".to_string(), "world".to_string()]);
}

#[test]
fn template_preserves_escapes_and_quotes() {
    assert_eq!(split_template("a \\u{1F600} b"), vec!["a", "u{1F600}", "b"]);
    assert_eq!(split_template("a \"b c\" d"), vec!["a", "b c", "d"]);
}

#[test]
fn array_and_options_forms_work() {
    let cmd = command_array("echo", &["hi"]).cwd("/tmp").env("K", "V").clear_env();
    assert_eq!(cmd.program(), "echo");
    assert_eq!(cmd.args(), &["hi".to_string()]);
}

#[test]
fn spawn_returns_a_handle_and_exit_code() {
    let mut child = command_array("sh", &["-c", "exit 3"]).spawn().unwrap();
    assert!(child.id().is_some());
    let out = child.wait().unwrap();
    assert_eq!(out.code, Some(3));
    assert!(!out.is_success());
}

#[test]
fn spawn_collects_stdout() {
    let mut child = command_array("echo", &["hello"]).spawn().unwrap();
    let out = child.wait().unwrap();
    assert_eq!(out.code, Some(0));
    assert!(out.is_success());
    assert_eq!(out.stdout_text().trim(), "hello");
}
