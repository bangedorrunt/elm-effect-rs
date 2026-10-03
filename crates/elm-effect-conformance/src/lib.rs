//! Reads `conformance/modules.toml`. The default tests check the ledger is
//! well-formed. The ignored test is the phase-13 gate: every port row matched.

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
            })
            .collect()
    }
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
        let mut rows = Vec::new();
        for (number, raw) in text.lines().enumerate() {
            if raw.is_empty() || raw.starts_with('#') || raw.starts_with("status\t") {
                continue;
            }
            let mut fields = raw.splitn(6, '\t');
            let status = fields
                .next()
                .ok_or_else(|| format!("line {}: status", number + 1))?
                .to_string();
            let path = fields
                .next()
                .ok_or_else(|| format!("line {}: path", number + 1))?
                .to_string();
            let line: u32 = fields
                .next()
                .ok_or_else(|| format!("line {}: line", number + 1))?
                .parse()
                .map_err(|_| format!("line {}: line is not a number", number + 1))?;
            let kind = fields
                .next()
                .ok_or_else(|| format!("line {}: kind", number + 1))?
                .to_string();
            let title = fields
                .next()
                .ok_or_else(|| format!("line {}: title", number + 1))?
                .to_string();
            let rust = fields.next().unwrap_or("").to_string();
            rows.push(Self {
                status,
                path,
                line,
                kind,
                title,
                rust,
            });
        }
        Ok(rows)
    }

    pub fn still_open(rows: &[Self]) -> usize {
        rows.iter().filter(|row| row.status == "unported").count()
    }
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
    use super::parse_modules;

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
