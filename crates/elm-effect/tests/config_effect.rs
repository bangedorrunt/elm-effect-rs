//! `Config.mapEffect` cases, seen from outside the crate.
//!
//! These were `#[cfg(test)] mod tests` inside `config_effect.rs` while `config`
//! was a private module. They are the proof that the surface opens: every name
//! here is reachable by an application.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use elm_effect::config::Value;
use elm_effect::config_provider::{Lookup, Node, Provider, ProviderError};
use elm_effect::{Config, ConfigError, Defect, Effect};

fn provider(pairs: &[(&str, &str)]) -> Provider {
    Provider::from_pairs(
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string())),
    )
}

fn record(pairs: Vec<(&str, Value)>) -> Value {
    Value::Map(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
}

fn increment(value: Value) -> Value {
    match value {
        Value::Int(n) => Value::Int(n + 1),
        other => other,
    }
}

#[test]
fn mapped_effectfully_mapped_and_constant_siblings_decode_alike() {
    // Config.test.ts:1060 — the three arms differ only in how `port` is
    // produced. With `host` absent each group is a missing-key group, so
    // the default and the option both recover it.
    let fallback = record(vec![
        ("port", Value::Int(3000)),
        ("host", Value::Str("localhost".into())),
    ]);
    let arms = [
        Config::integer(Some("port".to_string())).map(increment),
        Config::integer(Some("port".to_string()))
            .map_effect(|value| Effect::<Value, ConfigError, ()>::succeed(increment(value))),
        Config::succeed(Value::Int(81)),
    ];

    for arm in arms {
        let config = Config::record(vec![
            ("port".to_string(), arm),
            ("host".to_string(), Config::string(Some("host".to_string()))),
        ]);
        assert_eq!(
            config
                .clone()
                .with_default(fallback.clone())
                .load(&provider(&[("port", "80")])),
            Ok(fallback.clone())
        );
        assert_eq!(
            config.option().load(&provider(&[("port", "80")])),
            Ok(Value::None)
        );
    }
}

#[test]
fn or_else_recovers_a_map_effect_failure_and_the_group_then_defaults() {
    // Config.test.ts:1173 — the transform fails with a config error, the
    // sibling `or_else` recovers it, and `required` is still absent, so the
    // whole record falls back.
    let validation = ConfigError::Invalid {
        path: Vec::new(),
        got: "\"value\"".to_string(),
        reason: "invalid value".to_string(),
    };
    let recovered = Config::string(Some("recovered".to_string()))
        .map_effect(move |_| Effect::<Value, ConfigError, ()>::fail(validation.clone()))
        .or_else(Config::succeed(Value::Str("fallback".into())));
    let fallback = record(vec![
        ("recovered", Value::Str("default".into())),
        ("required", Value::Str("default".into())),
    ]);
    let config = Config::record(vec![
        ("recovered".to_string(), recovered),
        (
            "required".to_string(),
            Config::string(Some("required".to_string())),
        ),
    ])
    .with_default(fallback.clone());

    assert_eq!(
        config.load(&provider(&[("recovered", "value")])),
        Ok(fallback)
    );
}

#[test]
fn a_defect_in_the_transform_refuses_with_the_descriptor_path() {
    // The ceiling named in the module doc: a defect is not a config error in
    // Effect 4, but a load has no defect channel, so it refuses at the
    // descriptor's path instead of passing a wrong value through.
    let config = Config::string(Some("port".to_string()))
        .map_effect(|_| Effect::<Value, ConfigError, ()>::die(Defect::new("boom")));
    match config.load(&provider(&[("port", "80")])) {
        Err(errors) => match &errors[0] {
            ConfigError::Invalid { path, reason, .. } => {
                assert_eq!(path, &vec!["port".to_string()]);
                assert_eq!(reason, "the mapping effect failed with a defect");
            }
            other => panic!("expected an invalid value, got {other:?}"),
        },
        Ok(value) => panic!("expected a refusal, got {value:?}"),
    }
}

mod provider_defect {
    use super::*;

    /// Every path reads as absent except one, where the provider dies
    /// (`ConfigProvider.make(() => Effect.die(defect))`).
    struct DyingProvider {
        failed_path: Vec<String>,
        defect: Defect,
    }

    impl Lookup for DyingProvider {
        fn get(&self, path: &[String]) -> Option<String> {
            self.try_get(path).ok().flatten()
        }

        fn has_children(&self, _path: &[String]) -> bool {
            true
        }

        fn node(&self, _path: &[String]) -> Option<Node> {
            Some(Node::Object)
        }

        fn try_get(&self, path: &[String]) -> Result<Option<String>, ProviderError> {
            if path == self.failed_path.as_slice() {
                Err(ProviderError::defect(self.defect.clone()))
            } else {
                Ok(None)
            }
        }
    }

    /// Config.test.ts:65 — a provider defect is not a config error. Neither
    /// the fallback nor the option may answer for it; the load returns the
    /// defect itself so a caller can surface the provider's own failure.
    #[test]
    fn a_provider_defect_survives_or_else_and_option() {
        let failed_path = vec!["settings".to_string(), "value".to_string()];
        let provider = DyingProvider {
            failed_path: failed_path.clone(),
            defect: Defect::new("provider defect"),
        };
        let recovered = Arc::new(AtomicBool::new(false));
        let config = Config::string(Some("value".to_string()))
            .nested("settings")
            .or_else_with({
                let recovered = Arc::clone(&recovered);
                move |_| {
                    recovered.store(true, Ordering::SeqCst);
                    Config::succeed(Value::Str("fallback".into()))
                }
            })
            .option();

        match config.load(&provider) {
            Err(errors) => {
                assert_eq!(errors.len(), 1, "one defect, not a pile of errors");
                match &errors[0] {
                    ConfigError::Defect { defect } => {
                        assert_eq!(defect.message, "provider defect")
                    }
                    other => panic!("expected the provider's defect, got {other:?}"),
                }
            }
            Ok(value) => panic!("a defect must not load: {value:?}"),
        }
        assert!(
            !recovered.load(Ordering::SeqCst),
            "orElse must not answer for a defect"
        );
    }
}

#[test]
fn log_level_is_shared_by_config_and_observability() {
    // The compiler is the witness: a level parsed through `config` is the
    // logger's own type, with no conversion.
    let parsed = elm_effect::config::LogLevel::parse("Warning").expect("valid");
    assert_eq!(parsed, elm_effect::observability::LogLevel::Warn);
    assert!(elm_effect::observability::is_enabled(
        parsed,
        elm_effect::config::LogLevel::Info
    ));
}
