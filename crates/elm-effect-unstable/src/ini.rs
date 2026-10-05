//! `encoding/Ini.ts` behavior port: sections, arrays, scalars, quoted
//! values, `;` comments, and escaped section dots.
//!
//! Value typing follows the spec exactly: `true`/`false` become booleans,
//! `null` becomes null, everything else stays a string (`yes` is a string).
//! `tag[]=` appends; `[a\.b]` is one key `a.b`; `;` starts a comment
//! outside quotes; `#` inside quotes is retained.

use std::collections::BTreeMap;

/// An INI value: boolean, null, string, array, or section map.
#[derive(Clone, Debug, PartialEq)]
pub enum IniValue {
    /// Boolean literal.
    Bool(bool),
    /// Null literal.
    Null,
    /// Any other scalar.
    Text(String),
    /// Repeated `key[]=` entries, in order.
    Array(Vec<IniValue>),
    /// A `[section]` map.
    Section(BTreeMap<String, IniValue>),
}

/// Parse one INI document into top-level entries.
pub fn parse(input: &str) -> BTreeMap<String, IniValue> {
    let mut root: BTreeMap<String, IniValue> = BTreeMap::new();
    let mut section: Option<String> = None;
    for raw_line in input.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = Some(unescape_dots(&line[1..line.len() - 1]));
            continue;
        }
        let Some((key, value)) = split_key_value(line) else {
            continue;
        };
        insert_value(&mut root, section.clone(), &key, value);
    }
    root
}

fn insert_value(
    root: &mut BTreeMap<String, IniValue>,
    section: Option<String>,
    key: &str,
    value: IniValue,
) {
    let map = match &section {
        Some(name) => match root.get_mut(name) {
            Some(IniValue::Section(map)) => map,
            _ => {
                root.insert(name.clone(), IniValue::Section(BTreeMap::new()));
                match root.get_mut(name) {
                    Some(IniValue::Section(map)) => map,
                    _ => unreachable!(),
                }
            }
        },
        None => root,
    };
    if let Some(base) = key.strip_suffix("[]") {
        match map.entry(base.to_string()) {
            std::collections::btree_map::Entry::Occupied(mut slot) => match slot.get_mut() {
                IniValue::Array(items) => items.push(value),
                other => {
                    let first = other.clone();
                    slot.insert(IniValue::Array(vec![first, value]));
                }
            },
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(IniValue::Array(vec![value]));
            }
        }
    } else {
        map.insert(key.to_string(), value);
    }
}

/// Unescape `\.` to `.` in section headers.
fn unescape_dots(header: &str) -> String {
    let mut out = String::new();
    let mut chars = header.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Split `key = value`, stripping a trailing `;` comment outside quotes.
/// Quoted values keep everything inside the quotes verbatim.
fn split_key_value(line: &str) -> Option<(String, IniValue)> {
    let equals = line.find('=')?;
    let key = line[..equals].trim().to_string();
    let rest = line[equals + 1..].trim();
    let value = if let Some(quoted) = rest.strip_prefix('"') {
        let end = quoted.find('"')?;
        quoted[..end].to_string()
    } else {
        rest.split(';').next().unwrap_or("").trim().to_string()
    };
    Some((key, scalar(&value)))
}

/// Type one scalar: booleans and null literally, anything else a string.
fn scalar(text: &str) -> IniValue {
    match text {
        "true" => IniValue::Bool(true),
        "false" => IniValue::Bool(false),
        "null" => IniValue::Null,
        other => IniValue::Text(other.to_string()),
    }
}
