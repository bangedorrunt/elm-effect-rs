//! `Record.ts` port over `BTreeMap`: string-keyed dictionaries with sorted
//! keys. Residuals: `union` conflict resolvers beyond first-wins, deep
//! `get` paths (one level here; nested paths compose manually).

use std::collections::BTreeMap;

/// A sorted string-keyed dictionary.
pub type Record<V> = BTreeMap<String, V>;

/// Empty record (`empty`, `make` with no entries).
pub fn empty<V>() -> Record<V> {
    BTreeMap::new()
}

/// From pairs (`fromEntries`, `fromIterable`).
pub fn from_entries<V>(entries: impl IntoIterator<Item = (String, V)>) -> Record<V> {
    entries.into_iter().collect()
}

/// Lookup (`get`).
pub fn get<V: Clone>(record: &Record<V>, key: &str) -> Option<V> {
    record.get(key).cloned()
}

/// Insert or replace (`set`).
pub fn set<V>(record: &mut Record<V>, key: String, value: V) {
    record.insert(key, value);
}

/// Remove, returning the old value (`remove`, `pop`).
pub fn remove<V>(record: &mut Record<V>, key: &str) -> Option<V> {
    record.remove(key)
}

/// Key presence (`has`).
pub fn has<V>(record: &Record<V>, key: &str) -> bool {
    record.contains_key(key)
}

/// True when empty (`isEmptyRecord`, `isEmptyReadonlyRecord`).
pub fn is_empty<V>(record: &Record<V>) -> bool {
    record.is_empty()
}

/// Subset check (`isSubrecord`).
pub fn is_subrecord<V: Clone + PartialEq>(record: &Record<V>, other: &Record<V>) -> bool {
    record
        .iter()
        .all(|(k, v)| other.get(k).is_some_and(|o| o == v))
}

/// Entry count (`size`).
pub fn size<V>(record: &Record<V>) -> usize {
    record.len()
}

/// Sorted keys (`keys`).
pub fn keys<V>(record: &Record<V>) -> Vec<String> {
    record.keys().cloned().collect()
}

/// Values in key order (`values`).
pub fn values<V: Clone>(record: &Record<V>) -> Vec<V> {
    record.values().cloned().collect()
}

/// Entries in key order (`toEntries`).
pub fn entries<V: Clone>(record: &Record<V>) -> Vec<(String, V)> {
    record.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// Map keys, keeping values (`mapKeys`).
pub fn map_keys<V: Clone>(record: &Record<V>, mut f: impl FnMut(&str) -> String) -> Record<V> {
    record.iter().map(|(k, v)| (f(k), v.clone())).collect()
}

/// Map entries to pairs (`mapEntries`).
pub fn map_entries<V: Clone, K2: Ord, U>(
    record: &Record<V>,
    mut f: impl FnMut(&str, &V) -> (K2, U),
) -> BTreeMap<K2, U> {
    record.iter().map(|(k, v)| f(k, v)).collect()
}

/// Update one key (`modify`).
pub fn modify<V: Clone>(record: &Record<V>, key: &str, f: impl FnOnce(V) -> V) -> Record<V> {
    let mut out = record.clone();
    if let Some(current) = out.remove(key) {
        out.insert(key.to_string(), f(current));
    }
    out
}

/// True when any value passes (`some`).
pub fn some<V>(record: &Record<V>, mut predicate: impl FnMut(&V) -> bool) -> bool {
    record.values().any(|v| predicate(v))
}

/// True when every value passes (`every`).
pub fn every<V>(record: &Record<V>, mut predicate: impl FnMut(&V) -> bool) -> bool {
    record.values().all(|v| predicate(v))
}

/// Split by predicate (`partition`, `separate` shape).
pub fn partition<V: Clone>(
    record: &Record<V>,
    mut predicate: impl FnMut(&V) -> bool,
) -> (Record<V>, Record<V>) {
    let mut yes = BTreeMap::new();
    let mut no = BTreeMap::new();
    for (k, v) in record {
        if predicate(v) {
            yes.insert(k.clone(), v.clone());
        } else {
            no.insert(k.clone(), v.clone());
        }
    }
    (yes, no)
}

/// Fold values (`reduce`, `collect` shape).
pub fn reduce<V, U>(record: &Record<V>, init: U, mut f: impl FnMut(U, &V) -> U) -> U {
    record.values().fold(init, |acc, v| f(acc, v))
}

/// Keys present in both (`intersection`).
pub fn intersection<V: Clone>(left: &Record<V>, right: &Record<V>) -> Record<V> {
    left.iter()
        .filter(|(k, _)| right.contains_key(*k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Keys only in the left (`difference`).
pub fn difference<V: Clone>(left: &Record<V>, right: &Record<V>) -> Record<V> {
    left.iter()
        .filter(|(k, _)| !right.contains_key(*k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Build from an iterable with key/value projections (`fromIterableWith`,
/// `fromIterableBy`).
pub fn from_iterable_with<T, V>(
    iterable: impl IntoIterator<Item = T>,
    key: impl Fn(&T) -> String,
    value: impl Fn(&T) -> V,
) -> Record<V> {
    iterable.into_iter().map(|a| (key(&a), value(&a))).collect()
}

/// Keep defined values, unwrapping (`getSomes`).
pub fn get_somes<V: Clone>(record: &Record<Option<V>>) -> Record<V> {
    record
        .iter()
        .filter_map(|(k, v)| v.clone().map(|v| (k.clone(), v)))
        .collect()
}
/// Map values, keeping keys (`map`).
pub fn map_values<V, U>(record: &Record<V>, mut f: impl FnMut(&V) -> U) -> Record<U> {
    record.iter().map(|(k, v)| (k.clone(), f(v))).collect()
}

/// Keep entries passing the predicate (`filter`).
pub fn filter<V: Clone>(record: &Record<V>, mut predicate: impl FnMut(&V) -> bool) -> Record<V> {
    record
        .iter()
        .filter(|(_, v)| predicate(v))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Union with first-wins on conflicts (`union`).
pub fn union<V: Clone>(left: &Record<V>, right: &Record<V>) -> Record<V> {
    let mut out = left.clone();
    for (k, v) in right {
        out.entry(k.clone()).or_insert_with(|| v.clone());
    }
    out
}
