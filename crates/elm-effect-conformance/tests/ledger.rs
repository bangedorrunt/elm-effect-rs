use std::collections::BTreeSet;
use std::fs;

use elm_effect_conformance::{Behavior, Ledger, repo_root};

const CLASSES: &[&str] = &[
    "port",
    "unstable-port",
    "experimental-port",
    "native",
    "platform-foreign",
];

const NATIVE: &[&str] = &[
    "Effectable",
    "HKT",
    "Inspectable",
    "Newtype",
    "Pipeable",
    "Symbol",
    "Types",
    "Unify",
];

#[test]
fn ledger_is_well_formed() {
    let root = repo_root();
    let pin = fs::read_to_string(root.join("conformance/PIN")).unwrap();
    assert_eq!(pin, "effect@4.0.0\n");

    let native_file = fs::read_to_string(root.join("conformance/native.txt")).unwrap();
    let native_stems: Vec<&str> = native_file
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    assert_eq!(native_stems, NATIVE);

    let ledger = Ledger::load(&root.join("conformance/modules.toml")).unwrap();
    assert!(
        ledger.modules.len() > 300,
        "expected the effect package surface, got {} rows",
        ledger.modules.len()
    );

    let mut seen = BTreeSet::new();
    for module in &ledger.modules {
        assert!(
            seen.insert(module.path.clone()),
            "duplicate path {}",
            module.path
        );
        assert!(
            CLASSES.contains(&module.class.as_str()),
            "{} has class {}",
            module.path,
            module.class
        );
        match module.class.as_str() {
            "native" | "platform-foreign" => {
                assert_eq!(module.status, "classified", "{}", module.path);
                assert!(!module.reason.is_empty(), "{}", module.path);
            }
            _ => assert!(
                module.status == "unmatched" || module.status == "matched",
                "{}",
                module.path
            ),
        }
        let stem = module
            .path
            .rsplit('/')
            .next()
            .unwrap()
            .trim_end_matches(".ts");
        if NATIVE.contains(&stem) && module.tag == "none" {
            assert_eq!(module.class, "native", "{}", module.path);
        }
        if stem == "Arbitrary" {
            assert_eq!(module.class, "unstable-port");
            assert_eq!(module.tag, "unstable");
        }
        if module.path.ends_with("testing/TestClock.ts") {
            assert_eq!(module.class, "port");
            assert_eq!(module.tag, "none");
        }
        if module.path.ends_with("ai/Prompt.ts") {
            assert!(
                module.tests.iter().all(|test| test.contains("/ai/")),
                "ai Prompt linked {}",
                module.tests.join(", ")
            );
        }
        if module.path.ends_with("cli/Prompt.ts") {
            assert!(
                module.tests.iter().all(|test| test.contains("/cli/")),
                "cli Prompt linked {}",
                module.tests.join(", ")
            );
        }
    }

    let behaviors = Behavior::load(&root.join("conformance/behavior-tests.tsv")).unwrap();
    assert!(
        behaviors.len() > 9_000,
        "effect v4 behavior tests, got {}",
        behaviors.len()
    );
    let mut seen = BTreeSet::new();
    let mut effect_n = 0;
    let mut each_n = 0;
    for row in &behaviors {
        assert!(
            seen.insert((row.path.clone(), row.line)),
            "duplicate {}:{}",
            row.path,
            row.line
        );
        assert!(
            matches!(
                row.status.as_str(),
                "unported" | "ported" | "skipped-upstream"
            ),
            "{}:{} status {}",
            row.path,
            row.line,
            row.status
        );
        if row.kind == "it.effect" {
            effect_n += 1;
        }
        if row.kind == "it.each" {
            each_n += 1;
        }
        if row.kind == "it.skip" {
            assert_eq!(row.status, "skipped-upstream");
        }
        if row.status == "ported" {
            assert!(
                !row.rust.is_empty(),
                "{}:{} ported without a rust test",
                row.path,
                row.line
            );
        }
    }
    assert!(effect_n > 3_000, "it.effect rows: {effect_n}");
    assert!(each_n > 10, "it.each rows: {each_n}");
    assert!(behaviors.iter().any(|row| {
        row.path.ends_with("sql/SqlSchema.test.ts")
            && row.title == "returns decoded results"
            && row.kind == "it.effect"
    }));
    assert!(behaviors.iter().any(|row| {
        row.title == "sets the exact outgoing Content-Length for $name" && row.kind == "it.each"
    }));
}

#[test]
#[ignore = "phase 13: every port-like module is matched and every behavior test is ported"]
fn port_rows_are_matched() {
    let root = repo_root();
    let ledger = Ledger::load(&root.join("conformance/modules.toml")).unwrap();
    let open = ledger.port_like_unmatched();
    assert!(
        open.is_empty(),
        "{} port rows are still unmatched",
        open.len()
    );
    let behaviors = Behavior::load(&root.join("conformance/behavior-tests.tsv")).unwrap();
    let still = Behavior::still_open(&behaviors);
    assert_eq!(still, 0, "{still} behavior tests are still unported");
}
