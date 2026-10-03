//! Compile-fail fixture: an effect requiring `Database` is used where `()`
//! (no requirements) is expected, with no `provide` in between. This file
//! must NEVER compile; `missing_service_does_not_compile` asserts that.

use elm_effect::{Effect, ServiceTag};

pub struct Database;

impl ServiceTag for Database {
    type Service = String;
    const ID: &'static str = "myapp/db/Database";
}

pub fn broken() -> Effect<String, String, ()> {
    let e: Effect<String, String, Database> = Effect::succeed("x".to_string());
    e
}
