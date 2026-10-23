//! `Config.mapEffect`'s driver: how a config load runs one described
//! transform.
//!
//! The pin's config evaluation is itself effect-shaped, so `mapEffect` is just
//! a `flatMap` on the resolution. This port's evaluation is a synchronous
//! `Result` (see `config.rs`), which leaves the transform one place to run: the
//! calling thread, through the same interpreter `Runtime::run` uses. So the
//! transform must be an effect that finishes without suspending, and the two
//! shapes worth naming are carried here rather than left to each caller:
//!
//! - a typed failure is the load's failure, with the descriptor's path, which
//!   `or_else` recovers from the way the pin's `orElse` does;
//! - a defect has no channel in a `Result<Value, Vec<ConfigError>>`, so it
//!   surfaces as [`ConfigError::Invalid`] carrying the rendered cause. That is
//!   the honest ceiling of a load with no fiber: the config still refuses, but
//!   it says "invalid value" where the pin would have died.

use crate::config::{ConfigEffect, ConfigError, Value};
use crate::Effect;

impl ConfigEffect for Effect<Value, ConfigError, ()> {
    fn resolve(self, path: &[String]) -> Result<Value, ConfigError> {
        match self.run_sync() {
            Ok(value) => Ok(value),
            Err(cause) => match cause.failure() {
                Some(error) => Err(error.clone()),
                None => Err(ConfigError::Invalid {
                    path: path.to_vec(),
                    got: cause.to_string(),
                    reason: "the mapping effect failed with a defect".to_string(),
                }),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::config_provider::Provider;
    use crate::Effect;

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
        let fallback = record(vec![("port", Value::Int(3000)), ("host", Value::Str("localhost".into()))]);
        let arms = [
            Config::integer(Some("port".to_string())).map(increment),
            Config::integer(Some("port".to_string())).map_effect(|value| {
                Effect::<Value, ConfigError, ()>::succeed(increment(value))
            }),
            Config::succeed(Value::Int(81)),
        ];

        for arm in arms {
            let config = Config::record(vec![
                ("port".to_string(), arm),
                ("host".to_string(), Config::string(Some("host".to_string()))),
            ]);
            assert_eq!(
                config.clone().with_default(fallback.clone()).load(&provider(&[("port", "80")])),
                Ok(fallback.clone())
            );
            assert_eq!(config.option().load(&provider(&[("port", "80")])), Ok(Value::None));
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
        // The ceiling named in the module doc: a defect is not a config error
        // in the pin, but a load has no defect channel, so it refuses at the
        // descriptor's path instead of passing a wrong value through.
        let config = Config::string(Some("port".to_string())).map_effect(|_| {
            Effect::<Value, ConfigError, ()>::die(crate::Defect::new("boom"))
        });
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
}
