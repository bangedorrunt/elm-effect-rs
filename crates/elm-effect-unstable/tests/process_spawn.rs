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

// Row witnesses for packages/effect/test/process/ChildProcessSpawnerTest.ts.
// Each test names the upstream line it proves.

/// Spawn a spec and take the failure; the error has no `Debug` on the
/// success type, so `unwrap_err` is unavailable.
fn spawn_err(spec: &process::CommandSpec) -> process::ProcessError {
    match spec.spawn() {
        Ok(_) => panic!("expected spawn to fail"),
        Err(error) => error,
    }
}

fn collect(spec: &process::CommandSpec) -> process::CommandOutput {
    spec.spawn().unwrap().wait().unwrap()
}

#[test]
fn spawn_simple_command_and_collect_output() {
    // :45
    let out = collect(&command_array("sh", &["-c", "printf portable"]));
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout_text().trim(), "portable");
}

#[test]
fn spawn_echo_command() {
    // :55
    let out = collect(&command_array("echo", &["hello", "world"]));
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout_text().trim(), "hello world");
}

#[test]
fn spawn_with_template_literal() {
    // :65
    let out = collect(&command_template("echo spawned"));
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout_text().trim(), "spawned");
}

#[test]
fn spawn_with_working_directory() {
    // :77 — /tmp is a symlink to /private/tmp on macOS, so match loosely.
    let out = collect(&command_array("pwd", &[]).cwd("/tmp"));
    assert_eq!(out.code, Some(0));
    assert!(out.stdout_text().contains("tmp"));
}

#[test]
fn spawn_cwd_with_template_literal_form() {
    // :88
    let out = collect(&command_template("pwd").cwd("/tmp"));
    assert_eq!(out.code, Some(0));
    assert!(out.stdout_text().contains("tmp"));
}

#[test]
fn spawn_environment_variables() {
    // :100
    let out = collect(
        &command_array("sh", &["-c", "echo $TEST_VAR"]).env("TEST_VAR", "test_value"),
    );
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout_text().trim(), "test_value");
}

#[test]
fn spawn_multiple_environment_variables() {
    // :113
    let out = collect(
        &command_array("sh", &["-c", "echo $VAR1-$VAR2-$VAR3"])
            .env("VAR1", "one")
            .env("VAR2", "two")
            .env("VAR3", "three"),
    );
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout_text().trim(), "one-two-three");
}

#[test]
fn spawn_merge_environment_variables_with_set_env() {
    // :126 — setEnv merges over the command's own env; later wins.
    let out = collect(
        &command_array("sh", &["-c", "echo $VAR1-$VAR2-$VAR3"])
            .env("VAR1", "one")
            .env("VAR2", "two")
            .env("VAR2", "override")
            .env("VAR3", "three"),
    );
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout_text().trim(), "one-override-three");
}

#[test]
fn spawn_execute_with_shell_when_using_sh_c() {
    // :142
    let out = collect(
        &command_array("sh", &["-c", "echo $SHELL_EXPANSION_VAR"])
            .env("SHELL_EXPANSION_VAR", "expanded"),
    );
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout_text().trim(), "expanded");
}

#[test]
fn spawn_not_expand_variables_without_shell() {
    // :156 — no shell, so the argument reaches `echo` literally.
    let out = collect(&command_template("echo $HOME"));
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout_text().trim(), "$HOME");
}

#[test]
fn spawn_allow_piping_with_shell() {
    // :167
    let out = collect(&command_array("sh", &["-c", "echo hello | tr a-z A-Z"]));
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout_text().trim(), "HELLO");
}

#[test]
fn spawn_template_literal_form() {
    // :179
    let out = collect(&command_template("echo hello"));
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout_text().trim(), "hello");
}

#[test]
fn spawn_string_interpolation() {
    // :189
    let name = "world";
    let out = collect(&command_template(&format!("echo hello {name}")));
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout_text().trim(), "hello world");
}

#[test]
fn spawn_number_interpolation() {
    // :200
    let count = 42;
    let out = collect(&command_template(&format!("echo count is {count}")));
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout_text().trim(), "count is 42");
}

#[test]
fn spawn_array_interpolation() {
    // :211 — an interpolated array flattens to one word per element.
    let dir = std::env::temp_dir().join(format!("ee-array-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("array-interpolation.txt"), b"test").unwrap();
    let args = ["-l", "-a"];
    let spec = command_template(&format!("ls {} {}", args.join(" "), dir.display()));
    assert_eq!(spec.args().first().map(String::as_str), Some("-l"));
    let out = collect(&spec);
    assert_eq!(out.code, Some(0));
    assert!(out.stdout_text().contains("array-interpolation.txt"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn spawn_multiple_interpolations() {
    // :228
    let (greeting, target) = ("hello", "world");
    let out = collect(&command_template(&format!("echo {greeting} {target}")));
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout_text().trim(), "hello world");
}

#[test]
fn spawn_options_with_template_literal() {
    // :240
    let filename = "test.txt";
    let out = collect(&command_template(&format!("echo {filename}")).cwd("/tmp"));
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout_text().trim(), "test.txt");
}

#[test]
fn spawn_capture_stderr_output() {
    // :253
    let out = collect(&command_array("sh", &["-c", "echo error message >&2"]));
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stderr_text().trim(), "error message");
}

#[test]
fn spawn_capture_both_stdout_and_stderr() {
    // :263
    let out = collect(&command_array("sh", &["-c", "echo stdout; echo stderr >&2"]));
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout_text().trim(), "stdout");
    assert_eq!(out.stderr_text().trim(), "stderr");
}

#[test]
fn spawn_more_stdout_than_stderr() {
    // :277
    let out = collect(&command_array(
        "sh",
        &[
            "-c",
            "echo line1; echo line2; echo line3; echo line4; echo line5; echo error >&2",
        ],
    ));
    assert_eq!(out.code, Some(0));
    assert_eq!(
        out.stdout_text().trim(),
        ["line1", "line2", "line3", "line4", "line5"].join("\n")
    );
    assert_eq!(out.stderr_text().trim(), "error");
}

#[test]
fn spawn_more_stderr_than_stdout() {
    // :295
    let out = collect(&command_array(
        "sh",
        &[
            "-c",
            "echo output; echo err1 >&2; echo err2 >&2; echo err3 >&2; echo err4 >&2; echo err5 >&2",
        ],
    ));
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout_text().trim(), "output");
    assert_eq!(
        out.stderr_text().trim(),
        ["err1", "err2", "err3", "err4", "err5"].join("\n")
    );
}

#[test]
fn spawn_read_stdout_when_stderr_empty() {
    // :313
    let out = collect(&command_array("echo", &["only stdout"]));
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout_text().trim(), "only stdout");
    assert_eq!(out.stderr_text().trim(), "");
}

#[test]
fn spawn_read_stderr_when_stdout_empty() {
    // :328
    let out = collect(&command_array("sh", &["-c", "echo only stderr >&2"]));
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout_text().trim(), "");
    assert_eq!(out.stderr_text().trim(), "only stderr");
}

#[test]
fn spawn_stream_stdout() {
    // :429
    let out = collect(&command_array("echo", &["streaming output"]));
    assert_eq!(out.stdout_text().trim(), "streaming output");
}

#[test]
fn spawn_stream_multiple_lines() {
    // :437
    let out = collect(&command_array("sh", &["-c", "echo line1; echo line2; echo line3"]));
    assert_eq!(
        out.stdout_text().trim(),
        ["line1", "line2", "line3"].join("\n")
    );
}

#[test]
fn spawn_kill_a_process() {
    // :447 — killed by signal, so the status carries no exit code.
    let mut child = command_array("sleep", &["10"]).spawn().unwrap();
    child.kill().unwrap();
    let out = child.wait().unwrap();
    assert!(!out.is_success());
    assert_eq!(out.code, None);
}

#[test]
fn spawn_simple_pipeline() {
    // :545
    let pipe = Pipeline::new(vec![
        command_template("echo hello world"),
        command_template("tr a-z A-Z"),
    ]);
    assert!(pipe.is_piped());
    let out = pipe.run().unwrap();
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout_text().trim(), "HELLO WORLD");
}

#[test]
fn spawn_three_stage_pipeline() {
    // :557
    let pipe = Pipeline::new(vec![
        command_template("echo hello world"),
        command_template("tr a-z A-Z"),
        command_array("tr", &[" ", "-"]),
    ]);
    let out = pipe.run().unwrap();
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout_text().trim(), "HELLO-WORLD");
}

#[test]
fn spawn_pipe_grep_output() {
    // :570 — printf, not echo: /bin/echo leaves \n literal on macOS.
    let pipe = Pipeline::new(vec![
        command_array("printf", &["line1\nline2\nline3\n"]),
        command_template("grep line2"),
    ]);
    let out = pipe.run().unwrap();
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout_text().trim(), "line2");
}

#[test]
fn spawn_mixed_command_forms_in_pipeline() {
    // :582
    let pipe = Pipeline::new(vec![
        command_array("echo", &["hello"]),
        command_template("tr a-z A-Z"),
    ]);
    let out = pipe.run().unwrap();
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout_text().trim(), "HELLO");
}

#[test]
fn pipeline_default_pipes_stdout_only() {
    // :628 — stderr is not piped, so only the stdout half reaches `cat`.
    let pipe = Pipeline::new(vec![
        command_array("sh", &["-c", "echo stdout; echo stderr >&2"]),
        command_template("cat"),
    ]);
    let out = pipe.run().unwrap();
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout_text().trim(), "stdout");
}

#[test]
fn pipeline_stdout_to_stdin() {
    // :642, :654, :666 — the Rust Pipeline has no `from`/`to` options:
    // stdout into stdin is the only wiring, so all three spellings agree.
    let pipe = Pipeline::new(vec![
        command_template("echo hello"),
        command_template("tr a-z A-Z"),
    ]);
    let out = pipe.run().unwrap();
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout_text().trim(), "HELLO");
}

#[test]
fn spawn_non_zero_exit_code() {
    // :695
    let out = collect(&command_array("sh", &["-c", "exit 1"]));
    assert_eq!(out.code, Some(1));
}

#[test]
fn spawn_fail_for_invalid_command() {
    // :703
    let error = spawn_err(&command_array("nonexistent-command-12345", &[]));
    assert!(matches!(error, process::ProcessError::Spawn(_)), "{error:?}");
}

#[test]
fn spawn_error_with_invalid_cwd() {
    // :712
    let error = spawn_err(&command_array("echo", &["test"]).cwd("/nonexistent/directory/path"));
    assert!(matches!(error, process::ProcessError::Spawn(_)), "{error:?}");
}

#[test]
fn spawn_permission_denied_typed_error() {
    // :721 — a script without its execute bit is refused before it runs.
    let path = std::env::temp_dir().join(format!("ee-noperm-{}.sh", std::process::id()));
    std::fs::write(&path, b"#!/bin/sh\necho hi\n").unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o644);
    std::fs::set_permissions(&path, perms).unwrap();

    let error = spawn_err(&command_array(path.to_str().unwrap(), &[]));
    assert!(
        matches!(error, process::ProcessError::PermissionDenied(_)),
        "{error:?}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn spawn_large_stdout_no_deadlock() {
    // :1143
    let out = collect(&command_array("seq", &["1", "100000"]));
    assert_eq!(out.code, Some(0));
    let text = out.stdout_text();
    let lines: Vec<&str> = text.trim().split('\n').collect();
    assert_eq!(lines.len(), 100000);
    assert_eq!(lines[0], "1");
    assert_eq!(lines[99999], "100000");
}

#[test]
fn spawn_string_large_output_no_deadlock() {
    // :1163 — `run` is the string convenience: collect stdout, no deadlock.
    let out = process::run("seq", &["1", "100000"]).unwrap();
    let text = out.stdout_text();
    let lines: Vec<&str> = text.trim().split('\n').collect();
    assert_eq!(lines.len(), 100000);
    assert_eq!(lines[0], "1");
    assert_eq!(lines[99999], "100000");
}
