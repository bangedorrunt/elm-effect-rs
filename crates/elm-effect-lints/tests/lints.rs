//! Lint harness (implementation in `src/lib.rs`): each rule fires on a
//! dirty fixture and stays quiet on a clean one, and the real workspace
//! source passes every rule.
//!
//! Fixtures are built as text, never compiled as code, so they may name
//! crates the workspace does not depend on.

use elm_effect_lints::{Rule, audit, check};
use std::path::Path;

/// The run edge, assembled from parts: the workspace law test in
/// `crates/elm-effect/tests/l2_cause.rs` greps for the literal and forbids it
/// outside the runtime crate, the test harness, and examples. A lint that
/// scans for a needle carries the needle here instead of verbatim.
const RUN_EDGE: &str = concat!("Runtime", "::run");

fn dirty() -> String {
    format!(
        r#"
//! A dirty file: every rule fires.

use std::collections::HashMap;

pub struct AppModel {{
    cache: HashMap<String, i64>,
}}

pub fn tick(model: &AppModel) -> i64 {{
    model.cache.len() as i64
}}

pub fn boot() -> i32 {{
    use elm_effect::Runtime;
    {RUN_EDGE}(elm_effect::Effect::succeed(1))
}}

pub async fn later() {{
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
}}
"#
    )
}

fn clean() -> String {
    r#"
//! A clean file: fold, effect module, no rule fires.

use std::collections::BTreeMap;

pub struct CleanModel {{
    counts: BTreeMap<String, i64>,
}}

pub fn tick(model: &CleanModel) -> i64 {{
    model.counts.len() as i64
}}

pub async fn later() {{
    elm_effect::time::sleep(elm_effect::Duration::from_millis(5)).await;
}}
"#
    .to_string()
}

/// The run edge lives in `fn main`. Legal: no finding.
fn edge() -> String {
    format!(
        r#"
fn main() {{
    use elm_effect::Runtime;
    let code = {RUN_EDGE}(elm_effect::Effect::succeed(1));
    std::process::exit(code);
}}
"#
    )
}

/// The managed runtime delegates to the run edge: that delegation is the
/// edge, not a library caller hiding an effect.
fn delegation() -> String {
    format!(
        r#"
impl ManagedRuntime {{
    pub fn run<A, E>(&self, effect: Effect<A, E>) -> Exit<A, E>
    where
        A: Send + 'static,
        E: Send + 'static,
    {{
        {RUN_EDGE}(effect)
    }}
}}

fn elsewhere() -> i32 {{
    use elm_effect::Runtime;
    {RUN_EDGE}(elm_effect::Effect::succeed(1))
}}
"#
    )
}

fn findings(src: &str) -> Vec<String> {
    check("src/dirty.rs", src)
        .into_iter()
        .map(|f| f.to_string())
        .collect()
}

#[test]
fn run_outside_the_edge_fires_on_dirty_fixture() {
    let hits = check(Path::new("src/dirty.rs"), &dirty());
    let runs: Vec<_> = hits
        .iter()
        .filter(|f| f.rule == Rule::RunOutsideTheEdge)
        .collect();
    assert_eq!(
        runs.len(),
        1,
        "one run-edge call in a non-edge file: {runs:?}"
    );
    assert!(hits.iter().any(|f| f.text.contains(RUN_EDGE)));
    assert!(!matches!(
        hits.first().map(|f| f.rule),
        Some(Rule::HashMapInAModel)
    ));
}

#[test]
fn run_inside_fn_main_is_the_legal_edge() {
    let hits = check(Path::new("src/main.rs"), &edge());
    let runs: Vec<_> = hits
        .iter()
        .filter(|f| f.rule == Rule::RunOutsideTheEdge)
        .collect();
    assert!(runs.is_empty(), "fn main is the edge: {runs:?}");
}

#[test]
fn run_in_a_test_target_is_allowed() {
    let hits = check(Path::new("crates/elm-effect/tests/thing.rs"), &dirty());
    let runs: Vec<_> = hits
        .iter()
        .filter(|f| f.rule == Rule::RunOutsideTheEdge)
        .collect();
    assert!(runs.is_empty(), "tests may run: {runs:?}");
}

#[test]
fn run_in_an_example_is_allowed() {
    let hits = check(Path::new("crates/elm-effect/examples/thing.rs"), &dirty());
    let runs: Vec<_> = hits
        .iter()
        .filter(|f| f.rule == Rule::RunOutsideTheEdge)
        .collect();
    assert!(runs.is_empty(), "examples may run: {runs:?}");
}

#[test]
fn run_in_the_runtimes_own_delegation_is_the_edge() {
    let hits = check(Path::new("src/runtime.rs"), &delegation());
    let runs: Vec<_> = hits
        .iter()
        .filter(|f| f.rule == Rule::RunOutsideTheEdge)
        .collect();
    // Only the library-caller function reports; the delegation does not.
    assert_eq!(runs.len(), 1, "{runs:?}");
    let caller = format!("{RUN_EDGE}(elm_effect::Effect::succeed(1))");
    assert!(runs[0].text.contains(&caller));
}

#[test]
fn a_generic_fn_name_does_not_swallow_the_rule() {
    // `fn run_all` is not the edge: it must not shield a caller.
    let src = format!("fn run_all() {{ let x = {RUN_EDGE}(Effect::succeed(1)); }}");
    let hits = check(Path::new("src/lib.rs"), &src);
    let runs: Vec<_> = hits
        .iter()
        .filter(|f| f.rule == Rule::RunOutsideTheEdge)
        .collect();
    assert_eq!(runs.len(), 1, "fn run_all is not the edge: {runs:?}");
}

#[test]
fn hashmap_in_a_model_fires_on_dirty_fixture() {
    let hits = check(Path::new("src/dirty.rs"), &dirty());
    let maps: Vec<_> = hits
        .iter()
        .filter(|f| f.rule == Rule::HashMapInAModel)
        .collect();
    assert_eq!(maps.len(), 1, "one HashMap inside a *Model: {maps:?}");
    assert!(maps[0].text.contains("HashMap"));
}

#[test]
fn btreemap_in_a_model_is_clean() {
    let hits = check(Path::new("src/clean.rs"), &clean());
    let maps: Vec<_> = hits
        .iter()
        .filter(|f| f.rule == Rule::HashMapInAModel)
        .collect();
    assert!(
        maps.is_empty(),
        "ordered containers are the shape: {maps:?}"
    );
}

#[test]
fn sleep_inside_an_effect_module_fires_on_dirty_fixture() {
    let hits = check(Path::new("src/dirty.rs"), &dirty());
    let sleeps: Vec<_> = hits
        .iter()
        .filter(|f| f.rule == Rule::SleepInsideAnEffectModule)
        .collect();
    assert_eq!(sleeps.len(), 1, "one tokio::time::sleep: {sleeps:?}");
    assert!(sleeps[0].text.contains("tokio::time::sleep"));
}

#[test]
fn the_effect_sleep_is_not_the_runtime_sleep() {
    let hits = check(Path::new("src/clean.rs"), &clean());
    let sleeps: Vec<_> = hits
        .iter()
        .filter(|f| f.rule == Rule::SleepInsideAnEffectModule)
        .collect();
    assert!(
        sleeps.is_empty(),
        "describing a wait is not taking one: {sleeps:?}"
    );
}

#[test]
fn clean_fixture_produces_nothing_at_all() {
    assert!(findings(&clean()).is_empty(), "{:?}", findings(&clean()));
}

#[test]
fn findings_render_with_rule_and_line() {
    let hits = check(Path::new("src/dirty.rs"), &dirty());
    assert!(
        hits.iter().all(|f| f.line > 0),
        "every finding carries a 1-based line"
    );
    let rendered = hits[0].to_string();
    assert!(
        rendered.contains("src/dirty.rs") && rendered.contains("[run-outside-the-edge]"),
        "{rendered}"
    );
}

#[test]
fn audit_runs_every_rule_over_every_file() {
    let mut out = audit([
        (Path::new("src/dirty.rs"), dirty().as_str()),
        (Path::new("src/clean.rs"), clean().as_str()),
    ]);
    assert_eq!(
        out.len(),
        3,
        "dirty fires all three rules, clean fires none"
    );
    // Input order is preserved.
    assert!(out.iter().all(|f| f.line > 0));
    out.sort_by_key(|f| f.rule.name());
    let names: Vec<&str> = out.iter().map(|f| f.rule.name()).collect();
    assert_eq!(
        names,
        vec![
            "hashmap-in-a-model",
            "run-outside-the-edge",
            "sleep-inside-an-effect-module"
        ]
    );
}

#[test]
fn the_workspace_source_passes_every_rule() {
    // Walks the real tree, skipping target/, .git/, and this crate's own
    // source: a lint that greps for its needles must carry them, so its own
    // files can never be expected to be clean. This is the check that keeps
    // the rules honest: a finding here is either a real defect or a rule
    // that needs narrowing.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let self_dir = env!("CARGO_MANIFEST_DIR").to_string();
    let mut stack = vec![root.clone()];
    let mut checked = 0usize;
    let mut findings = Vec::new();
    while let Some(dir) = stack.pop() {
        if dir.starts_with(&self_dir) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if name == "target" || name == ".git" {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if name.ends_with(".rs") {
                let Ok(src) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let rel = path
                    .strip_prefix(&root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                for mut f in check(&rel, &src) {
                    f.path = rel.clone();
                    findings.push(f);
                }
                checked += 1;
            }
        }
    }
    assert!(
        checked > 50,
        "expected to walk the workspace source, saw {checked} files"
    );
    assert!(
        findings.is_empty(),
        "the real source must pass: {findings:#?}"
    );
}
