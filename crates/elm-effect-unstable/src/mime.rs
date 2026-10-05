//! MIME lookup: extension to type and back (TS `Mime`).
//!
//! Test-pinned subset: exactly the entries the ported rows assert, with
//! case-insensitive extension handling. Extend from mime-db as new rows
//! demand (ponytail: hand table; reach for `mime_guess` past ~50 entries).

use std::collections::BTreeSet;

/// (extension, type) pairs. The first entry per type is the preferred one
/// returned by [`get_extension`]; keep preferred forms first.
const TABLE: &[(&str, &str)] = &[
    ("html", "text/html"),
    ("htm", "text/html"),
    ("shtml", "text/html"),
    ("css", "text/css"),
    ("js", "text/javascript"),
    ("jpeg", "image/jpeg"),
    ("jpg", "image/jpeg"),
    ("png", "image/png"),
    ("pdf", "application/pdf"),
    ("woff2", "font/woff2"),
    ("atom", "application/atom+xml"),
    ("jar", "application/java-archive"),
    ("ics", "text/calendar"),
    ("glb", "model/gltf-binary"),
];

/// The content type for `filename` by extension (after the final dot of
/// the basename), case-insensitively. Extensionless and unknown files
/// yield `None` (TS `getType`).
pub fn get_type(filename: &str) -> Option<&'static str> {
    let base = filename.rsplit('/').next().unwrap_or(filename);
    if !base.contains('.') {
        return None;
    }
    let ext = base.rsplit('.').next().unwrap_or("");
    let lower = ext.to_ascii_lowercase();
    TABLE
        .iter()
        .find(|(entry, _)| *entry == lower)
        .map(|(_, mime)| *mime)
}

/// The preferred extension for a content type, ignoring `;` parameters
/// (TS `getExtension`).
pub fn get_extension(mime: &str) -> Option<&'static str> {
    let base = mime.split(';').next()?.trim();
    TABLE
        .iter()
        .find(|(_, entry)| *entry == base)
        .map(|(ext, _)| *ext)
}

/// Every extension for a content type, ignoring `;` parameters (TS
/// `getAllExtensions`). Unknown types yield `None`.
pub fn get_all_extensions(mime: &str) -> Option<BTreeSet<&'static str>> {
    let base = mime.split(';').next()?.trim();
    let set: BTreeSet<&'static str> = TABLE
        .iter()
        .filter(|(_, entry)| *entry == base)
        .map(|(ext, _)| *ext)
        .collect();
    if set.is_empty() {
        None
    } else {
        Some(set)
    }
}
