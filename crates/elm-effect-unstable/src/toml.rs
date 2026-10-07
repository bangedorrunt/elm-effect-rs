//! `encoding/Toml.ts` behavior port: tables, dotted keys, arrays, inline
//! tables, `[[arrays]]` with child tables, multiline strings, numeric
//! formats, and duplicate rejection.
//!
//! Datetimes with a time part become [`TomlValue::Datetime`] normalized
//! (`Z` gains `.000`, space separators become `T`); bare dates stay
//! strings. Duplicate keys throw; `1__000` throws.

use std::collections::BTreeMap;

/// A TOML value.
#[derive(Clone, Debug, PartialEq)]
pub enum TomlValue {
    /// UTF-8 string.
    Text(String),
    /// Integer (decimal, hex, octal, binary).
    Integer(i64),
    /// Float.
    Float(f64),
    /// Boolean.
    Bool(bool),
    /// Normalized datetime (`2026-08-05T01:02:03.000Z`).
    Datetime(String),
    /// Array of values.
    Array(Vec<TomlValue>),
    /// Table map.
    Table(BTreeMap<String, TomlValue>),
}

/// Why parsing failed. TOML errors throw like the spec.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TomlError(pub String);

impl std::fmt::Display for TomlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TOML parse error: {}", self.0)
    }
}

impl std::error::Error for TomlError {}

/// Parse one TOML document. Duplicate keys throw.
pub fn parse(input: &str) -> Result<BTreeMap<String, TomlValue>, TomlError> {
    let mut root: BTreeMap<String, TomlValue> = BTreeMap::new();
    // Current table path, or current array element (path + index).
    let mut table: Vec<String> = vec![];
    let mut element: Option<(Vec<String>, usize)> = None;
    for raw in join_continuations(input).lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with("[[") && line.ends_with("]]") {
            let path = split_path(line[2..line.len() - 2].trim())?;
            let index = push_array_element(&mut root, &path)?;
            element = Some((path, index));
            table.clear();
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let path = split_path(line[1..line.len() - 1].trim())?;
            // `[a.b]` under an active `[[a]]` element extends that element
            // when the header shares the array prefix.
            if let Some((ref array_path, index)) = element.clone() {
                if path.len() > array_path.len() && path[..array_path.len()] == array_path[..] {
                    table = path.clone();
                    // Materialize the sub-table (headers create empty tables).
                    ensure_element_subtable(
                        &mut root,
                        array_path,
                        index,
                        &path[array_path.len()..],
                    )?;
                    continue;
                }
            }
            let _ = element;
            table = path;
            element = None;
            ensure_table(&mut root, &table)?;
            continue;
        }
        let (key, raw_value) = split_key_value(line)?;
        let value = parse_value(&raw_value)?;
        if let Some((ref array_path, index)) = element.clone() {
            // Inside `[[array]]` (possibly a child table path): resolve the
            // element map, then any sub-table segments, then insert.
            let sub: Vec<String> = if table.len() > array_path.len() {
                table[array_path.len()..].to_vec()
            } else {
                vec![]
            };
            insert_at_element(&mut root, array_path.clone(), index, &sub, &key, value)?;
        } else {
            insert_at_path(&mut root, &table, &key, value)?;
        }
    }
    Ok(root)
}

/// Create tables along `sub` inside array `array_path[index]`.
fn ensure_element_subtable(
    root: &mut BTreeMap<String, TomlValue>,
    array_path: &[String],
    index: usize,
    sub: &[String],
) -> Result<(), TomlError> {
    let (parent, name) = array_path.split_at(array_path.len() - 1);
    let mut cursor = root;
    for segment in parent {
        cursor = match cursor.get_mut(segment) {
            Some(TomlValue::Table(next)) => next,
            _ => return Err(TomlError("not an array".into())),
        };
    }
    let element = match cursor.get_mut(&name[0]) {
        Some(TomlValue::Array(items)) => items
            .get_mut(index)
            .ok_or_else(|| TomlError("no such element".into()))?,
        _ => return Err(TomlError("not an array".into())),
    };
    let mut node = match element {
        TomlValue::Table(map) => map,
        _ => return Err(TomlError("not a table".into())),
    };
    for segment in sub {
        let entry = node
            .entry(segment.clone())
            .or_insert_with(|| TomlValue::Table(BTreeMap::new()));
        match entry {
            TomlValue::Table(next) => node = next,
            _ => return Err(TomlError("duplicate key".into())),
        }
    }
    Ok(())
}

fn insert_at_element(
    root: &mut BTreeMap<String, TomlValue>,
    array_path: Vec<String>,
    index: usize,
    sub: &[String],
    key: &[String],
    value: TomlValue,
) -> Result<(), TomlError> {
    let (parent, name) = array_path.split_at(array_path.len() - 1);
    let mut cursor = root;
    for segment in parent {
        cursor = match cursor.get_mut(segment) {
            Some(TomlValue::Table(next)) => next,
            _ => return Err(TomlError("not an array".into())),
        };
    }
    let element = match cursor.get_mut(&name[0]) {
        Some(TomlValue::Array(items)) => items
            .get_mut(index)
            .ok_or_else(|| TomlError("no such element".into()))?,
        _ => return Err(TomlError("not an array".into())),
    };
    let map = match element {
        TomlValue::Table(map) => map,
        _ => return Err(TomlError("not a table".into())),
    };
    let mut node = map;
    for segment in sub {
        let entry = node
            .entry(segment.clone())
            .or_insert_with(|| TomlValue::Table(BTreeMap::new()));
        match entry {
            TomlValue::Table(next) => node = next,
            _ => return Err(TomlError("duplicate key".into())),
        }
    }
    insert_dotted(node, key, value)
}

fn insert_at_path(
    root: &mut BTreeMap<String, TomlValue>,
    table: &[String],
    key: &[String],
    value: TomlValue,
) -> Result<(), TomlError> {
    let map = navigate_map(root, table)?;
    insert_dotted(map, key, value)
}

fn navigate_map<'m>(
    root: &'m mut BTreeMap<String, TomlValue>,
    path: &[String],
) -> Result<&'m mut BTreeMap<String, TomlValue>, TomlError> {
    let mut node = root;
    for segment in path {
        let entry = node
            .entry(segment.clone())
            .or_insert_with(|| TomlValue::Table(BTreeMap::new()));
        match entry {
            TomlValue::Table(next) => node = next,
            _ => return Err(TomlError("not a table".into())),
        }
    }
    Ok(node)
}

/// Merge triple-quote strings into single logical lines: the newline
/// right after an opening `"""` drops, a trailing `\` joins the next
/// line with its leading whitespace dropped, other newlines inside the
/// string are preserved.
fn join_continuations(input: &str) -> String {
    let triple = "\"\"\"";
    if input.matches(&triple).count() < 2 {
        return input.to_string();
    }
    let mut out = String::new();
    let mut current = String::new();
    let mut in_triple = false;
    let mut join_next = false;
    for line in input.split('\n') {
        let mut chunk = line.to_string();
        if join_next {
            chunk = chunk.trim_start().to_string();
            join_next = false;
        }
        if !in_triple {
            if let Some(pos) = chunk.find(&triple) {
                let after = chunk[pos + 3..].to_string();
                if after.contains(&triple) {
                    out.push_str(&chunk);
                    out.push('\n');
                } else {
                    current.push_str(&chunk);
                    in_triple = true;
                }
            } else {
                out.push_str(&chunk);
                out.push('\n');
            }
            continue;
        }
        if let Some(end) = chunk.find(&triple) {
            current.push_str(&chunk[..end + 3]);
            current.push_str(&chunk[end + 3..]);
            out.push_str(&current);
            out.push('\n');
            current = String::new();
            in_triple = false;
            continue;
        }
        let trimmed = chunk.trim_end().to_string();
        let doubled = "\\\\";
        if trimmed.ends_with('\\') && !trimmed.ends_with(&doubled) {
            current.push_str(&trimmed[..trimmed.len() - 1]);
            join_next = true;
        } else {
            current.push_str(&chunk);
            current.push('\n');
        }
    }
    out.push_str(&current);
    out
}
fn split_path(path: &str) -> Result<Vec<String>, TomlError> {
    if path.is_empty() {
        return Err(TomlError("empty path".into()));
    }
    Ok(path.split('.').map(|s| s.trim().to_string()).collect())
}

/// Append a fresh table element to the array at `path`, creating parent
/// tables. Returns the new element index.
fn push_array_element(
    root: &mut BTreeMap<String, TomlValue>,
    path: &[String],
) -> Result<usize, TomlError> {
    let (parent, name) = path.split_at(path.len() - 1);
    let mut node = root;
    for segment in parent {
        let entry = node
            .entry(segment.clone())
            .or_insert_with(|| TomlValue::Table(BTreeMap::new()));
        match entry {
            TomlValue::Table(next) => node = next,
            _ => return Err(TomlError("not a table".into())),
        }
    }
    match node.get_mut(&name[0]) {
        Some(TomlValue::Array(items)) => {
            items.push(TomlValue::Table(BTreeMap::new()));
            Ok(items.len() - 1)
        }
        None => {
            node.insert(
                name[0].clone(),
                TomlValue::Array(vec![TomlValue::Table(BTreeMap::new())]),
            );
            Ok(0)
        }
        _ => Err(TomlError("not an array".into())),
    }
}

fn ensure_table(root: &mut BTreeMap<String, TomlValue>, path: &[String]) -> Result<(), TomlError> {
    let mut node = root;
    for segment in path {
        let entry = node
            .entry(segment.clone())
            .or_insert_with(|| TomlValue::Table(BTreeMap::new()));
        match entry {
            TomlValue::Table(next) => node = next,
            _ => return Err(TomlError("duplicate key".into())),
        }
    }
    Ok(())
}

fn split_key_value(line: &str) -> Result<(Vec<String>, String), TomlError> {
    let equals = line
        .find('=')
        .ok_or_else(|| TomlError("expected key = value".into()))?;
    let key = split_path(line[..equals].trim())?;
    let value = line[equals + 1..].trim().to_string();
    if value.is_empty() {
        return Err(TomlError("expected key = value".into()));
    }
    Ok((key, value))
}

fn insert_dotted(
    map: &mut BTreeMap<String, TomlValue>,
    key: &[String],
    value: TomlValue,
) -> Result<(), TomlError> {
    let (prefix, last) = key.split_at(key.len() - 1);
    let mut node = map;
    for segment in prefix {
        let entry = node
            .entry(segment.clone())
            .or_insert_with(|| TomlValue::Table(BTreeMap::new()));
        match entry {
            TomlValue::Table(next) => node = next,
            _ => return Err(TomlError("duplicate key".into())),
        }
    }
    if node.contains_key(&last[0]) {
        return Err(TomlError("duplicate key".into()));
    }
    node.insert(last[0].clone(), value);
    Ok(())
}

/// Parse one scalar/composite value.
fn parse_value(raw: &str) -> Result<TomlValue, TomlError> {
    let text = raw.trim();
    if text.starts_with('"') {
        return parse_basic_string(text);
    }
    if text.starts_with('\'') {
        return parse_literal_string(text);
    }
    if text.starts_with('[') {
        return parse_array(text);
    }
    if text.starts_with('{') {
        return parse_inline_table(text);
    }
    if text == "true" {
        return Ok(TomlValue::Bool(true));
    }
    if text == "false" {
        return Ok(TomlValue::Bool(false));
    }
    if let Some(number) = parse_number(text) {
        return Ok(number);
    }
    if let Some(datetime) = parse_datetime(text) {
        return Ok(datetime);
    }
    Err(TomlError(format!("invalid value: {text}")))
}

fn parse_basic_string(text: &str) -> Result<TomlValue, TomlError> {
    if let Some(inner) = text.strip_prefix("\"\"\"") {
        let body = inner
            .strip_suffix("\"\"\"")
            .ok_or_else(|| TomlError("unterminated string".into()))?;
        let body = body.strip_prefix('\n').unwrap_or(body);
        return Ok(TomlValue::Text(unescape(body)));
    }
    let inner = text
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or_else(|| TomlError("unterminated string".into()))?;
    Ok(TomlValue::Text(unescape(inner)))
}

fn parse_literal_string(text: &str) -> Result<TomlValue, TomlError> {
    let inner = text
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .ok_or_else(|| TomlError("unterminated string".into()))?;
    Ok(TomlValue::Text(inner.to_string()))
}

fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn parse_array(text: &str) -> Result<TomlValue, TomlError> {
    let inner = text
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or_else(|| TomlError("unterminated array".into()))?;
    if inner.trim().is_empty() {
        return Ok(TomlValue::Array(vec![]));
    }
    inner
        .split(',')
        .map(|item| parse_value(item.trim()))
        .collect::<Result<Vec<_>, _>>()
        .map(TomlValue::Array)
}

fn parse_inline_table(text: &str) -> Result<TomlValue, TomlError> {
    let inner = text
        .strip_prefix('{')
        .and_then(|s| s.strip_suffix('}'))
        .ok_or_else(|| TomlError("unterminated table".into()))?;
    let mut map = BTreeMap::new();
    if inner.trim().is_empty() {
        return Ok(TomlValue::Table(map));
    }
    for pair in inner.split(',') {
        let equals = pair
            .find('=')
            .ok_or_else(|| TomlError("expected key = value".into()))?;
        let key = pair[..equals].trim().to_string();
        let value = parse_value(pair[equals + 1..].trim())?;
        if map.contains_key(&key) {
            return Err(TomlError("duplicate key".into()));
        }
        map.insert(key, value);
    }
    Ok(TomlValue::Table(map))
}

fn parse_number(text: &str) -> Option<TomlValue> {
    if text.contains("__") || text.starts_with('_') || text.ends_with('_') {
        return None;
    }
    let digits = text.replace('_', "");
    if digits.is_empty() {
        return None;
    }
    let lower = digits.to_lowercase();
    if let Some(hex) = lower.strip_prefix("0x") {
        return i64::from_str_radix(hex, 16).ok().map(TomlValue::Integer);
    }
    if let Some(oct) = lower.strip_prefix("0o") {
        return i64::from_str_radix(oct, 8).ok().map(TomlValue::Integer);
    }
    if let Some(bin) = lower.strip_prefix("0b") {
        return i64::from_str_radix(bin, 2).ok().map(TomlValue::Integer);
    }
    if digits.contains(['.', 'e', 'E']) {
        return digits
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .map(TomlValue::Float);
    }
    let unsigned = digits.strip_prefix(['+', '-']).unwrap_or(&digits);
    if unsigned.is_empty() || !unsigned.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if unsigned.len() > 1 && unsigned.starts_with('0') {
        return None;
    }
    digits.parse::<i64>().ok().map(TomlValue::Integer)
}

/// Datetimes with a time part become normalized `Datetime`; bare dates
/// stay strings (callers keep them as text, like the spec).
fn parse_datetime(text: &str) -> Option<TomlValue> {
    if let Some(t) = text.find('T') {
        let (date, time) = (text[..t].to_string(), text[t + 1..].to_string());
        if !valid_date(&date) {
            return None;
        }
        let zone;
        let clock;
        if let Some(stripped) = time.strip_suffix(['Z', 'z']) {
            zone = "Z".to_string();
            clock = stripped.split('.').next().unwrap_or(stripped).to_string();
        } else {
            return None;
        }
        if !valid_clock(&clock) {
            return None;
        }
        return Some(TomlValue::Datetime(format!("{date}T{clock}.000{zone}")));
    }
    if text.len() > 10 && text.as_bytes().get(10) == Some(&b' ') {
        let date = &text[..10];
        let clock = text[11..].split('.').next().unwrap_or(&text[11..]);
        if valid_date(date) && valid_clock(clock) {
            return Some(TomlValue::Datetime(format!("{date}T{clock}")));
        }
    }
    None
}

fn valid_date(date: &str) -> bool {
    let parts: Vec<&str> = date.split('-').collect();
    parts.len() == 3
        && parts[0].len() == 4
        && parts[1].len() == 2
        && parts[2].len() == 2
        && parts.iter().all(|p| p.bytes().all(|b| b.is_ascii_digit()))
}

fn valid_clock(clock: &str) -> bool {
    let parts: Vec<&str> = clock.split(':').collect();
    (parts.len() == 2 || parts.len() == 3)
        && parts
            .iter()
            .all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_digit()))
}
