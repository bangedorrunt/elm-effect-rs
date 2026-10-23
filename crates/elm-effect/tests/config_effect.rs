//! `Config.mapEffect` rows, seen from outside the crate.
//!
//! These were `#[cfg(test)] mod tests` inside `config_effect.rs` while `config`
//! was a private module. They are the proof that the surface opens: every name
//! here is reachable by an application.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use elm_effect::config::Value;
use elm_effect::config_provider::{Lookup, Node, Provider, ProviderError};
use elm_effect::{Config, ConfigError, Defect, Effect, Exit, Runtime};

#[cfg(feature = "schema")]
use elm_effect::Schema;

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
            (
                "host".to_string(),
                Config::string(Some("host".to_string())),
            ),
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
    // the pin, but a load has no defect channel, so it refuses at the
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

#[cfg(feature = "schema")]
mod provider_defect {
    use super::*;

    /// The row's provider: every path answers a record with one `value` key,
    /// except one path, where the provider dies
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
                // No scalar anywhere else: the record's keys live as
                // children of the node.
                Ok(None)
            }
        }
    }

    /// Config.test.ts:65 — a provider defect is not a config error.
    ///
    /// The fallback must never run, the option must not turn the defect into
    /// an absence, and `catchDefect` must receive the provider's own defect,
    /// at the root and at a nested field alike.
    #[test]
    fn a_provider_defect_reaches_catch_defect_and_no_fallback_answers_it() {
        let paths = [
            vec!["settings".to_string()],
            vec!["settings".to_string(), "value".to_string()],
        ];
        for failed_path in paths {
            let provider = DyingProvider {
                failed_path: failed_path.clone(),
                defect: Defect::new("provider defect"),
            };
            let schema = Schema::struct_([("value", Schema::as_unknown(Schema::string()))]);
            let recovered = Arc::new(AtomicBool::new(false));
            let config = Config::schema(&schema, Some("settings".to_string())).or_else_with({
                let recovered = Arc::clone(&recovered);
                move |_| {
                    recovered.store(true, Ordering::SeqCst);
                    Config::succeed(Value::Str("fallback".into()))
                }
            })
            .option();

            let caught = Arc::new(Mutex::new(None::<Defect>));
            let effect = config.load_effect(&provider).catch_defect({
                let caught = Arc::clone(&caught);
                move |defects| {
                    *caught.lock().unwrap() = defects.into_iter().next();
                    Effect::succeed(Value::Str("caught".into()))
                }
            });

            assert_eq!(
                Runtime::run(effect),
                Exit::Success(Value::Str("caught".into())),
                "{failed_path:?}: the defect must reach catch_defect"
            );
            assert_eq!(
                caught.lock().unwrap().as_ref().map(|d| d.message.clone()),
                Some("provider defect".to_string()),
                "{failed_path:?}: the provider's own defect must arrive"
            );
            assert!(
                !recovered.load(Ordering::SeqCst),
                "{failed_path:?}: orElse must not answer for a defect"
            );
        }
    }
}
