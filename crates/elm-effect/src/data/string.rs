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

/// First `n` characters (`takeLeft`).
pub fn take_left(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Last `n` characters (`takeRight`).
pub fn take_right(s: &str, n: usize) -> String {
    s.chars()
        .rev()
        .take(n)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
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

/// Capitalize the first character.
pub fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

/// Pad on the left to `width` with `pad` (`padStart`).
pub fn pad_start(s: &str, width: usize, pad: char) -> String {
    let len = s.chars().count();
    if len >= width {
        s.to_string()
    } else {
        std::iter::repeat(pad).take(width - len).collect::<String>() + s
    }
}

/// Pad on the right to `width` with `pad` (`padEnd`).
pub fn pad_end(s: &str, width: usize, pad: char) -> String {
    let len = s.chars().count();
    if len >= width {
        s.to_string()
    } else {
        s.to_string() + &std::iter::repeat(pad).take(width - len).collect::<String>()
    }
}

/// Truncate with an ellipsis (`ellipsis`): fits `max` chars total.
pub fn ellipsis(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else if max <= 1 {
        "…".to_string()
    } else {
        take_left(s, max - 1) + "…"
    }
}
