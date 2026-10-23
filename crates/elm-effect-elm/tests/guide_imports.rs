//! The guides publish Rust, and nothing compiled it. This test is the witness.
//!
//! `PUBLISHED` is the set of `use` lines `docs/guides/*.mdx` and
//! `skills/elm-effect/**/*.md` print. The module-scope imports below compile
//! that set, so a moved or renamed item is a build failure here rather than a
//! reader's error. The test half fails when a guide prints a line that is not
//! in the set, which is what makes a new snippet get compiled too.
//!
//! The motivating break: `config_provider.rs` is included by path into four
//! test binaries, and a crate-root `use crate::Defect` in it broke the one host
//! that did not re-export the name. No guide line named `Defect`, so this fence
//! would not have caught that specific failure; it catches the class that
//! matters for readers, an API the guides call after it moved.

#![allow(unused_imports)]

// Every line in `PUBLISHED`, in the same order.
use elm_effect::{effect, retry, use_service, Effect, Exit, Runtime, Schedule, Schema, ServiceTag};
use elm_effect::{apply, compose, const_unit, flow, identity, pipe, sk};
use elm_effect::{Config, ConfigError};
use elm_effect::config::Value;
use elm_effect::config_provider::Provider;
use elm_effect_elm::{Boot, Program, Update};

use std::fs;
use std::path::{Path, PathBuf};

/// The `use` lines the guides print, normalized to one space between tokens.
///
/// Adding a snippet to a guide means adding its import line here; the test
/// fails until the two agree.
const PUBLISHED: &[&str] = &[
    "use elm_effect::{Effect, Exit, Runtime};",
    "use elm_effect::{Effect, Runtime};",
    "use elm_effect::{retry, Effect, Schedule, Runtime};",
    "use elm_effect::{retry, Schedule};",
    "use elm_effect::effect;",
    "use elm_effect::{use_service, Effect, Runtime, ServiceTag};",
    "use elm_effect::Schema;",
    // The metric section's import.
    "use elm_effect::{Attributes, Counter, Metrics};",
    // The combinator section's import. `pipe` and `flow` are macros.
    "use elm_effect::{apply, compose, const_unit, flow, identity, pipe, sk};",
    // The config section's imports.
    "use elm_effect::{Config, ConfigError};",
    "use elm_effect::config::Value;",
    "use elm_effect::config_provider::Provider;",
    "use elm_effect_elm::{Boot, Program, Update};",
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every file a guide or a skill publishes: `docs/guides/*.mdx` plus
/// `skills/elm-effect/**/*.md`.
fn doc_files() -> Vec<PathBuf> {
    let root = repo_root();
    let mut files: Vec<PathBuf> = fs::read_dir(root.join("docs/guides"))
        .expect("docs/guides exists")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "mdx"))
        .collect();
    collect_markdown(&root.join("skills/elm-effect"), &mut files);
    files.sort();
    files
}

fn collect_markdown(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_markdown(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "md") {
            out.push(path);
        }
    }
}

fn published_use_lines() -> Vec<(PathBuf, String)> {
    let mut found = Vec::new();
    for file in doc_files() {
        let text = fs::read_to_string(&file).expect("a guide is readable");
        for line in text.lines() {
            let line = line.trim();
            if !line.starts_with("use elm_effect") || !line.ends_with(';') {
                continue;
            }
            found.push((file.clone(), line.to_string()));
        }
    }
    found
}

#[test]
fn the_published_set_is_what_the_module_compiles() {
    // Dedup, and normalize whitespace the way the const list is written.
    let mut printed: Vec<String> = published_use_lines()
        .into_iter()
        .map(|(_, line)| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();
    printed.sort();
    printed.dedup();

    let mut published: Vec<String> = PUBLISHED.iter().map(|line| line.to_string()).collect();
    published.sort();
    published.dedup();

    assert_eq!(
        printed, published,
        "a guide prints a use line this test does not compile, or the set carries a line no guide prints"
    );
}
