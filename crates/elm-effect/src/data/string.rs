//! `String.ts` port: `str`/`String` are the substrate.
//!
//! Length and slicing are CHAR-based here, not UTF-16 code units: astral
//! characters count once (the tag counts surrogates twice). Tests pin ASCII
//! behavior plus the documented divergence. What's here: the tag's named
//! helpers; `starts_with`/`ends_with`/`contains`/`trim` are `std` and are
//! pinned by tests directly. Out of scope: regex shapes (owned by the
//! `RegExp` bead).

/// Always true for `&str` (the type already narrows; mirrors `isString`).
pub fn is_string(_: &str) -> bool {
    true
}

/// Character count (`length` on the BMP; astral chars count once, unlike the
/// tag's UTF-16 length — documented divergence).
pub fn length(s: &str) -> usize {
    s.chars().count()
}

/// Lines without terminators (`lines`).
pub fn lines(s: &str) -> Vec<&str> {
    s.lines().collect()
}

/// Lines with their `\n` terminators kept (`linesWithSeparators`).
pub fn lines_with_separators(s: &str) -> Vec<&str> {
    s.split_inclusive('\n').collect()
}

/// Strip one common indent prefix (`stripMargin` shape: drops a leading `|`
/// per line after trimming leading whitespace).
pub fn strip_margin(s: &str) -> String {
    s.lines()
        .map(|line| {
            let trimmed = line.trim_start();
            trimmed.strip_prefix('|').unwrap_or(trimmed)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Strip a custom margin character per line (`stripMarginWith`).
pub fn strip_margin_with(s: &str, margin: char) -> String {
    s.lines()
        .map(|line| {
            let trimmed = line.trim_start();
            trimmed.strip_prefix(margin).unwrap_or(trimmed)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Convert `snake_case` to `camelCase` (`snakeToCamel`).
pub fn snake_to_camel(s: &str) -> String {
    let mut parts = s.split('_');
    let mut out = parts.next().unwrap_or("").to_owned();
    for part in parts {
        let mut chars = part.chars();
        match chars.next() {
            None => {}
            Some(first) => {
                out.extend(first.to_uppercase());
                out.push_str(chars.as_str());
            }
        }
    }
    out
}

/// First `n` characters (`takeLeft`; floored counts, empty for
/// non-positive or NaN).
pub fn take_left(s: &str, count: f64) -> String {
    if !count.is_finite() || count < 1.0 {
        return String::new();
    }
    s.chars().take(count.floor() as usize).collect()
}

/// Last `n` characters (`takeRight`; floored counts, empty for
/// non-positive or NaN).
pub fn take_right(s: &str, count: f64) -> String {
    if !count.is_finite() || count < 1.0 {
        return String::new();
    }
    let skip = s.chars().count().saturating_sub(count.floor() as usize);
    s.chars().skip(skip).collect()
}

/// Drop the first `n` characters (`dropLeft`).
pub fn drop_left(s: &str, n: usize) -> String {
    s.chars().skip(n).collect()
}

/// Drop the last `n` characters (`dropRight`).
pub fn drop_right(s: &str, n: usize) -> String {
    let keep = s.chars().count().saturating_sub(n);
    s.chars().take(keep).collect()
}

/// Char-range slice `[start, end)` (`slice` on chars, not UTF-16 units).
pub fn slice_chars(s: &str, start: usize, end: usize) -> String {
    s.chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}

/// Char-range slice with JS semantics: negative indices count from the
/// end (clamped at zero), an omitted end runs to the end (`slice`).
pub fn slice(s: &str, start: i64, end: Option<i64>) -> String {
    let len = s.chars().count() as i64;
    let clamp = |index: i64| index.clamp(0, len) as usize;
    let from = clamp(if start < 0 { len + start } else { start });
    let to = match end {
        Some(end) => clamp(if end < 0 { len + end } else { end }),
        None => len as usize,
    };
    if from >= to {
        return String::new();
    }
    s.chars().skip(from).take(to - from).collect()
}

/// Capitalize the first character.
pub fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

/// Lowercase the first character, leaving the rest unchanged (`uncapitalize`).
pub fn uncapitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
    }
}

/// Pad on the left to `width` with `pad` (`padStart`).
pub fn pad_start(s: &str, width: usize, pad: char) -> String {
    let len = s.chars().count();
    if len >= width {
        s.to_string()
    } else {
        std::iter::repeat_n(pad, width - len).collect::<String>() + s
    }
}

/// Pad on the right to `width` with `pad` (`padEnd`).
pub fn pad_end(s: &str, width: usize, pad: char) -> String {
    let len = s.chars().count();
    if len >= width {
        s.to_string()
    } else {
        s.to_string() + &std::iter::repeat_n(pad, width - len).collect::<String>()
    }
}

/// Truncate with an ellipsis (`ellipsis`): fits `max` chars total.
pub fn ellipsis(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else if max <= 1 {
        "…".to_string()
    } else {
        take_left(s, (max - 1) as f64) + "…"
    }
}

// --- Wave-6 portable remainder (regex shapes stay residual: no engine) ---

/// Split on a separator (`split`, data shapes; an empty separator
/// splits into chars the way JS does, without edge empties).
pub fn split<'a>(s: &'a str, separator: &str) -> Vec<&'a str> {
    if separator.is_empty() {
        if s.is_empty() {
            return vec![""];
        }
        let mut out = Vec::new();
        let mut chars = s.char_indices().peekable();
        while let Some((start, _)) = chars.next() {
            let end = chars.peek().map(|(index, _)| *index).unwrap_or(s.len());
            out.push(&s[start..end]);
        }
        return out;
    }
    s.split(separator).collect()
}

/// Replace all occurrences (`replaceAll`).
pub fn replace_all(s: &str, from: &str, to: &str) -> String {
    s.replace(from, to)
}

/// Replace the first occurrence (`replace`).
pub fn replace_first(s: &str, from: &str, to: &str) -> String {
    match s.find(from) {
        None => s.to_string(),
        Some(i) => {
            let mut out = String::with_capacity(s.len());
            out.push_str(&s[..i]);
            out.push_str(to);
            out.push_str(&s[i + from.len()..]);
            out
        }
    }
}

/// Uppercase (`toUpperCase`, char-wise).
pub fn to_uppercase(s: &str) -> String {
    s.to_uppercase()
}

/// Lowercase (`toLowerCase`, char-wise).
pub fn to_lowercase(s: &str) -> String {
    s.to_lowercase()
}

/// Repeat floored `count` times; non-positive and NaN give `""` (`repeat`).
pub fn repeat(s: &str, count: f64) -> String {
    if !count.is_finite() || count < 1.0 {
        return String::new();
    }
    s.repeat(count.floor() as usize)
}

/// Concatenate (`concat`).
pub fn concat(first: &str, second: &str) -> String {
    first.to_owned() + second
}

/// Total order on strings (`Order`; the `Ord` implementation).
pub fn order(first: &str, second: &str) -> std::cmp::Ordering {
    first.cmp(second)
}

/// Contains (`includes`).
pub fn contains(s: &str, needle: &str) -> bool {
    s.contains(needle)
}

/// Prefix test (`startsWith`).
pub fn starts_with(s: &str, prefix: &str) -> bool {
    s.starts_with(prefix)
}

/// Suffix test (`endsWith`).
pub fn ends_with(s: &str, suffix: &str) -> bool {
    s.ends_with(suffix)
}

/// Strip prefix, if present (`stripPrefix` shape).
pub fn strip_prefix<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    s.strip_prefix(prefix)
}

/// Strip suffix, if present (`stripSuffix` shape).
pub fn strip_suffix<'a>(s: &'a str, suffix: &str) -> Option<&'a str> {
    s.strip_suffix(suffix)
}

/// Trim both ends (`trim`).
pub fn trim(s: &str) -> &str {
    s.trim()
}

/// Trim leading whitespace (`trimStart`).
pub fn trim_start(s: &str) -> &str {
    s.trim_start()
}

/// Trim trailing whitespace (`trimEnd`).
pub fn trim_end(s: &str) -> &str {
    s.trim_end()
}

/// True for empty strings (`isEmpty`).
pub fn is_empty(s: &str) -> bool {
    s.is_empty()
}

/// True for non-empty strings (`isNonEmpty`).
pub fn is_non_empty(s: &str) -> bool {
    !s.is_empty()
}

/// Char offset of a position, clamped at the end.
fn char_offset(s: &str, position: usize) -> usize {
    s.char_indices()
        .nth(position)
        .map(|(index, _)| index)
        .unwrap_or(s.len())
}

/// `includes` searching from a char position (clamped at the end).
pub fn includes_from(s: &str, needle: &str, position: usize) -> bool {
    s[char_offset(s, position)..].contains(needle)
}

/// `startsWith` checking at a char position (clamped at the end).
pub fn starts_with_at(s: &str, prefix: &str, position: usize) -> bool {
    s[char_offset(s, position)..].starts_with(prefix)
}

/// `endsWith` checking within the first `len` chars (clamped).
pub fn ends_with_at(s: &str, suffix: &str, len: usize) -> bool {
    s[..char_offset(s, len)].ends_with(suffix)
}

/// Code point at a char index, if any (`charCodeAt`; astral planes
/// differ from UTF-16 surrogates by design — chars throughout).
pub fn char_code_at(s: &str, index: usize) -> Option<u32> {
    s.chars().nth(index).map(|c| c as u32)
}

/// Character at a char index, if any (`at`).
pub fn char_at(s: &str, index: usize) -> Option<char> {
    s.chars().nth(index)
}

/// Unicode scalar at a char index, if any (`codePointAt`; full points,
/// never lone surrogates).
pub fn code_point_at(s: &str, index: usize) -> Option<u32> {
    s.chars().nth(index).map(|c| c as u32)
}

/// UTF-16 unit offset of the first occurrence, if any (`indexOf`;
/// units match TS exactly, unlike the char-based slicers).
pub fn index_of(s: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    let haystack: Vec<u16> = s.encode_utf16().collect();
    let needle: Vec<u16> = needle.encode_utf16().collect();
    haystack
        .windows(needle.len())
        .position(|window| window == needle.as_slice())
}

/// UTF-16 unit offset of the last occurrence, if any (`lastIndexOf`).
pub fn last_index_of(s: &str, needle: &str) -> Option<usize> {
    let haystack: Vec<u16> = s.encode_utf16().collect();
    if needle.is_empty() {
        return Some(haystack.len());
    }
    let needle: Vec<u16> = needle.encode_utf16().collect();
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle.as_slice())
}

/// Char-range slice with JS `substring` semantics: negatives count as
/// zero, inverted ends swap, an omitted end runs out.
pub fn substring(s: &str, start: i64, end: Option<i64>) -> String {
    let len = s.chars().count() as i64;
    let clamp = |index: i64| index.clamp(0, len) as usize;
    let (mut from, mut to) = match end {
        Some(end) => (clamp(start), clamp(end)),
        None => (clamp(start), len as usize),
    };
    if from > to {
        std::mem::swap(&mut from, &mut to);
    }
    s.chars().skip(from).take(to - from).collect()
}

/// First character, if any (`charAt(0)` shape).
pub fn head(s: &str) -> Option<char> {
    s.chars().next()
}

/// Last character, if any.
pub fn last(s: &str) -> Option<char> {
    s.chars().last()
}
