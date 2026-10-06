//! YAML subset: nested block and flow collections.

#[path = "../src/yaml.rs"]
mod yaml;

use yaml::{YamlError, YamlValue};

fn text(value: &str) -> YamlValue {
    YamlValue::Text(value.to_string())
}

#[test]
fn parses_nested_block_and_flow_collections() {
    let document = "\nname: effect\nenabled: true\nports: [3000, 3001]\ndatabase:\n  host: localhost\n  credentials:\n    - user: root\n      roles: [admin, writer]\n    - user: guest\n      roles: []\n";
    let parsed = yaml::parse(document).unwrap();
    let expected = YamlValue::Map(vec![
        ("name".to_string(), text("effect")),
        ("enabled".to_string(), YamlValue::Bool(true)),
        (
            "ports".to_string(),
            YamlValue::Seq(vec![YamlValue::Int(3000), YamlValue::Int(3001)]),
        ),
        (
            "database".to_string(),
            YamlValue::Map(vec![
                ("host".to_string(), text("localhost")),
                (
                    "credentials".to_string(),
                    YamlValue::Seq(vec![
                        YamlValue::Map(vec![
                            ("user".to_string(), text("root")),
                            (
                                "roles".to_string(),
                                YamlValue::Seq(vec![text("admin"), text("writer")]),
                            ),
                        ]),
                        YamlValue::Map(vec![
                            ("user".to_string(), text("guest")),
                            ("roles".to_string(), YamlValue::Seq(vec![])),
                        ]),
                    ]),
                ),
            ]),
        ),
    ]);
    assert_eq!(parsed, expected);
}

#[test]
fn rejects_content_outside_the_subset() {
    assert!(matches!(
        yaml::parse("key: [unclosed"),
        Err(YamlError(_))
    ));
    assert!(yaml::parse("").is_err());
}

#[test]
fn parses_quoted_and_block_scalars() {
    use yaml::YamlValue;
    let parsed =
        yaml::parse("quoted: \"line\\nvalue\"\nliteral: |\n  first\n  second\nfolded: >-\n  first\n  second\n")
            .unwrap();
    let expected = YamlValue::Map(vec![
        (
            "quoted".to_string(),
            YamlValue::Text("line\nvalue".to_string()),
        ),
        (
            "literal".to_string(),
            YamlValue::Text("first\nsecond\n".to_string()),
        ),
        (
            "folded".to_string(),
            YamlValue::Text("first second".to_string()),
        ),
    ]);
    assert_eq!(parsed, expected);
}

#[test]
fn folds_block_scalars() {
    use std::collections::BTreeMap;
    // Yaml.test.ts:53 - folded breaks: blanks and indent shifts break lines.
    let parsed = yaml::parse(
        "paragraph: >-\n  first\n\n  second\nindented: >-\n  first\n    second\n  third\n",
    )
    .expect("parses");
    let want = YamlValue::Map(vec![
        ("paragraph".to_owned(), text("first\nsecond")),
        ("indented".to_owned(), text("first\n  second\nthird")),
    ]);
    assert_eq!(parsed, want);
}

#[test]
fn keeps_chomping_and_digit_indents() {
    // Yaml.test.ts:72 - explicit digits, keep chomping, blank runs.
    for (source, expected) in [
        (
            "message: >-2\n\n    first\n  second\n",
            "\n  first\nsecond",
        ),
        (
            "message: >+2\n  first\n    second\n",
            "first\n  second\n",
        ),
        (
            "message: >-\n  first\n\n\n  second\n",
            "first\n\nsecond",
        ),
    ] {
        let parsed = yaml::parse(source).expect("parses");
        let want = YamlValue::Map(vec![("message".to_owned(), text(expected))]);
        assert_eq!(parsed, want, "{source:?}");
    }
}

#[test]
fn resolves_anchors_and_aliases() {
    // Yaml.test.ts:92 - anchored mappings resolve through aliases.
    let parsed = yaml::parse(
        "defaults: &defaults\n  host: localhost\n  port: 5432\ndevelopment:\n  settings: *defaults\n",
    )
    .expect("parses");
    let host = ("host".to_owned(), text("localhost"));
    let port = ("port".to_owned(), YamlValue::Int(5432));
    let want = YamlValue::Map(vec![
        ("defaults".to_owned(), YamlValue::Map(vec![host.clone(), port.clone()])),
        (
            "development".to_owned(),
            YamlValue::Map(vec![(
                "settings".to_owned(),
                YamlValue::Map(vec![host, port]),
            )]),
        ),
    ]);
    assert_eq!(parsed, want);
    // Unknown aliases fail instead of resolving to null.
    assert!(yaml::parse("settings: *missing\n").is_err());
}#[test]
fn rejects_invalid_indentation() {
    // Yaml.test.ts:108 - inconsistent dedents fail instead of merging.
    let rejected = yaml::parse("root:\n   child: true\n  sibling: false\n");
    assert!(rejected.is_err(), "expected rejection, got {rejected:?}");
}