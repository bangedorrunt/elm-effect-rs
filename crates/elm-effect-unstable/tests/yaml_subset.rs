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

#[test]
fn parses_indentless_sequences() {
    // Yaml.test.ts:114 - block sequences may sit at the key's indent.
    for (source, expected) in [
        (
            "allowed-tools:\n- Read\n- Bash\n",
            YamlValue::Map(vec![(
                "allowed-tools".to_owned(),
                YamlValue::Seq(vec![text("Read"), text("Bash")]),
            )]),
        ),
        (
            "metadata:\n  tools:\n  - Read\n",
            YamlValue::Map(vec![(
                "metadata".to_owned(),
                YamlValue::Map(vec![(
                    "tools".to_owned(),
                    YamlValue::Seq(vec![text("Read")]),
                )]),
            )]),
        ),
        (
            "tools:\n- Read\n- Bash\nroles:\n- admin\n",
            YamlValue::Map(vec![
                (
                    "tools".to_owned(),
                    YamlValue::Seq(vec![text("Read"), text("Bash")]),
                ),
                ("roles".to_owned(), YamlValue::Seq(vec![text("admin")])),
            ]),
        ),
        // Comments and blanks vanish around indentless items.
        (
            "tools: # key comment\n# before sequence\n\n- Read\n- Bash\n",
            YamlValue::Map(vec![(
                "tools".to_owned(),
                YamlValue::Seq(vec![text("Read"), text("Bash")]),
            )]),
        ),
    ] {
        assert_eq!(yaml::parse(source).expect("parses"), expected, "{source:?}");
    }
}

#[test]
fn folds_plain_multiline_scalars() {
    // Yaml.test.ts:220 - plain scalars fold across lines around comments.
    let parsed = yaml::parse(
        "description: Deploy\n  and verify. # end\n# between keys\nenabled: true\n",
    )
    .expect("parses");
    assert_eq!(
        parsed,
        yaml::YamlValue::Map(vec![
            (
                "description".to_owned(),
                yaml::YamlValue::Text("Deploy and verify.".to_owned())
            ),
            ("enabled".to_owned(), yaml::YamlValue::Bool(true)),
        ])
    );
}

#[test]
fn folds_plain_sequence_items() {
    // Yaml.test.ts:227 - sequence items fold continuations too.
    assert_eq!(
        yaml::parse("- Deploy\n  and verify.\n- Finish\n").expect("parses"),
        yaml::YamlValue::Seq(vec![
            yaml::YamlValue::Text("Deploy and verify.".to_owned()),
            yaml::YamlValue::Text("Finish".to_owned()),
        ])
    );
}

#[test]
fn parses_indentless_matrix() {
    // Yaml.test.ts:114 - the full indentless matrix.
    for (source, expected) in [
        (
            "allowed-tools:\n- Read\n- Bash\n",
            YamlValue::Map(vec![(
                "allowed-tools".to_owned(),
                YamlValue::Seq(vec![text("Read"), text("Bash")]),
            )]),
        ),
        (
            "metadata:\n  tools:\n  - Read\n",
            YamlValue::Map(vec![(
                "metadata".to_owned(),
                YamlValue::Map(vec![(
                    "tools".to_owned(),
                    YamlValue::Seq(vec![text("Read")]),
                )]),
            )]),
        ),
        (
            "tools:\n- Read\n- Bash\nroles:\n- admin\n",
            YamlValue::Map(vec![
                (
                    "tools".to_owned(),
                    YamlValue::Seq(vec![text("Read"), text("Bash")]),
                ),
                ("roles".to_owned(), YamlValue::Seq(vec![text("admin")])),
            ]),
        ),
        (
            "tools: # key comment\n# before sequence\n\n- Read # item comment\n  # between items\n\n- Bash\n",
            YamlValue::Map(vec![(
                "tools".to_owned(),
                YamlValue::Seq(vec![text("Read"), text("Bash")]),
            )]),
        ),
        (
            "steps:\n- name: deploy\n  tools:\n  - Read\n  - Bash\n  enabled: true\n- name: verify\n",
            YamlValue::Map(vec![(
                "steps".to_owned(),
                YamlValue::Seq(vec![
                    YamlValue::Map(vec![
                        ("name".to_owned(), text("deploy")),
                        (
                            "tools".to_owned(),
                            YamlValue::Seq(vec![text("Read"), text("Bash")]),
                        ),
                        ("enabled".to_owned(), YamlValue::Bool(true)),
                    ]),
                    YamlValue::Map(vec![("name".to_owned(), text("verify"))]),
                ]),
            )]),
        ),
        (
            "metadata:\n  tools:\n  - Read\n  roles:\n  - admin\n  enabled: true\nname: deploy\n",
            YamlValue::Map(vec![
                (
                    "metadata".to_owned(),
                    YamlValue::Map(vec![
                        (
                            "tools".to_owned(),
                            YamlValue::Seq(vec![text("Read")]),
                        ),
                        (
                            "roles".to_owned(),
                            YamlValue::Seq(vec![text("admin")]),
                        ),
                        ("enabled".to_owned(), YamlValue::Bool(true)),
                    ]),
                ),
                ("name".to_owned(), text("deploy")),
            ]),
        ),
        (
            "description: Deploy the service\n  and verify it.\ntools:\n- Read\nmetadata:\n  description: 'Check\n    the logs'\n  tools:\n  - Bash\n",
            YamlValue::Map(vec![
                (
                    "description".to_owned(),
                    text("Deploy the service and verify it."),
                ),
                (
                    "tools".to_owned(),
                    YamlValue::Seq(vec![text("Read")]),
                ),
                (
                    "metadata".to_owned(),
                    YamlValue::Map(vec![
                        ("description".to_owned(), text("Check the logs")),
                        (
                            "tools".to_owned(),
                            YamlValue::Seq(vec![text("Bash")]),
                        ),
                    ]),
                ),
            ]),
        ),
    ] {
        assert_eq!(yaml::parse(source).expect("parses"), expected, "{source:?}");
    }
}

#[test]
fn rejects_scalar_boundary_violations() {
    // Yaml.test.ts:233 - every boundary shape throws.
    for source in [
        "description: Use when: deploy\n",
        "description: Deploy:\n",
        "description: Use when:\tdeploy\n",
        "description: Deploy\n  when: ready\n",
        "description: Deploy # end\n  and verify.\n",
        "description: \"Deploy\" extra\n",
        "description: 'Deploy' extra\n",
        "description:\n\tDeploy\n",
        "description: first\ndescription: second\n",
    ] {
        assert!(yaml::parse(source).is_err(), "should reject {source:?}");
    }
}

#[test]
fn folds_plain_scalars_everywhere() {
    // Yaml.test.ts:163 - plain scalars fold below keys and across blanks.
    for (source, expected) in [
        (
            "description:\n  Deploy the service\n  and verify it.\n",
            "Deploy the service and verify it.",
        ),
        (
            "description: Deploy the service\n  and verify it.\n",
            "Deploy the service and verify it.",
        ),
        (
            "description:\n    Deploy\n  the service\n      and verify it.\n",
            "Deploy the service and verify it.",
        ),
        (
            "description: Deploy\n\n  and verify.\n",
            "Deploy\nand verify.",
        ),
        (
            "description: Deploy\n\n\n  and verify.\n",
            "Deploy\n\nand verify.",
        ),
        (
            "description: Deploy\n   \n  and verify.\n",
            "Deploy\nand verify.",
        ),
        (
            "# before key\ndescription: # before value\n  # before scalar\n  Deploy\n  and verify. # after scalar\n# after value\n",
            "Deploy and verify.",
        ),
    ] {
        let parsed = yaml::parse(source).expect("parses");
        let want = YamlValue::Map(vec![("description".to_owned(), text(expected))]);
        assert_eq!(parsed, want);
    }
}

#[test]
fn preserves_numeric_and_quoted_colons() {
    // Yaml.test.ts:253 - ints decode, quotes shelter colons.
    assert_eq!(
        yaml::parse("description: 123\nquoted: 'Use when: deploy'\ntrailing: \"Deploy:\"\n")
            .expect("parses"),
        YamlValue::Map(vec![
            ("description".to_owned(), YamlValue::Int(123)),
            ("quoted".to_owned(), text("Use when: deploy")),
            ("trailing".to_owned(), text("Deploy:")),
        ])
    );
}

#[test]
fn rejects_malformed_around_shapes() {
    // Yaml.test.ts:268 - malformed content throws.
    for source in [
        "broken: [one, two\n",
        "broken: {one: two\n",
        "broken: true\nunmatched\n",
        "broken:\n\tchild: true\n",
        "broken:\n   child: true\n  sibling: false\n",
        "broken: one\nbroken: two\n",
    ] {
        assert!(yaml::parse(source).is_err(), "should reject {source:?}");
    }
}

#[test]
fn rejects_nested_sequences() {
    // Yaml.test.ts:286 - valid but unsupported nesting throws.
    for source in [
        "a:\n- - 1\n  - 2\n",
        "a:\n  - - 1\n    - 2\n",
        "- - 1\n  - 2\n",
        "- - 1\n\n  - 2\n",
        "a:\n- 1\n- - 2\n",
        "a:\n- - b: 1\n",
    ] {
        assert!(yaml::parse(source).is_err(), "should reject {source:?}");
    }
}

#[test]
fn folds_dashes_after_content() {
    // Yaml.test.ts:299 - dashes fold after non-empty content.
    assert_eq!(
        yaml::parse("a: one\n  - two\n").expect("parses"),
        YamlValue::Map(vec![("a".to_owned(), text("one - two"))])
    );
    assert_eq!(
        yaml::parse("- one\n  - two\n").expect("parses"),
        YamlValue::Seq(vec![text("one - two")])
    );
}

#[test]
fn preserves_indented_markers() {
    // Yaml.test.ts:311 - indented markers stay plain values.
    for (marker, suffix) in [("---", ""), ("---", " # comment"), ("...", ""), ("...", " # comment")] {
        let source = format!("a:\n  {marker}{suffix}\n");
        assert_eq!(
            yaml::parse(&source).expect("parses"),
            YamlValue::Map(vec![("a".to_owned(), text(marker))]),
            "{source:?}"
        );
        let nested = format!("metadata:\n  a:\n    {marker}{suffix}\n");
        assert_eq!(
            yaml::parse(&nested).expect("parses"),
            YamlValue::Map(vec![(
                "metadata".to_owned(),
                YamlValue::Map(vec![("a".to_owned(), text(marker))]),
            )]),
            "{nested:?}"
        );
    }
}

#[test]
fn rejects_document_streams() {
    // Yaml.test.ts:321 - only single documents parse.
    for source in [
        "a: 1\n---\nb: 2\n",
        "one\n---\ntwo\n",
        "---\na: 1\n--- # second document\nb: 2\n",
        "---\n---\nb: 2\n",
        "a: 1\n---\n",
    ] {
        assert!(yaml::parse(source).is_err(), "should reject {source:?}");
    }
}

#[test]
fn preserves_single_document_markers() {
    // Yaml.test.ts:332 - single-document markers are skipped, not content.
    for (source, expected) in [
        (
            "# before\n--- # start\na: 1\n... # end\n# after\n",
            YamlValue::Map(vec![("a".to_owned(), YamlValue::Int(1))]),
        ),
        (
            "---\n- one\n- two\n...\n",
            YamlValue::Seq(vec![text("one"), text("two")]),
        ),
        (
            "---\none\n...\n",
            YamlValue::Text("one".to_owned()),
        ),
        (
            "---\n...\n",
            YamlValue::Null,
        ),
    ] {
        assert_eq!(yaml::parse(source).expect("parses"), expected, "{source:?}");
    }
}

#[test]
fn resolves_anchored_multiline() {
    // Yaml.test.ts:345 - anchors capture folded scalars for aliases.
    for (source, expected) in [
        (
            "description: &text Deploy\n  and verify.\nalias: *text\n",
            "Deploy and verify.",
        ),
        (
            "description: &text 'Deploy # first\n  # second' # end\nalias: *text\n",
            "Deploy # first # second",
        ),
        (
            "description: &text \"Deploy # first\n  # second\" # end\nalias: *text\n",
            "Deploy # first # second",
        ),
    ] {
        assert_eq!(
            yaml::parse(source).expect("parses"),
            YamlValue::Map(vec![
                ("description".to_owned(), text(expected)),
                ("alias".to_owned(), text(expected)),
            ]),
            "{source:?}"
        );
    }
}

#[test]
fn resolves_anchored_indentless_sequences() {
    // Yaml.test.ts:361 - anchors capture indentless sequences.
    assert_eq!(
        yaml::parse("metadata:\n  tools: &tools\n  - Read\n  - Bash\n  alias: *tools\n")
            .expect("parses"),
        YamlValue::Map(vec![(
            "metadata".to_owned(),
            YamlValue::Map(vec![
                (
                    "tools".to_owned(),
                    YamlValue::Seq(vec![text("Read"), text("Bash")]),
                ),
                (
                    "alias".to_owned(),
                    YamlValue::Seq(vec![text("Read"), text("Bash")]),
                ),
            ]),
        )])
    );
}
