//! `Struct.ts` port: functional record updates over sorted string maps.
//!
//! TypeScript structs are static shapes; the portable runtime story is
//! key-directed evolve/pick/omit over `BTreeMap<String, V>`. Field-typed
//! structs stay native Rust structs (the compiler checks those).

use std::collections::BTreeMap;

/// One key-directed update: the key plus its value function.
pub type Update<'a, V> = (&'a str, Box<dyn Fn(V) -> V>);

/// Sorted keys (`keys`).
pub fn keys<V>(record: &BTreeMap<String, V>) -> Vec<String> {
    record.keys().cloned().collect()
}

/// Map values, keeping keys (`map`).
pub fn map_values<V, U>(
    record: &BTreeMap<String, V>,
    mut f: impl FnMut(&V) -> U,
) -> BTreeMap<String, U> {
    record.iter().map(|(k, v)| (k.clone(), f(v))).collect()
}

/// Transform one entry (`evolveEntries`).
pub fn evolve_entry<V: Clone>(
    record: &BTreeMap<String, V>,
    key: &str,
    f: impl FnOnce(V) -> V,
) -> BTreeMap<String, V> {
    let mut out = record.clone();
    if let Some(current) = out.remove(key) {
        out.insert(key.to_string(), f(current));
    }
    out
}

/// Rename keys (`renameKeys`); unknown pairs are ignored.
pub fn rename_keys<V: Clone>(
    record: &BTreeMap<String, V>,
    renames: &[(&str, &str)],
) -> BTreeMap<String, V> {
    let mut out = record.clone();
    for (from, to) in renames {
        if let Some(v) = out.remove(*from) {
            out.insert(to.to_string(), v);
        }
    }
    out
}

/// Omit by predicate (`omitKeyWhen` custom shapes; the default never omits).
pub fn omit_when<V: Clone>(
    record: &BTreeMap<String, V>,
    mut predicate: impl FnMut(&str, &V) -> bool,
) -> BTreeMap<String, V> {
    record
        .iter()
        .filter(|(k, v)| !predicate(k, v))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Transform one entry's key and value together (`evolveEntries`).
pub fn evolve_entries<V: Clone>(
    record: &BTreeMap<String, V>,
    key: &str,
    f: impl FnOnce(String, V) -> (String, V),
) -> BTreeMap<String, V> {
    let mut out = record.clone();
    if let Some(v) = out.remove(key) {
        let (k2, v2) = f(key.to_string(), v);
        out.insert(k2, v2);
    }
    out
}

/// Structural equality (`makeEquivalence` shape over `PartialEq` values).
pub fn equiv<V: PartialEq>(a: &BTreeMap<String, V>, b: &BTreeMap<String, V>) -> bool {
    a == b
}

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
    updates: &[Update<'_, V>],
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
