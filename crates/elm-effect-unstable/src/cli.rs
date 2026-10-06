//! `cli` port (`cli/Command`, `Argument`, `Flag`, `Param` at the pin).
//!
//! argv parses to a typed [`CliRequest`] before any effect runs, on clap 4
//! (builder API, behind the crate's `cli` feature): commands nest, flags
//! take values or switch booleans, positionals collect in order. Parsing
//! runs on an argument vector, never on the process, so tests never spawn
//! anything. Help text renders from the same tree.
//!
//! Residuals: shell completions, ANSI help styling, interactive prompts,
//! global-flag merging across levels, and wizard flows.

use clap::{Arg, ArgAction, ArgMatches, Command};
use std::collections::HashMap;

/// One flag definition.
#[derive(Clone, Debug)]
pub struct FlagDef {
    /// Long name (`--output`).
    pub name: &'static str,
    /// Short alias (`-o`), if any.
    pub short: Option<char>,
    /// Help text.
    pub about: &'static str,
    /// Takes a value (`--out f`) rather than switching (`--verbose`).
    pub takes_value: bool,
    /// Default when absent (value flags only).
    pub default: Option<&'static str>,
}

/// One command node.
#[derive(Clone, Debug, Default)]
pub struct CliCommand {
    /// Command name (`"serve"`).
    pub name: &'static str,
    /// Help text.
    pub about: &'static str,
    /// Flags accepted here.
    pub flags: Vec<FlagDef>,
    /// Subcommands.
    pub subcommands: Vec<CliCommand>,
}

/// Whole application: top-level flags plus commands.
#[derive(Clone, Debug, Default)]
pub struct CliApp {
    /// Binary name.
    pub name: &'static str,
    /// Version string.
    pub version: &'static str,
    /// Help text.
    pub about: &'static str,
    /// Global flags.
    pub flags: Vec<FlagDef>,
    /// Top-level commands.
    pub commands: Vec<CliCommand>,
}

/// Typed parse result: the command path plus bound values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliRequest {
    /// Command path from the root (empty for the root itself).
    pub command: Vec<String>,
    /// Bound flag values by long name (`"true"` for switches).
    pub flags: HashMap<String, String>,
    /// Positional arguments in order.
    pub positional: Vec<String>,
}

/// Parse failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CliError {
    /// No such command or flag (carries clap's message).
    Unknown(String),
    /// A value flag got no value.
    MissingValue(String),
    /// Help or version was requested (carries the rendered text).
    Help(String),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(what) => write!(f, "unknown: {what}"),
            Self::MissingValue(flag) => write!(f, "missing value: {flag}"),
            Self::Help(text) => write!(f, "{text}"),
        }
    }
}

fn flag_arg(def: &FlagDef) -> Arg {
    let mut arg = Arg::new(def.name).long(def.name).help(def.about);
    if let Some(short) = def.short {
        arg = arg.short(short);
    }
    if def.takes_value {
        arg = arg.action(ArgAction::Set);
        if let Some(default) = &def.default {
            arg = arg.default_value(*default);
        }
    } else {
        arg = arg.action(ArgAction::SetTrue);
    }
    arg
}

fn command_tree(node: &CliCommand) -> Command {
    let mut command = Command::new(node.name)
        .about(node.about)
        .arg(Arg::new("positional").num_args(0..));
    for flag in &node.flags {
        command = command.arg(flag_arg(flag));
    }
    for sub in &node.subcommands {
        command = command.subcommand(command_tree(sub));
    }
    command
}

/// Parse `argv` (including the binary name at position 0) against `app`.
/// `--help`/`-h`/`--version` surface as [`CliError::Help`] with clap's
/// rendered text instead of exiting the process.
pub fn parse(app: &CliApp, argv: &[String]) -> Result<CliRequest, CliError> {
    let mut root = Command::new(app.name).version(app.version).about(app.about);
    for flag in &app.flags {
        root = root.arg(flag_arg(flag));
    }
    for command in &app.commands {
        root = root.subcommand(command_tree(command));
    }
    let matches = root.try_get_matches_from(argv).map_err(|e| {
        use clap::error::ErrorKind::*;
        match e.kind() {
            DisplayHelp | DisplayVersion => CliError::Help(e.to_string()),
            _ => CliError::Unknown(e.to_string()),
        }
    })?;
    let mut request = CliRequest {
        command: Vec::new(),
        flags: HashMap::new(),
        positional: Vec::new(),
    };
    bind_flags(&matches, &app.flags, &mut request);
    descend(&mut request, &matches, &app.commands);
    Ok(request)
}

/// Walk the matched subcommand chain, binding each level's flags.
fn descend(request: &mut CliRequest, level: &ArgMatches, pool: &[CliCommand]) {
    if let Some((name, sub)) = level.subcommand()
        && let Some(node) = pool.iter().find(|c| c.name == name)
    {
        request.command.push(name.to_owned());
        bind_flags(sub, &node.flags, request);
        descend(request, sub, &node.subcommands);
    }
}

/// Bind one level's flags (plus trailing positionals) into the request.
fn bind_flags(matches: &ArgMatches, flags: &[FlagDef], request: &mut CliRequest) {
    if let Ok(Some(values)) = matches.try_get_many::<String>("positional") {
        request.positional.extend(values.cloned());
    }
    for flag in flags {
        if flag.takes_value {
            if let Some(value) = matches.get_one::<String>(&flag.name) {
                request.flags.insert(flag.name.to_owned(), value.clone());
            }
        } else if matches.get_flag(&flag.name) {
            request
                .flags
                .insert(flag.name.to_owned(), "true".to_owned());
        }
    }
}

/// Positional arguments are collected by callers from their own argv slice;
/// [`parse`] binds flags and the command path.
pub fn demo_app() -> CliApp {
    CliApp {
        name: "demo",
        version: "0.1.0",
        about: "demo cli",
        flags: Vec::new(),
        commands: vec![CliCommand {
            name: "serve",
            about: "serve traffic",
            flags: vec![
                FlagDef {
                    name: "port",
                    short: Some('p'),
                    about: "port",
                    takes_value: true,
                    default: Some("8080"),
                },
                FlagDef {
                    name: "verbose",
                    short: Some('v'),
                    about: "verbose",
                    takes_value: false,
                    default: None,
                },
            ],
            subcommands: Vec::new(),
        }],
    }
}

/// Move the cursor, mirroring `cursorTo`: one-indexed CSI sequences with
/// the column clamped at zero.
pub fn cursor_to(column: i64, row: Option<i64>) -> String {
    match row {
        None => format!("\x1b[{}G", (column + 1).max(0)),
        Some(row) => format!("\x1b[{};{}H", row + 1, (column + 1).max(0)),
    }
}
