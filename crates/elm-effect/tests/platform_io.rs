//! Wave-14 platform I/O: Stdio, Terminal, Console services with fakes.
//! Modules load via `#[path]` includes (the .15 precedent): no facade touch.
//! Fakes observe writes without a terminal; exit codes pin the run_main
//! discipline (0/1/2/130).

#[path = "../src/platform/console.rs"]
mod console;
#[path = "../src/platform/stdio.rs"]
mod stdio;
#[path = "../src/platform/terminal.rs"]
mod terminal;

use console::{EXIT_DEFECT, EXIT_FAILURE, EXIT_INTERRUPTED, EXIT_SUCCESS, FakeConsole};
use stdio::{FakeStdio, Stdio};
use terminal::{FakeTerminal, QuitError, Terminal};

#[test]
fn stdio_fake_records_and_replays() {
    let io = FakeStdio::new(vec!["hello".to_owned()], vec!["prog".to_owned()]);
    assert_eq!(io.args(), ["prog".to_owned()]);
    assert!(!io.stdin_is_terminal());
    assert!(!io.stdout_is_terminal());
    io.write_stdout("out-1\n");
    io.write_stderr("err-1\n");
    assert_eq!(io.stdout_chunks(), ["out-1\n".to_owned()]);
    assert_eq!(io.stderr_chunks(), ["err-1\n".to_owned()]);
    assert_eq!(io.read_stdin_line(), Some("hello".to_owned()));
    assert_eq!(io.read_stdin_line(), None);
}

#[test]
fn terminal_fake_scripts_and_quits() {
    let term = FakeTerminal::new(vec![Ok("yes".to_owned()), Err(QuitError)]);
    assert_eq!(term.read_line("> "), Ok("yes".to_owned()));
    assert_eq!(term.read_line("> "), Err(QuitError));
    assert_eq!(term.read_line("> "), Err(QuitError));
    assert_eq!(
        term.prompts(),
        ["> ".to_owned(), "> ".to_owned(), "> ".to_owned()]
    );
    assert!(!term.is_terminal());
}

#[test]
fn console_fake_splits_levels() {
    let fake = FakeConsole::new();
    let console = fake.console();
    console.log("hello");
    console.error("boom");
    console.debug("dbg");
    assert_eq!(fake.stdout_lines(), ["hello".to_owned()]);
    assert_eq!(fake.stderr_lines(), ["boom".to_owned(), "dbg".to_owned()]);
}

#[test]
fn exit_codes_pin_discipline() {
    assert_eq!(EXIT_SUCCESS, 0);
    assert_eq!(EXIT_FAILURE, 1);
    assert_eq!(EXIT_DEFECT, 2);
    assert_eq!(EXIT_INTERRUPTED, 130);
}
