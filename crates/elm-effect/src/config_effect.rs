//! `Config.mapEffect`'s driver and `Config.load_effect`: how a config result
//! becomes an effect.
//!
//! The pin's config evaluation is itself effect-shaped, so `mapEffect` is just
//! a `flatMap` on the resolution and a provider defect is an ordinary defect in
//! the parsing effect. This port's evaluation is a synchronous `Result` (see
//! `config.rs`), which leaves two seams for this module:
//!
//! - a `mapEffect` transform runs on the calling thread, through the same
//!   interpreter `Runtime::run` uses, so it must be an effect that finishes
//!   without suspending. A typed failure is the load's failure, with the
//!   descriptor's path, which `or_else` recovers from the way the pin's
//!   `orElse` does; a defect has no channel in a `Result<Value, Vec<ConfigError>>`,
//!   so it surfaces as [`ConfigError::Invalid`] carrying the rendered cause.
//!   That is the honest ceiling of a load with no fiber: the config still
//!   refuses, but it says "invalid value" where the pin would have died.
//! - `load_effect` reifies a *provider* defect back into the cause it belongs
//!   to, because the load can only report it as [`ConfigError::Defect`]
//!   (`Config.test.ts:65`).

use crate::Effect;
use crate::config::{ConfigEffect, ConfigError, Value};
use crate::config_provider::Lookup;

impl crate::config::Config {
    /// `Config.parse` as an effect: the load's typed failures are the effect's
    /// error, and a provider that died is the effect's *defect*.
    ///
    /// The pin's `parse` fails with a `ConfigError`; a provider defect is not
    /// one of those, so `orElse` and `option` never see it and only
    /// `catchDefect` can answer for it (`Config.test.ts:65`). A load here is a
    /// synchronous `Result`, which has no defect channel, so this is the one
    /// place that reifies `ConfigError::Defect` back into a cause.
    pub fn load_effect(&self, provider: &dyn Lookup) -> Effect<Value, Vec<ConfigError>, ()> {
        match self.load(provider) {
            Ok(value) => Effect::succeed(value),
            Err(errors) => {
                let defect = errors.iter().find_map(|error| match error {
                    ConfigError::Defect { defect } => Some(defect.clone()),
                    _ => None,
                });
                match defect {
                    Some(defect) => Effect::die(defect),
                    None => Effect::fail(errors),
                }
            }
        }
    }
}

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
