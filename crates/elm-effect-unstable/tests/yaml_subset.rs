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
