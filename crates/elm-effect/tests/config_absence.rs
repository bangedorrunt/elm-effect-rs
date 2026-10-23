//! Config absence semantics: option, default, and group recovery.
//!
//! `config` is parked behind a private module, so it loads by path like the
//! other wave tests. Most of its surface is unexercised here, so dead-code
//! warnings describe this test's coverage, not a defect in the module.
#![allow(dead_code)]

#[path = "../src/config.rs"]
mod config;
#[path = "../src/config_provider.rs"]
mod config_provider;

use config::{Config, ConfigError, Value};
use config_provider::Provider;

fn provider(pairs: &[(&str, &str)]) -> Provider {
    Provider::from_pairs(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())))
}

#[test]
fn config_option_maps_absence_to_none() {
    // Config.test.ts:806 — a present value is Some, absence is None, and an
    // empty env value reads as absent unless the provider preserves it.
    let finite = Config::finite(Some("a".to_string())).option();
    assert_eq!(finite.load(&provider(&[("a", "1")])), Ok(Value::Num(1.0)));
    assert_eq!(finite.load(&Provider::empty()), Ok(Value::None));
    assert_eq!(
        finite.load(&provider(&[("a", "")])),
        Ok(Value::None),
        "an empty value is absent by default"
    );

    let string = Config::string(Some("a".to_string())).option();
    let preserving = Provider::from_pairs_preserving([("a".to_string(), String::new())]);
    assert_eq!(string.load(&preserving), Ok(Value::Str(String::new())));

    // A present but undecodable value is a failure, not absence.
    match finite.load(&provider(&[("a", "value")])) {
        Err(errors) => assert!(
            errors
                .iter()
                .all(|error| matches!(error, ConfigError::Invalid { .. })),
            "expected only invalid input, got {errors:?}"
        ),
        Ok(value) => panic!("expected a failure, got {value:?}"),
    }
}

#[test]
fn config_option_over_a_group_requires_every_child() {
    // :826 — the group is Some only when every required child is present.
    let config = Config::record(vec![
        ("a".to_string(), Config::non_empty_string(Some("b".to_string()))),
        ("c".to_string(), Config::finite(Some("d".to_string()))),
    ])
    .option();

    assert_eq!(
        config.load(&provider(&[("b", "b"), ("d", "1")])),
        Ok(Value::Map(vec![
            ("a".to_string(), Value::Str("b".to_string())),
            ("c".to_string(), Value::Num(1.0)),
        ]))
    );
    assert_eq!(config.load(&Provider::empty()), Ok(Value::None));
    assert_eq!(config.load(&provider(&[("b", "b")])), Ok(Value::None));
    assert_eq!(config.load(&provider(&[("d", "1")])), Ok(Value::None));
    assert_eq!(
        config.load(&provider(&[("b", ""), ("d", "1")])),
        Ok(Value::None),
        "an empty required child is absent"
    );

    let preserving = Provider::from_pairs_preserving([
        ("b".to_string(), String::new()),
        ("d".to_string(), "1".to_string()),
    ]);
    assert!(
        config.load(&preserving).is_err(),
        "a preserved empty child is invalid, not absent"
    );
}

#[test]
fn config_log_level_default_does_not_cover_invalid_input() {
    // :684 — a missing key takes the default; a present but invalid level
    // fails instead of falling back.
    let config =
        Config::log_level(Some("LOG_LEVEL".to_string())).with_default(Value::Str("Info".to_string()));
    assert_eq!(
        config.load(&Provider::empty()),
        Ok(Value::Str("Info".to_string()))
    );
    assert!(
        config.load(&provider(&[("LOG_LEVEL", "debug")])).is_err(),
        "a present invalid level must not use the default"
    );
}
