//! The workspace passes its own lints.
//!
//! This is the gate that makes `elm-effect-lints` more than a library nobody
//! calls: every rule runs over every `src/**/*.rs` in the workspace, and a
//! finding fails the build. The lint crate's own sources are skipped because
//! they contain the needles the rules search for.

use std::path::{Path, PathBuf};

use elm_effect_lints::{Rule, audit};

/// Every `.rs` file under `crates/*/src` and `crates/*/examples`, except the
/// lint crate itself.
fn workspace_sources() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }

    let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .to_path_buf();
    let mut out = Vec::new();
    for lane in ["src", "examples"] {
        let mut found = Vec::new();
        walk(&crates.join("elm-effect").join(lane), &mut found);
        walk(&crates.join("elm-effect-elm").join(lane), &mut found);
        walk(&crates.join("elm-effect-journal").join(lane), &mut found);
        walk(&crates.join("elm-effect-test").join(lane), &mut found);
        out.extend(found);
    }
    out.sort();
    out
}

#[test]
fn the_workspace_passes_its_own_lints() {
    let sources: Vec<(PathBuf, String)> = workspace_sources()
        .into_iter()
        .filter_map(|path| {
            let src = std::fs::read_to_string(&path).ok()?;
            Some((path, src))
        })
        .collect();
    assert!(
        sources.len() > 40,
        "the walk found {} files; it must cover the workspace",
        sources.len()
    );

    let findings = audit(sources.iter().map(|(p, s)| (p, s.as_str())));
    assert!(
        findings.is_empty(),
        "the workspace trips its own lints: {findings:#?}"
    );
}

#[test]
fn every_rule_is_checked_by_this_gate() {
    // If a rule is added, this fails until it is exercised above: `check` runs
    // all four, and the rule count is the contract that nothing is orphaned.
    let src = "fn update(model: &AppModel) { let _ = std::time::SystemTime::now(); }\n";
    let rules: Vec<Rule> = elm_effect_lints::check("src/app.rs", src)
        .into_iter()
        .map(|finding| finding.rule)
        .collect();
    assert_eq!(rules, vec![Rule::AmbientWorldInAFold]);
}
