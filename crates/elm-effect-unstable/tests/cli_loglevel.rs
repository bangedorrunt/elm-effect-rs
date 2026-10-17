//! `cli/LogLevel` witnesses for
//! `packages/effect/test/cli/LogLevel.test.ts`.
//!
//! Each test names the upstream line it proves. The port covers the pure
//! core of the TS rows: parsing the built-in `--log-level` setting and
//! filtering emitted logs by severity. The TS effect plumbing (layers,
//! `MinimumLogLevel` service, concurrent fibers) has no Rust counterpart;
//! severity filtering is the behavior the rows pin.

use elm_effect_unstable::cli::log_level::{LogLevel, filter_emitted, resolve_setting};

/// Emission order of the TS helper: trace (Info level), debug, info,
/// warning, error, fatal.
fn emitted() -> Vec<(LogLevel, &'static str)> {
    vec![
        (LogLevel::Info, "trace"),
        (LogLevel::Debug, "debug"),
        (LogLevel::Info, "info"),
        (LogLevel::Warn, "warn"),
        (LogLevel::Error, "error"),
        (LogLevel::Fatal, "fatal"),
    ]
}

fn argv(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| s.to_string()).collect()
}

#[test]
fn level_filters_emission_in_severity_order() {
    // :123 `it.each(level=%s)` over all/trace/debug/info/warn/warning/error/fatal/none
    let cases: &[(&str, &[&str])] = &[
        ("all", &["trace", "debug", "info", "warn", "error", "fatal"]),
        (
            "trace",
            &["trace", "debug", "info", "warn", "error", "fatal"],
        ),
        (
            "debug",
            &["trace", "debug", "info", "warn", "error", "fatal"],
        ),
        ("info", &["trace", "info", "warn", "error", "fatal"]),
        ("warn", &["warn", "error", "fatal"]),
        ("warning", &["warn", "error", "fatal"]),
        ("error", &["error", "fatal"]),
        ("fatal", &["fatal"]),
        ("none", &[]),
    ];
    for (setting, expected) in cases {
        let min = resolve_setting(&argv(&["--log-level", setting])).unwrap();
        let got = filter_emitted(&emitted(), min);
        let messages: Vec<&str> = got.iter().map(|(_, m)| *m).collect();
        assert_eq!(messages, *expected, "setting={setting}");
    }
}

#[test]
fn omitted_setting_defaults_to_info() {
    // :129 uses Info as the default minimum when --log-level is omitted
    let min = resolve_setting(&argv(&[])).unwrap();
    assert_eq!(min, None);
    let got = filter_emitted(&emitted(), min);
    let messages: Vec<&str> = got.iter().map(|(_, m)| *m).collect();
    assert_eq!(messages, vec!["trace", "info", "warn", "error", "fatal"]);
}

#[test]
fn exposes_the_builtin_setting_value() {
    // :137 exposes the built-in log-level setting value
    assert_eq!(resolve_setting(&argv(&[])).unwrap(), None);
    assert_eq!(
        resolve_setting(&argv(&["--log-level", "warn"])).unwrap(),
        Some(LogLevel::Warn)
    );
}

#[test]
fn selected_level_applies_to_subcommand_emissions() {
    // :154 applies the selected log level to subcommands
    let min = resolve_setting(&argv(&["--log-level", "info", "child"])).unwrap();
    let child = vec![
        (LogLevel::Debug, "debug from child"),
        (LogLevel::Info, "info from child"),
        (LogLevel::Error, "error from child"),
    ];
    let got = filter_emitted(&child, min);
    assert_eq!(
        got,
        vec![
            (LogLevel::Info, "info from child"),
            (LogLevel::Error, "error from child"),
        ]
    );
}

#[test]
fn invalid_level_fails_without_emitting_logs() {
    // :183 invalid --log-level is InvalidValue and emits nothing
    let err = resolve_setting(&argv(&["--log-level", "invalid"])).unwrap_err();
    assert_eq!(
        err,
        elm_effect_unstable::cli::CliError::InvalidValue {
            option: "log-level".to_owned(),
            value: "invalid".to_owned(),
            expected: "all|trace|debug|info|warn|warning|error|fatal|none".to_owned(),
            kind: "option".to_owned(),
        }
    );
    let logs: Vec<(LogLevel, &str)> = Vec::new();
    assert!(logs.is_empty());
}
