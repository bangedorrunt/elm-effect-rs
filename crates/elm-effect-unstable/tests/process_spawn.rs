//! Process spawning: command forms, handles, exit codes, stdio.

#[path = "../src/process.rs"]
#[allow(dead_code)]
mod process;

use process::{Command, Pipeline, command_array, command_template, split_template};

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
    let cmd = command_array("echo", &["hi"])
        .cwd("/tmp")
        .env("K", "V")
        .clear_env();
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

#[test]
fn shell_form_executes_commands() {
    let mut child = command_array("sh", &["-c", "echo shell-hey"])
        .spawn()
        .unwrap();
    let out = child.wait().unwrap();
    assert!(out.is_success());
    assert_eq!(out.stdout_text().trim(), "shell-hey");
}

#[test]
fn env_vars_merge_and_override() {
    let mut child = command_array("sh", &["-c", "echo $SGH_PROBE"])
        .env("SGH_PROBE", "merged")
        .spawn()
        .unwrap();
    let out = child.wait().unwrap();
    assert_eq!(out.stdout_text().trim(), "merged");

    let mut child = command_template("sh -c 'echo $SGH_PROBE'")
        .env("SGH_PROBE", "templated")
        .spawn()
        .unwrap();
    let out = child.wait().unwrap();
    assert_eq!(out.stdout_text().trim(), "templated");
}

#[test]
fn pipelines_chain_stdio() {
    use process::Pipeline;
    let pipe = Pipeline::new(vec![
        command_array("echo", &["hello"]),
        command_array("tr", &["a-z", "A-Z"]),
    ]);
    assert!(pipe.is_piped());
    let out = pipe.run().unwrap();
    assert!(out.is_success());
    assert_eq!(out.stdout_text().trim(), "HELLO");
}

#[test]
fn single_stage_is_not_piped() {
    use process::Pipeline;
    let single = Pipeline::new(vec![command_array("echo", &["hi"])]);
    assert!(!single.is_piped());
    let out = single.run().unwrap();
    assert_eq!(out.stdout_text().trim(), "hi");
}

#[test]
fn fd_names_parse_and_render() {
    use process::{fd_name, parse_fd_name};
    assert_eq!(parse_fd_name("fd3"), Some(3));
    assert_eq!(parse_fd_name("fd10"), Some(10));
    assert_eq!(parse_fd_name("fd100"), Some(100));
    for bad in [
        "stdin", "stdout", "stderr", "fd0", "fd1", "fd2", "invalid", "fd", "",
    ] {
        assert_eq!(parse_fd_name(bad), None, "{bad}");
    }
    assert_eq!(fd_name(3), "fd3");
    assert_eq!(fd_name(10), "fd10");
}

#[test]
fn is_command_detects_both_forms() {
    let single = Command::Standard(command_template("echo hello"));
    let piped = Command::Piped(Pipeline::new(vec![
        command_array("cat", &["file"]),
        command_array("grep", &["pattern"]),
    ]));
    assert!(single.is_command());
    assert!(piped.is_command());
}

#[test]
fn is_standard_command_rejects_piped() {
    let array = Command::Standard(command_array("echo", &["hello"]));
    let templated = Command::Standard(command_template("echo hello"));
    let piped = Command::Piped(Pipeline::new(vec![
        command_array("cat", &["file"]),
        command_array("grep", &["pattern"]),
    ]));
    assert!(array.is_standard_command());
    assert!(templated.is_standard_command());
    assert!(!piped.is_standard_command());
}

#[test]
fn is_piped_command_rejects_single() {
    let single = Command::Standard(command_template("echo hello"));
    let piped = Command::Piped(Pipeline::new(vec![
        command_array("cat", &["file"]),
        command_array("grep", &["pattern"]),
    ]));
    assert!(!single.is_piped_command());
    assert!(piped.is_piped_command());
}
