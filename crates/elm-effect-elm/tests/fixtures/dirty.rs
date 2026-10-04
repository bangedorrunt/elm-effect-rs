//! Dirty fixture: every lint must fire here and name the hazard.
//! Not compiled as code (included as text by `lints.rs`).

use std::collections::HashMap;
use std::time::SystemTime;

pub struct AppModel {
    pub counts: HashMap<String, u64>,
    pub last_seen: u64,
}

pub fn update(model: &AppModel, msg: &str) -> AppModel {
    let now = SystemTime::now();
    let home = std::env::var("HOME").unwrap_or_default();
    let jitter: u64 = rand::thread_rng().gen_range(0..10);
    let _ = (now, home, jitter);
    let mut counts = model.counts.clone();
    *counts.entry(msg.to_string()).or_insert(0) += 1;
    AppModel { counts, last_seen: model.last_seen + 1 }
}
