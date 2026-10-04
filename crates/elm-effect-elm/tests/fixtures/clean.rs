//! Clean fixture: no lint may fire here.
//! Not compiled as code (included as text by `lints.rs`).

use std::collections::BTreeMap;

pub struct AppModel {
    pub counts: BTreeMap<String, u64>,
    pub last_seen: u64,
}

pub fn update(model: &AppModel, msg: &str) -> AppModel {
    let mut counts = model.counts.clone();
    *counts.entry(msg.to_string()).or_insert(0) += 1;
    AppModel { counts, last_seen: model.last_seen + 1 }
}
