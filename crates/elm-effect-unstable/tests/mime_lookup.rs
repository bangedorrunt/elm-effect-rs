//! MIME lookups: common types, standard types, misses, case, reverse.

use elm_effect_unstable::mime;
use std::collections::BTreeSet;

#[test]
fn looks_up_common_static_file_types() {
    for (file, expected) in [
        ("index.html", "text/html"),
        ("styles.css", "text/css"),
        ("app.js", "text/javascript"),
        ("image.png", "image/png"),
        ("document.pdf", "application/pdf"),
        ("font.woff2", "font/woff2"),
    ] {
        assert_eq!(mime::get_type(file), Some(expected), "{file}");
    }
}

#[test]
fn looks_up_standard_mime_types() {
    for (file, expected) in [
        ("feed.atom", "application/atom+xml"),
        ("archive.jar", "application/java-archive"),
        ("calendar.ics", "text/calendar"),
        ("model.glb", "model/gltf-binary"),
    ] {
        assert_eq!(mime::get_type(file), Some(expected), "{file}");
    }
}

#[test]
fn returns_none_for_unknown_and_missing_extensions() {
    assert_eq!(mime::get_type("file.unknown"), None);
    assert_eq!(mime::get_type("/directory/file"), None);
}

#[test]
fn handles_extensions_case_insensitively() {
    assert_eq!(mime::get_type("/directory/INDEX.HTML"), Some("text/html"));
    assert_eq!(mime::get_type("PHOTO.JpEg"), Some("image/jpeg"));
}

#[test]
fn exposes_reverse_lookups() {
    assert_eq!(
        mime::get_extension("text/html; charset=utf-8"),
        Some("html")
    );
    let mut expected = BTreeSet::new();
    expected.insert("html");
    expected.insert("htm");
    expected.insert("shtml");
    assert_eq!(mime::get_all_extensions("text/html"), Some(expected.clone()));
    assert_eq!(
        mime::get_all_extensions("text/html; charset=utf-8"),
        Some(expected)
    );
}
