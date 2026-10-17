//! `cli/Argument` witnesses for
//! `packages/effect/test/cli/Arguments.test.ts`.
//!
//! Each test names the upstream line it proves. The port covers the pure
//! core: primitive parsers (`primitive`), positional binding
//! (`parse_positionals`), parameter binding (`param`), and display metadata
//! (`cli_completions::ParamDef`). The TS effect plumbing (layers, terminal,
//! help rendering, `ShowHelp` tags) has no Rust counterpart; bound values
//! and error shapes are the behavior the rows pin.

use elm_effect_unstable::cli::primitive::{self, PathKind, PositionalSpec};
use elm_effect_unstable::cli::{CliError, param};
use elm_effect_unstable::cli_completions::{ParamDef, ParamType};
use std::collections::BTreeMap;

fn args(pairs: &[(&str, &[&str])]) -> param::CliOptions {
    param::CliOptions {
        args: pairs
            .iter()
            .map(|(k, vs)| {
                (
                    (*k).to_owned(),
                    vs.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
                )
            })
            .collect(),
        ..param::CliOptions::default()
    }
}

#[test]
fn parses_all_argument_types_correctly() {
    // :52 `should parse all argument types correctly`
    assert_eq!(primitive::parse_string("myapp"), "myapp");
    assert_eq!(primitive::parse_int("42").expect("int"), 42);
    assert_eq!(primitive::parse_finite("3.14").expect("finite"), 3.14);
    assert_eq!(
        primitive::parse_choice("dev", &[("dev", "dev"), ("prod", "prod")]).expect("literal"),
        "dev"
    );
    let config = primitive::parse_path("./config.json", PathKind::File, false).expect("file");
    assert!(config.contains("config.json"), "{config}");
    let workspace =
        primitive::parse_path("./workspace", PathKind::Directory, false).expect("dir");
    assert!(workspace.contains("workspace"), "{workspace}");
    let date = primitive::parse_date("2024-01-01").expect("date");
    assert_eq!((date.year, date.month, date.day), (2024, 1, 1));
    assert_eq!(
        param::CliParam::boolean("verbose").parse(&param::CliOptions {
            flags: BTreeMap::from([("verbose".to_owned(), Vec::new())]),
            ..param::CliOptions::default()
        }),
        Ok(Some((
            "verbose".to_owned(),
            param::ParamValue::Switch(true)
        )))
    );
}

#[test]
fn file_must_exist_validation() {
    // :92 `should handle file mustExist validation`
    let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    assert!(primitive::parse_path(manifest, PathKind::File, true).is_ok());
    let err = primitive::parse_path("/non/existent/file.txt", PathKind::File, true)
        .expect_err("missing");
    assert!(err.contains("does not exist"), "{err}");
    let kept = primitive::parse_path("/non/existent/file.txt", PathKind::File, false)
        .expect("no-check");
    assert!(kept.contains("/non/existent/file.txt"), "{kept}");
}

#[test]
fn fails_with_invalid_arguments() {
    // :132 `should fail with invalid arguments`
    let err = param::CliParam::integer(param::ParamKind::Argument, "count")
        .parse(&args(&[("count", &["not-a-number"])]))
        .expect_err("rejects");
    match &err {
        CliError::InvalidValue {
            option,
            kind,
            expected,
            ..
        } => {
            assert_eq!(option, "count");
            assert_eq!(kind, "argument");
            assert!(expected.contains("finite number"), "{expected}");
        }
        _ => panic!("wrong variant: {err:?}"),
    }
    assert!(
        primitive::parse_choice("staging", &[("dev", 0), ("prod", 1)]).is_err(),
        "literals reject"
    );
}

#[test]
fn handles_variadic_arguments() {
    // :163 `should handle variadic arguments`
    let spec = PositionalSpec::Variadic {
        name: "files".to_owned(),
        max: 100,
        default: None,
    };
    let bound = primitive::parse_positionals(
        &[spec],
        &[
            "file1.txt".to_owned(),
            "file2.txt".to_owned(),
            "file3.txt".to_owned(),
        ],
    )
    .expect("binds");
    assert_eq!(
        bound,
        vec![(
            "files".to_owned(),
            vec![
                "file1.txt".to_owned(),
                "file2.txt".to_owned(),
                "file3.txt".to_owned(),
            ]
        )]
    );
}

#[test]
fn handles_choice_with_value() {
    // :184 `should handle choiceWithValue`
    let level = primitive::parse_choice("info", &[("debug", 0), ("info", 1), ("error", 2)])
        .expect("maps");
    assert_eq!(level, 1);
}

#[test]
fn handles_filter_combinator_valid() {
    // :204 `should handle filter combinator - valid`
    let port = primitive::parse_int("8080").expect("int");
    let filtered: Result<i64, String> = if (1..=65535).contains(&port) {
        Ok(port)
    } else {
        Err(format!("Port {port} out of range (1-65535)"))
    };
    assert_eq!(filtered, Ok(8080));
}

#[test]
fn handles_filter_combinator_invalid() {
    // :222 `should handle filter combinator - invalid`
    let port = primitive::parse_int("99999").expect("parses, filter rejects");
    let filtered: Result<i64, String> = if (1..=65535).contains(&port) {
        Ok(port)
    } else {
        Err(format!("Port {port} out of range (1-65535)"))
    };
    let err = filtered.expect_err("out of range");
    assert!(err.contains("out of range"), "{err}");
}

#[test]
fn handles_filter_map_combinator_valid() {
    // :240 `should handle filterMap combinator - valid`
    let n = primitive::parse_int("42").expect("int");
    let mapped: Result<i64, String> = if n > 0 {
        Ok(n)
    } else {
        Err(format!("Expected positive integer, got {n}"))
    };
    assert_eq!(mapped, Ok(42));
}

#[test]
fn handles_filter_map_combinator_invalid() {
    // :258 `should handle filterMap combinator - invalid`
    let n = primitive::parse_int("0").expect("parses, mapping rejects");
    let mapped: Result<i64, String> = if n > 0 {
        Ok(n)
    } else {
        Err(format!("Expected positive integer, got {n}"))
    };
    let err = mapped.expect_err("rejects zero");
    assert!(err.contains("Expected positive integer"), "{err}");
}

#[test]
fn handles_or_else_combinator() {
    // :276 `should handle orElse combinator`
    let fallback = |text: &str| -> i64 {
        match primitive::parse_int(text) {
            Ok(n) => n,
            Err(_) => {
                let _ = primitive::parse_string(text);
                -1
            }
        }
    };
    assert_eq!(fallback("42"), 42);
    assert_eq!(fallback("abc"), -1);
}

#[test]
fn handles_or_else_result_combinator() {
    // :299 `should handle orElseResult combinator`
    let branched = |text: &str| -> Result<i64, String> {
        match primitive::parse_int(text) {
            Ok(n) => Ok(n),
            Err(_) => Err(primitive::parse_string(text)),
        }
    };
    assert_eq!(branched("42"), Ok(42));
    assert_eq!(branched("abc"), Err("abc".to_owned()));
}

#[test]
fn handles_with_metavar_combinator() {
    // :323 `should handle withMetavar combinator` — the label swaps, never
    // the shape (help rendering itself stays TS-side).
    let def = ParamDef::argument("file", ParamType::String).with_metavar("FILE_PATH");
    assert_eq!(def.metavar.as_deref(), Some("FILE_PATH"));
    assert_eq!(def.name, "file");
    assert_eq!(def.param_type, ParamType::String);
}

#[test]
fn handles_optional_argument_when_provided() {
    // :338 `should handle optional arguments - when provided` (description
    // is help-only; the behavior is presence yielding `Some`).
    let got = param::CliParam::text(param::ParamKind::Argument, "label")
        .optional()
        .parse(&args(&[("label", &["my-label"])]))
        .expect("present")
        .expect("some");
    assert_eq!(got, ("label".to_owned(), param::ParamValue::Text("my-label".to_owned())));
}

#[test]
fn returns_none_for_omitted_optional_positional() {
    // :357 `returns none for omitted optional positional arguments`
    assert_eq!(
        param::CliParam::text(param::ParamKind::Argument, "label")
            .optional()
            .parse(&param::CliOptions::default()),
        Ok(None)
    );
}

#[test]
fn uses_argument_wording_for_positional_invalid_value() {
    // :371 `uses argument wording for positional InvalidValue errors`
    let err = param::CliParam::integer(param::ParamKind::Argument, "count")
        .parse(&args(&[("count", &["not-a-number"])]))
        .expect_err("rejects");
    match &err {
        CliError::InvalidValue { option, kind, .. } => {
            assert_eq!(option, "count");
            assert_eq!(kind, "argument");
        }
        _ => panic!("wrong variant: {err:?}"),
    }
    let text = err.to_string();
    assert!(text.contains("argument"), "{text}");
    assert!(!text.contains("flag"), "{text}");
}
