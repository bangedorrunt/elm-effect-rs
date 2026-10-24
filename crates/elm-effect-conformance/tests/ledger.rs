use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use elm_effect_conformance::{Behavior, Ledger, load_scope, repo_root};

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

/// The ledger's own header is the row count the generator wrote. A row that
/// silently disappears (or a shifted one that parses) has to be caught here,
/// because every downstream tool trusts the file's shape.
#[test]
fn the_declared_behavior_count_matches_the_parsed_rows() {
    let root = repo_root();
    let path = root.join("conformance/behavior-tests.tsv");
    let text = fs::read_to_string(&path).unwrap();
    let declared: usize = text
        .lines()
        .find_map(|line| line.strip_prefix("# behaviors "))
        .and_then(|count| count.trim().parse().ok())
        .expect("no `# behaviors N` header");
    let rows = Behavior::load(&path).unwrap();
    assert_eq!(
        declared,
        rows.len(),
        "header declares {declared} rows, the file parses {}",
        rows.len()
    );
}

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
                matches!(module.status.as_str(), "unmatched" | "matched" | "excused"),
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
                "unported" | "ported" | "skipped-upstream" | "not-applicable"
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
        if row.status == "not-applicable" {
            assert!(
                !row.rust.is_empty(),
                "{}:{} not-applicable without a reason",
                row.path,
                row.line
            );
        }
    }
    assert!(effect_n > 3_000, "it.effect rows: {effect_n}");
    assert!(each_n > 10, "it.each rows: {each_n}");
    let unported = behaviors
        .iter()
        .filter(|row| row.status == "unported")
        .count();
    let excused = behaviors
        .iter()
        .filter(|row| row.status == "not-applicable")
        .count();
    assert!(excused > 0, "expected migrated not-applicable rows");
    assert_eq!(
        Behavior::still_open(&behaviors),
        unported,
        "still_open must count unported only"
    );
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
fn port_rows_are_matched() {
    let root = repo_root();
    let ledger = Ledger::load(&root.join("conformance/modules.toml")).unwrap();
    let behaviors = Behavior::load(&root.join("conformance/behavior-tests.tsv")).unwrap();

    // Unmatched is permanent for a module the pin ships no test for: ADR-0002
    // requires a named test before `matched`, so the old "no port row is
    // unmatched" assertion could never pass and 142 permanent rows drowned out
    // any real drift. The gate now asks two questions that can go red: a module
    // with a pin test must be matched, and one without must be on the scope
    // list that `tracker-audit.py` enforces.
    let open = ledger.port_like_owing(&behaviors);
    assert!(
        open.is_empty(),
        "{} port rows with a pin test are still unmatched: {:?}",
        open.len(),
        open.iter().map(|m| m.path.as_str()).collect::<Vec<_>>()
    );

    let scope = load_scope(&root.join("conformance/modules-without-rows.tsv")).unwrap();
    let invisible: Vec<&str> = ledger
        .port_like_untested(&behaviors)
        .into_iter()
        .map(|module| module.path.as_str())
        .filter(|path| !scope.contains(*path))
        .collect();
    assert!(
        invisible.is_empty(),
        "{} untested port modules are on no scope list: {invisible:?}",
        invisible.len()
    );

    let still = Behavior::still_open(&behaviors);
    assert_eq!(still, 0, "{still} behavior tests are still unported");
}

/// ADR-0002 §"Behavior parity": "A module row is not `matched` while any
/// behavior row from its linked test file is still `unported`."
///
/// This rule lived only in prose, so `inventory.sh` carried a hand-set
/// `matched` forward forever. 26 modules claimed parity while up to 122 of
/// their own rows were still `unported` (`JsonSchema.ts` was `matched` with
/// **zero** ported rows), and 31 more were `matched` with no linked test
/// file at all. The status is now derived in the generator; this test is what
/// makes that permanent, so a regen or a hand edit cannot reintroduce drift.
#[test]
fn matched_modules_have_no_unported_behaviors() {
    let root = repo_root();
    let ledger = Ledger::load(&root.join("conformance/modules.toml")).unwrap();
    let behaviors = Behavior::load(&root.join("conformance/behavior-tests.tsv")).unwrap();

    // (linked rows, ported, unported). The ported count is load-bearing: see
    // the `matched if ported == 0` arm below.
    let mut linked: BTreeMap<&str, (usize, usize, usize)> = BTreeMap::new();
    for row in &behaviors {
        let slot = linked.entry(row.path.as_str()).or_insert((0, 0, 0));
        slot.0 += 1;
        match row.status.as_str() {
            "ported" => slot.1 += 1,
            "unported" => slot.2 += 1,
            _ => {}
        }
    }

    let mut unearned = Vec::new();
    let mut drift = Vec::new();
    for module in &ledger.modules {
        if !matches!(
            module.class.as_str(),
            "port" | "unstable-port" | "experimental-port"
        ) {
            continue;
        }
        let mut rows = 0usize;
        let mut ported = 0usize;
        let mut unported = 0usize;
        for test in &module.tests {
            if let Some((total, done, open)) = linked.get(test.as_str()) {
                rows += total;
                ported += done;
                unported += open;
            }
        }
        match module.status.as_str() {
            // ADR-0002:565 — parity theater: types without tests. A module
            // can claim parity with nothing behind it three ways, and each
            // needs its own arm: no linked test file, linked rows that are
            // all excused as not-applicable, and linked rows with nothing
            // ported at all. The last is the one an excuse-only ledger hits.
            "matched" if rows == 0 => {
                unearned.push(format!(
                    "{}: matched with no linked behavior rows",
                    module.path
                ));
            }
            "matched" if ported == 0 => {
                unearned.push(format!(
                    "{}: matched with zero ported behavior rows ({} linked, {} excused)",
                    module.path,
                    rows,
                    rows - unported
                ));
            }
            // ADR-0002:100 — matched while a linked row is still open.
            "matched" if unported > 0 => {
                drift.push(format!(
                    "{}: matched with {unported} unported behavior rows",
                    module.path
                ));
            }
            // The other direction: every row ported but the module never
            // advanced. Deriving the status makes both impossible.
            "unmatched" if rows > 0 && unported == 0 && ported > 0 => {
                drift.push(format!(
                    "{}: unmatched with every behavior row already ported",
                    module.path
                ));
            }
            // An excuse-only module is not open work: every row it will ever
            // have is dispositioned, none of them carried. Leaving it in
            // `unmatched` made the backlog count lie, and calling it
            // `matched` would claim parity nothing backs.
            "unmatched" if rows > 0 && unported == 0 && ported == 0 => {
                drift.push(format!(
                    "{}: unmatched with every behavior row excused; use excused",
                    module.path
                ));
            }
            "excused" if unported > 0 => {
                drift.push(format!(
                    "{}: excused with {unported} unported behavior rows",
                    module.path
                ));
            }
            "excused" if ported > 0 => {
                drift.push(format!(
                    "{}: excused with {ported} ported behavior rows; use matched",
                    module.path
                ));
            }
            _ => {}
        }
    }

    assert!(
        unearned.is_empty(),
        "{} modules claim parity with no linked test:\n{}",
        unearned.len(),
        unearned.join("\n")
    );
    assert!(
        drift.is_empty(),
        "{} modules drift from ADR-0002:\n{}",
        drift.len(),
        drift.join("\n")
    );
}
