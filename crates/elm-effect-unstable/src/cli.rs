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
    /// No such command or flag (carries clap's message; clap-compat shim).
    Unknown(String),
    /// A value flag got no value (clap-compat shim).
    MissingValue(String),
    /// Help or version was requested (carries the rendered text).
    Help(String),
    /// Flag not defined by the command (`UnrecognizedOption`).
    UnrecognizedOption {
        /// Flag as typed.
        option: String,
        /// Command path from the root, when inside a subcommand.
        command: Option<Vec<String>>,
        /// Known flags with similar names.
        suggestions: Vec<String>,
    },
    /// Same flag on a parent and child command (`DuplicateOption`).
    DuplicateOption {
        /// Repeated flag.
        option: String,
        /// Parent command path.
        parent_command: Vec<String>,
        /// Child command path.
        child_command: Vec<String>,
    },
    /// A required flag is absent (`MissingOption`).
    MissingOption {
        /// Missing flag.
        option: String,
    },
    /// A required positional is absent (`MissingArgument`).
    MissingArgument {
        /// Missing positional.
        argument: String,
    },
    /// Extra positionals beyond the definition (`UnexpectedArgument`).
    UnexpectedArgument {
        /// Surplus positionals.
        arguments: Vec<String>,
    },
    /// A value failed its primitive (`InvalidValue`).
    InvalidValue {
        /// Flag or positional name.
        option: String,
        /// Rejected text.
        value: String,
        /// What the primitive wanted.
        expected: String,
        /// `"option"` or `"argument"`.
        kind: String,
    },
    /// No such subcommand (`UnknownSubcommand`).
    UnknownSubcommand {
        /// Subcommand as typed.
        subcommand: String,
        /// Parent command path, when nested.
        parent: Option<Vec<String>>,
        /// Known subcommands with similar names.
        suggestions: Vec<String>,
    },
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(what) => write!(f, "unknown: {what}"),
            Self::MissingValue(flag) => write!(f, "missing value: {flag}"),
            Self::Help(text) => write!(f, "{text}"),
            Self::UnrecognizedOption { option, .. } => {
                write!(f, "unrecognized option: {option}")
            }
            Self::DuplicateOption { option, .. } => write!(f, "duplicate option: {option}"),
            Self::MissingOption { option } => write!(f, "missing option: {option}"),
            Self::MissingArgument { argument } => write!(f, "missing argument: {argument}"),
            Self::UnexpectedArgument { arguments } => {
                write!(f, "unexpected arguments: {}", arguments.join(" "))
            }
            Self::InvalidValue { option, value, .. } => {
                write!(f, "invalid value for {option}: {value}")
            }
            Self::UnknownSubcommand { subcommand, .. } => {
                write!(f, "unknown subcommand: {subcommand}")
            }
        }
    }
}

impl CliError {
    /// Runtime tag matching the `CliError.*` class name.
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Unknown(_) => "Unknown",
            Self::MissingValue(_) => "MissingValue",
            Self::Help(_) => "Help",
            Self::UnrecognizedOption { .. } => "UnrecognizedOption",
            Self::DuplicateOption { .. } => "DuplicateOption",
            Self::MissingOption { .. } => "MissingOption",
            Self::MissingArgument { .. } => "MissingArgument",
            Self::UnexpectedArgument { .. } => "UnexpectedArgument",
            Self::InvalidValue { .. } => "InvalidValue",
            Self::UnknownSubcommand { .. } => "UnknownSubcommand",
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

/// One lexed command-line token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LexToken {
    /// `--name` or `--name=value`.
    LongOption {
        name: String,
        raw: String,
        value: Option<String>,
    },
    /// `-f` cluster element or `-f=value`.
    ShortOption {
        flag: String,
        raw: String,
        value: Option<String>,
    },
    /// Anything else, including negative numbers.
    Value { value: String },
}

/// Lexed argv: tokens plus everything after a `--` delimiter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lexed {
    pub tokens: Vec<LexToken>,
    pub trailing_operands: Vec<String>,
}

/// `true` for `-2`, `-3.70`, `-.5`, `-1e3`: optional fraction plus
/// optional exponent, fully consumed.
fn is_negative_number(arg: &str) -> bool {
    let rest = match arg.strip_prefix('-') {
        Some(rest) if !rest.is_empty() => rest,
        _ => return false,
    };
    let bytes = rest.as_bytes();
    let mut i = 0;
    let mut digits = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
        digits += 1;
    }
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
            digits += 1;
        }
        if digits == 0 {
            return false;
        }
    } else if digits == 0 {
        // No leading digits and no dot: `-`, `-e`, `-.` are not numbers.
        return false;
    }
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        i += 1;
        if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
            i += 1;
        }
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    i == bytes.len()
}

/// Split argv into tokens, mirroring the reference `lex`.
pub fn lex_argv(argv: &[&str]) -> Lexed {
    let end = argv.iter().position(|arg| *arg == "--");
    let (head, trailing_operands) = match end {
        None => (argv, Vec::new()),
        Some(index) => (
            &argv[..index],
            argv[index + 1..].iter().map(|s| s.to_string()).collect(),
        ),
    };
    let mut tokens = Vec::new();
    for arg in head {
        if !arg.starts_with('-') || is_negative_number(arg) {
            tokens.push(LexToken::Value {
                value: arg.to_string(),
            });
        } else if let Some(long) = arg.strip_prefix("--") {
            match long.find('=') {
                Some(equal) => tokens.push(LexToken::LongOption {
                    name: long[..equal].to_string(),
                    raw: arg.to_string(),
                    value: Some(long[equal + 1..].to_string()),
                }),
                None => tokens.push(LexToken::LongOption {
                    name: long.to_string(),
                    raw: arg.to_string(),
                    value: None,
                }),
            }
        } else if arg.len() > 1 {
            let flags = &arg[1..];
            match flags.find('=') {
                Some(equal) => {
                    let flag = &flags[..equal];
                    tokens.push(LexToken::ShortOption {
                        flag: flag.to_string(),
                        raw: format!("-{flag}"),
                        value: Some(flags[equal + 1..].to_string()),
                    });
                }
                None => {
                    for ch in flags.chars() {
                        tokens.push(LexToken::ShortOption {
                            flag: ch.to_string(),
                            raw: format!("-{ch}"),
                            value: None,
                        });
                    }
                }
            }
        } else {
            tokens.push(LexToken::Value {
                value: arg.to_string(),
            });
        }
    }
    Lexed {
        tokens,
        trailing_operands,
    }
}

/// Primitive CLI value parsers: the pure tables behind the option types.
pub mod primitive {
    /// Tag of the boolean primitive.
    pub const BOOLEAN_TAG: &str = "Boolean";
    /// Tag of the finite-number primitive.
    pub const FINITE_TAG: &str = "Finite";
    /// Tag of the integer primitive.
    pub const INT_TAG: &str = "Int";

    /// Parse `true/1/y/yes/on` and `false/0/n/no/off`, rejecting anything else.
    pub fn parse_bool(text: &str) -> Result<bool, String> {
        match text {
            "true" | "1" | "y" | "yes" | "on" => Ok(true),
            "false" | "0" | "n" | "no" | "off" => Ok(false),
            _ => Err("Expected \"true\" | \"yes\" | \"on\" | \"1\" | \"y\" | \"false\" | \"no\" | \"off\" | \"0\" | \"n\"".to_owned()),
        }
    }

    /// Parse a finite number, rejecting the rest with a tagged message.
    pub fn parse_finite(text: &str) -> Result<f64, String> {
        match text.parse::<f64>() {
            Ok(value) if value.is_finite() => Ok(value),
            _ => Err("Expected a string representing a finite number".to_owned()),
        }
    }

    /// Parse an integer, rejecting the rest with tagged messages.
    pub fn parse_int(text: &str) -> Result<i64, String> {
        match text.parse::<f64>() {
            Ok(value)
                if value.is_finite()
                    && value.fract() == 0.0
                    && value >= i64::MIN as f64
                    && value <= i64::MAX as f64 =>
            {
                Ok(value as i64)
            }
            Ok(_) => Err("Expected an integer".to_owned()),
            Err(_) => Err("Expected a string representing a finite number".to_owned()),
        }
    }
}

/// Cause carried by a [`UserError`]: text, an error message, or an
/// opaque value with no message to borrow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserCause {
    /// Plain-text failure detail.
    Text(String),
    /// An error's message.
    Error(String),
    /// Anything else: no message to borrow.
    Opaque,
}

/// A user handler failure with an explicit message fallback chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserError {
    /// What failed.
    pub cause: UserCause,
    /// Explicit user-facing message, when present.
    pub user_message: Option<String>,
}

impl UserError {
    /// Build from a cause, mirroring the `UserError` constructor.
    pub fn new(cause: UserCause) -> Self {
        Self {
            cause,
            user_message: None,
        }
    }

    /// Attach the explicit user-facing message.
    pub fn with_message(mut self, message: &str) -> Self {
        self.user_message = Some(message.to_owned());
        self
    }

    /// The explicit message, else the cause message, else a generic fallback.
    pub fn message(&self) -> String {
        if let Some(message) = &self.user_message {
            if !message.is_empty() {
                return message.clone();
            }
        }
        match &self.cause {
            UserCause::Text(detail) if !detail.is_empty() => detail.clone(),
            UserCause::Error(message) if !message.is_empty() => message.clone(),
            _ => "An error occurred".to_owned(),
        }
    }

    /// The runtime reports user errors before the runner renders them.
    pub fn is_error_reported(&self) -> bool {
        true
    }
}
