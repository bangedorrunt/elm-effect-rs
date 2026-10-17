//! Compile-fail fixture: providing a non-head tag of a requirement tuple.
//! Requirement tuples are ordered by provision: `Effect<A, E, (Db, Clock)>`
//! provides `Db` first. Providing `Clock` while `Db` is still the head must
//! be a compile error with a clear diagnostic, not a silent misroute.
//! This file must NEVER compile; the `provide_wrong_order` test asserts that.

use elm_effect::{Effect, ServiceTag};

pub struct Db(pub String);
pub struct Database;
impl ServiceTag for Database {
    type Service = Db;
    const ID: &'static str = "myapp/db/Database";
}

pub struct Tick(pub String);
pub struct Clock;
impl ServiceTag for Clock {
    type Service = Tick;
    const ID: &'static str = "myapp/clock/Clock";
}

pub fn broken() {
    let e: Effect<String, String, (Database, Clock)> = Effect::succeed("x".to_string());
    let _ = e.provide::<Clock, (Database,)>(Tick("t".to_string()));
}
