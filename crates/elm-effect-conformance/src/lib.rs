//! Reads `conformance/modules.toml`. The default tests check the ledger is
//! well-formed. The ignored test is the phase-13 gate: every port row that the
//! pin gives a test is matched, and every other one is named in
//! `conformance/modules-without-rows.tsv`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub path: String,
    pub class: String,
    pub status: String,
    pub tag: String,
    pub reason: String,
    pub tests: Vec<String>,
}

#[derive(Debug)]
pub struct Ledger {
    pub modules: Vec<Module>,
}

impl Ledger {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|err| err.to_string())?;
        Ok(Self {
            modules: parse_modules(&text)?,
        })
    }

    pub fn port_like_unmatched(&self) -> Vec<&Module> {
        self.modules
            .iter()
            .filter(|module| {
                matches!(
                    module.class.as_str(),
                    "port" | "unstable-port" | "experimental-port"
                ) && module.status != "matched"
                    // `excused` owes nothing: every row is dispositioned, so
                    // counting it here is the backlog lying in the other
                    // direction.
                    && module.status != "excused"
            })
            .collect()
    }

    /// Port-class modules that can still earn a `matched` row and have not.
    ///
    /// A module the pin gives no test can never be `matched`: ADR-0002 makes
    /// parity depend on a named test, which is the rule that stops types
    /// claiming parity with nothing behind them. Counting those 142 rows in the
    /// gate made it unsatisfiable, so a real regression could not move the
    /// number. They owe nothing here; [`load_scope`] is what keeps them from
    /// going invisible instead.
    pub fn port_like_owing<'a>(&'a self, behaviors: &[Behavior]) -> Vec<&'a Module> {
        let tested: BTreeSet<&str> = behaviors.iter().map(|row| row.path.as_str()).collect();
        self.port_like_unmatched()
            .into_iter()
            .filter(|module| {
                module
                    .tests
                    .iter()
                    .any(|test| tested.contains(test.as_str()))
            })
            .collect()
    }

    /// Port-class modules the pin gives no test file, by the ledger's own link.
    pub fn port_like_untested<'a>(&'a self, behaviors: &[Behavior]) -> Vec<&'a Module> {
        let tested: BTreeSet<&str> = behaviors.iter().map(|row| row.path.as_str()).collect();
        self.modules
            .iter()
            .filter(|module| {
                matches!(
                    module.class.as_str(),
                    "port" | "unstable-port" | "experimental-port"
                ) && !module
                    .tests
                    .iter()
                    .any(|test| tested.contains(test.as_str()))
            })
            .collect()
    }
}

/// Read `conformance/modules-without-rows.tsv`: the modules the ledger cannot
/// speak for, one `path\treason` per line. `tracker-audit.py` fails when a
/// module with no rows is missing here, so the file is a complete statement of
/// what the gate does not cover, not a place to park a module quietly.
pub fn load_scope(path: &Path) -> Result<BTreeSet<String>, String> {
    let text = fs::read_to_string(path).map_err(|err| err.to_string())?;
    let mut scope = BTreeSet::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let path = line.split('\t').next().unwrap_or("").trim();
        if path.is_empty() {
            return Err(format!("scope row without a path: {line:?}"));
        }
        scope.insert(path.to_string());
    }
    Ok(scope)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Behavior {
    pub status: String,
    pub path: String,
    pub line: u32,
    pub kind: String,
    pub title: String,
    pub rust: String,
}

impl Behavior {
    pub fn load(path: &Path) -> Result<Vec<Self>, String> {
        let text = fs::read_to_string(path).map_err(|err| err.to_string())?;
        parse_behaviors(&text)
    }

    /// Rows still blocking the port: `unported` only. `ported` rows are
    /// done, `skipped-upstream` rows are the tag's own skips, and
    /// `not-applicable` rows carry a reason instead of a test.
    pub fn still_open(rows: &[Self]) -> usize {
        rows.iter().filter(|row| row.status == "unported").count()
    }
}

/// Parse the behavior ledger. A data row has exactly six columns.
///
/// The old `splitn(6, '\t')` absorbed any extra column silently: 26 rows in
/// the toIso suite carried a duplicated line column for a whole release, and
/// every tool read `title = "it"` with the reason glued onto the rust column.
/// Corruption is refused here instead, because nothing downstream can tell a
/// well-formed row from a shifted one.
pub fn parse_behaviors(text: &str) -> Result<Vec<Behavior>, String> {
    let mut rows = Vec::new();
    for (number, raw) in text.lines().enumerate() {
        if raw.is_empty() || raw.starts_with('#') || raw.starts_with("status\t") {
            continue;
        }
        let fields: Vec<&str> = raw.split('\t').collect();
        if fields.len() != 6 {
            return Err(format!(
                "line {}: expected 6 tab-separated fields, got {}",
                number + 1,
                fields.len()
            ));
        }
        let line: u32 = fields[2]
            .parse()
            .map_err(|_| format!("line {}: line is not a number", number + 1))?;
        rows.push(Behavior {
            status: fields[0].to_string(),
            path: fields[1].to_string(),
            line,
            kind: fields[3].to_string(),
            title: fields[4].to_string(),
            rust: fields[5].to_string(),
        });
    }
    Ok(rows)
}

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn parse_modules(text: &str) -> Result<Vec<Module>, String> {
    let mut modules = Vec::new();
    let mut current: Option<Module> = None;
    for (number, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[module]]" {
            if let Some(module) = current.take() {
                modules.push(module);
            }
            current = Some(Module {
                path: String::new(),
                class: String::new(),
                status: String::new(),
                tag: String::new(),
                reason: String::new(),
                tests: Vec::new(),
            });
            continue;
        }
        let module = current
            .as_mut()
            .ok_or_else(|| format!("line {}: field outside [[module]]", number + 1))?;
        let (key, value) = line
            .split_once(" = ")
            .ok_or_else(|| format!("line {}: expected key = value", number + 1))?;
        match key {
            "path" => module.path = unquote(value)?,
            "class" => module.class = unquote(value)?,
            "status" => module.status = unquote(value)?,
            "tag" => module.tag = unquote(value)?,
            "reason" => module.reason = unquote(value)?,
            "tests" => module.tests = parse_array(value)?,
            other => return Err(format!("line {}: unknown field {other}", number + 1)),
        }
    }
    if let Some(module) = current {
        modules.push(module);
    }
    Ok(modules)
}

fn unquote(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        Ok(value[1..value.len() - 1].to_string())
    } else {
        Err(format!("expected quoted string, got {value}"))
    }
}

fn parse_array(value: &str) -> Result<Vec<String>, String> {
    let value = value.trim();
    if !value.starts_with('[') || !value.ends_with(']') {
        return Err(format!("expected array, got {value}"));
    }
    let inner = value[1..value.len() - 1].trim();
    if inner.is_empty() {
        return Ok(Vec::new());
    }
    inner.split(',').map(|item| unquote(item.trim())).collect()
}

#[cfg(test)]
mod tests {
    use super::{parse_behaviors, parse_modules};

    #[test]
    fn parses_one_behavior_row() {
        let row = parse_behaviors(
            "# behaviors 1\nstatus\tpath\tline\tkind\ttitle\trust\n\
             ported\tpackages/effect/test/Array.test.ts\t35\tit\tisCanonicalArrayIndex\tsome_witness\n",
        )
        .unwrap();
        assert_eq!(row.len(), 1);
        assert_eq!(row[0].title, "isCanonicalArrayIndex");
        assert_eq!(row[0].rust, "some_witness");
    }

    /// The toIso rows that carried a duplicated line column (7 fields).
    #[test]
    fn refuses_a_row_with_an_extra_column() {
        let err = parse_behaviors(
            "not-applicable\tpackages/effect/test/schema/toIso.test.ts\t30\t30\tit\tClass\tneeds work\n",
        )
        .unwrap_err();
        assert!(
            err.contains("expected 6 tab-separated fields, got 7"),
            "{err}"
        );
    }

    #[test]
    fn refuses_a_row_with_a_missing_column() {
        let err = parse_behaviors("ported\tpackages/effect/test/Array.test.ts\t35\tit\ttitle\n")
            .unwrap_err();
        assert!(
            err.contains("expected 6 tab-separated fields, got 5"),
            "{err}"
        );
    }

    #[test]
    fn parses_one_row() {
        let text = r#"
[[module]]
path = "packages/effect/src/Effect.ts"
class = "port"
status = "unmatched"
tag = "none"
reason = ""
tests = ["packages/effect/test/Effect.test.ts"]
"#;
        let rows = parse_modules(text).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, "packages/effect/src/Effect.ts");
        assert_eq!(rows[0].tests, vec!["packages/effect/test/Effect.test.ts"]);
    }
}
