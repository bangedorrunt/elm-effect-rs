//! `Struct.ts` port: functional record updates over sorted string maps.
//!
//! TypeScript structs are static shapes; the portable runtime story is
//! key-directed evolve/pick/omit over `BTreeMap<String, V>`. Field-typed
//! structs stay native Rust structs (the compiler checks those).

use std::collections::BTreeMap;

/// Keep only the listed keys (`pick`).
pub fn pick<V: Clone>(record: &BTreeMap<String, V>, keys: &[&str]) -> BTreeMap<String, V> {
    keys.iter()
        .filter_map(|k| record.get(*k).map(|v| (k.to_string(), v.clone())))
        .collect()
}

/// Drop the listed keys (`omit`).
pub fn omit<V: Clone>(record: &BTreeMap<String, V>, keys: &[&str]) -> BTreeMap<String, V> {
    record
        .iter()
        .filter(|(k, _)| !keys.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Transform a subset of keys in place shape (`evolve`): each listed key
/// maps through its function; unlisted keys pass through unchanged.
pub fn evolve<V: Clone>(
    record: &BTreeMap<String, V>,
    updates: &[(&str, Box<dyn Fn(V) -> V>)],
) -> BTreeMap<String, V> {
    let mut out = record.clone();
    for (key, f) in updates {
        if let Some(current) = out.remove(*key) {
            out.insert(key.to_string(), f(current));
        }
    }
    out
}

/// Read a field (`get`).
pub fn get<V: Clone>(record: &BTreeMap<String, V>, key: &str) -> Option<V> {
    record.get(key).cloned()
}

/// Set a field, replacing any previous value (`set`).
pub fn set<V>(record: &mut BTreeMap<String, V>, key: String, value: V) {
    record.insert(key, value);
}
