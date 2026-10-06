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

/// A routed command: path plus remaining argv.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandRoute {
    /// Command path from the root.
    pub path: Vec<String>,
    /// Argv left for the routed command.
    pub rest: Vec<String>,
}

impl CliCommand {
    /// Attach a subcommand.
    pub fn subcommand(mut self, child: CliCommand) -> Self {
        self.subcommands.push(child);
        self
    }

    /// Route argv: the first positional selects a subcommand, flags
    /// stay with the current command (subcommand routing core).
    pub fn route(&self, argv: &[String]) -> Result<CommandRoute, CliError> {
        let mut path = vec![self.name.to_owned()];
        let mut node = self;
        let mut rest = argv;
        loop {
            let Some((first, tail)) = rest.split_first() else {
                break;
            };
            if first.starts_with("--") {
                break;
            }
            match node.subcommands.iter().find(|child| child.name == *first) {
                Some(child) => {
                    path.push(child.name.to_owned());
                    node = child;
                    rest = tail;
                }
                None => {
                    return Err(CliError::UnknownSubcommand {
                        subcommand: first.clone(),
                        parent: Some(path.clone()),
                        suggestions: suggest(
                            first,
                            &node
                                .subcommands
                                .iter()
                                .map(|child| child.name)
                                .collect::<Vec<_>>(),
                        ),
                    });
                }
            }
        }
        Ok(CommandRoute {
            path,
            rest: rest.to_vec(),
        })
    }
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
    /// Tag of the string primitive.
    pub const STRING_TAG: &str = "String";
    /// Tag of the choice primitive.
    pub const CHOICE_TAG: &str = "Choice";
    /// Tag of the path primitive.
    pub const PATH_TAG: &str = "Path";
    /// Tag of the redacted primitive.
    pub const REDACTED_TAG: &str = "Redacted";
    /// Tag of the never primitive.
    pub const NEVER_TAG: &str = "Never";
    /// Tag of the date primitive.
    pub const DATE_TAG: &str = "Date";

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

    /// Parse a string (identity; every input is valid).
    pub fn parse_string(text: &str) -> String {
        text.to_owned()
    }

    /// Parse one of `options`, mapping the input to its output.
    /// Failures name the allowed inputs as `"a" | "b"`.
    pub fn parse_choice<T: Clone>(text: &str, options: &[(&str, T)]) -> Result<T, String> {
        match options.iter().find(|(input, _)| *input == text) {
            Some((_, output)) => Ok(output.clone()),
            None => Err(options
                .iter()
                .map(|(input, _)| format!("\"{input}\""))
                .collect::<Vec<_>>()
                .join(" | ")),
        }
    }

    /// What a path primitive accepts (`Primitive.Path` kinds).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum PathKind {
        /// Files or directories.
        Either,
        /// Files only.
        File,
        /// Directories only.
        Directory,
    }

    /// Parse a path: relative inputs resolve against the working
    /// directory; when `must_exist`, missing paths fail naming the
    /// resolved path. (Kind mismatch validation stays open: no row
    /// pins its message yet.)
    pub fn parse_path(text: &str, _kind: PathKind, must_exist: bool) -> Result<String, String> {
        let path = std::path::Path::new(text);
        let resolved = if path.is_absolute() {
            text.to_owned()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(path).to_string_lossy().into_owned(),
                Err(_) => text.to_owned(),
            }
        };
        if must_exist && std::fs::metadata(&resolved).is_err() {
            return Err(format!("Path does not exist: {resolved}"));
        }
        Ok(resolved)
    }

    /// A parsed secret: renders as `<redacted>`, never the bytes
    /// (`Primitive.Redacted`; mirrors the unwired core `Redacted`
    /// until it joins the library tree).
    #[derive(Clone, PartialEq, Eq)]
    pub struct RedactedString(String);

    impl RedactedString {
        /// Read the secret bytes back.
        pub fn value(&self) -> &str {
            &self.0
        }
    }

    impl std::fmt::Debug for RedactedString {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("<redacted>")
        }
    }

    impl std::fmt::Display for RedactedString {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("<redacted>")
        }
    }

    /// Parse a secret (every input is valid; the wrapper hides it).
    pub fn parse_redacted(text: &str) -> RedactedString {
        RedactedString(text.to_owned())
    }

    /// Reject every input: the always-failing sentinel.
    pub fn parse_never(_text: &str) -> Result<String, String> {
        Err("This option does not accept values".to_owned())
    }

    /// Read a file as text (`Primitive.FileText` core; OS errors
    /// surface as messages).
    pub fn parse_file_text(path: &str) -> Result<String, String> {
        std::fs::read_to_string(path).map_err(|e| e.to_string())
    }

    /// Read a file and parse its JSON (`Primitive.FileParse` core).
    pub fn parse_file_json(path: &str) -> Result<serde_json::Value, String> {
        let text = parse_file_text(path)?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }

    /// A calendar date-time in UTC with millis (`Primitive.Date` values).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct CliDate {
        /// Proleptic Gregorian year.
        pub year: i64,
        /// Month 1-12.
        pub month: u32,
        /// Day 1-31.
        pub day: u32,
        /// Hour 0-23.
        pub hour: u32,
        /// Minute 0-59.
        pub minute: u32,
        /// Second 0-59.
        pub second: u32,
        /// Millis 0-999.
        pub millis: u32,
    }

    impl CliDate {
        /// Render full ISO UTC (`toISOString`).
        pub fn to_iso_string(&self) -> String {
            format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
                self.year, self.month, self.day, self.hour, self.minute, self.second, self.millis
            )
        }
    }

    /// Days since the Unix epoch (Hinnant's civil-day algorithm).
    fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
        let year = if month <= 2 { year - 1 } else { year };
        let era = year.div_euclid(400);
        let year_of_era = year - era * 400;
        let month_prime = (month + 9) % 12;
        let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        era * 146097 + day_of_era - 719468
    }

    /// Inverse of [`days_from_civil`]: (year, month, day).
    fn civil_from_days(days: i64) -> (i64, u32, u32) {
        let shifted = days + 719468;
        let era = shifted.div_euclid(146097);
        let day_of_era = shifted - era * 146097;
        let year_of_era =
            (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
        let year = year_of_era + era * 400;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let month_prime = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
        let month = if month_prime < 10 {
            month_prime + 3
        } else {
            month_prime - 9
        };
        (
            if month <= 2 { year + 1 } else { year },
            month as u32,
            day as u32,
        )
    }

    /// Parse ISO dates, full UTC datetimes, and offset datetimes;
    /// zone-less datetimes fail (local-time semantics stay open).
    pub fn parse_date(text: &str) -> Result<CliDate, String> {
        let fail = || "Expected a valid Date".to_owned();
        let (date_text, time_text) = match text.split_once('T') {
            None => (text, None),
            Some((date, time)) => (date, Some(time)),
        };
        let mut date_parts = date_text.split('-');
        let (year, month, day) = match (date_parts.next(), date_parts.next(), date_parts.next()) {
            (Some(year), Some(month), Some(day)) if date_parts.next().is_none() => (
                year.parse::<i64>().map_err(|_| fail())?,
                month.parse::<i64>().map_err(|_| fail())?,
                day.parse::<i64>().map_err(|_| fail())?,
            ),
            _ => return Err(fail()),
        };
        if !(1..=12).contains(&month)
            || civil_from_days(days_from_civil(year, month, day))
                != (year, month as u32, day as u32)
        {
            return Err(fail());
        }
        let Some(time) = time_text else {
            return Ok(CliDate {
                year,
                month: month as u32,
                day: day as u32,
                hour: 0,
                minute: 0,
                second: 0,
                millis: 0,
            });
        };
        let mut time_parts = time.splitn(3, ':');
        let (hour_text, minute_text, second_text) =
            match (time_parts.next(), time_parts.next(), time_parts.next()) {
                (Some(hour), Some(minute), Some(second)) => (hour, minute, second),
                _ => return Err(fail()),
            };
        let (second_digits, zone) =
            match second_text.split_once(|c| c == '.' || c == 'Z' || c == '+' || c == '-') {
                None => (second_text, None),
                Some((digits, _)) => {
                    let at = second_text.len() - second_text[digits.len()..].len();
                    (digits, Some(&second_text[at..]))
                }
            };
        let (millis, zone) = match zone {
            None => (0u32, None),
            Some(rest) if rest.starts_with('.') => {
                let zone_at = rest.find(|c| c == 'Z' || c == '+' || c == '-');
                let (digits, zone) = match zone_at {
                    None => (&rest[1..], None),
                    Some(at) => (&rest[1..at], Some(&rest[at..])),
                };
                if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(fail());
                }
                let kept = &digits[..digits.len().min(3)];
                let scale = 10u32.pow(3 - kept.len() as u32);
                (kept.parse::<u32>().map_err(|_| fail())? * scale, zone)
            }
            Some(zone) => (0, Some(zone)),
        };
        let offset_minutes: i64 = match zone {
            None => return Err(fail()),
            Some("Z") => 0,
            Some(offset) => {
                let sign = if offset.starts_with('+') { 1 } else { -1 };
                let digits = offset
                    .strip_prefix('+')
                    .or_else(|| offset.strip_prefix('-'));
                let digits = match digits {
                    Some(digits) => digits,
                    None => return Err(fail()),
                };
                match digits.split_once(':') {
                    Some((hours, minutes)) if hours.len() == 2 && minutes.len() == 2 => {
                        let hours = hours.parse::<i64>().map_err(|_| fail())?;
                        let minutes = minutes.parse::<i64>().map_err(|_| fail())?;
                        if hours > 23 || minutes > 59 {
                            return Err(fail());
                        }
                        sign * (hours * 60 + minutes)
                    }
                    _ => return Err(fail()),
                }
            }
        };
        let hour = hour_text.parse::<i64>().map_err(|_| fail())?;
        let minute = minute_text.parse::<i64>().map_err(|_| fail())?;
        let second = second_digits.parse::<i64>().map_err(|_| fail())?;
        if !(0..24).contains(&hour) || !(0..60).contains(&minute) || !(0..60).contains(&second) {
            return Err(fail());
        }
        let day_millis = days_from_civil(year, month, day) * 86_400_000
            + hour * 3_600_000
            + minute * 60_000
            + second * 1_000
            + i64::from(millis)
            - offset_minutes * 60_000;
        let days = day_millis.div_euclid(86_400_000);
        let clock = day_millis.rem_euclid(86_400_000);
        let (year, month, day) = civil_from_days(days);
        Ok(CliDate {
            year,
            month,
            day,
            hour: (clock / 3_600_000) as u32,
            minute: ((clock / 60_000) % 60) as u32,
            second: ((clock / 1_000) % 60) as u32,
            millis: (clock % 1_000) as u32,
        })
    }

    /// Parse `key=value` on the first `=`; both sides must be non-empty.
    pub fn parse_key_value(text: &str) -> Result<(String, String), String> {
        match text.split_once('=') {
            None => Err(format!(
                "Invalid key=value format. Expected format: key=value, got: {text}"
            )),
            Some((key, value)) if key.is_empty() || value.is_empty() => Err(format!(
                "Invalid key=value format. Both key and value must be non-empty. Got: {text}"
            )),
            Some((key, value)) => Ok((key.to_owned(), value.to_owned())),
        }
    }
    /// A string option descriptor: long name plus whether absence fails.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct StrOption {
        /// Long name without dashes.
        pub name: String,
        /// Fail with `MissingOption` when absent.
        pub required: bool,
    }

    /// Parsed options: bound values plus accumulated recognition errors.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct ParsedOptions {
        /// Bound `name -> value` pairs in argv order.
        pub values: Vec<(String, String)>,
        /// Unknown flags in argv order (fatal only via `MissingOption`).
        pub errors: Vec<super::CliError>,
    }

    /// Parse `--name value` (`--name=value` too) against string
    /// descriptors: unknown flags accumulate, absent required flags
    /// fail. Positionals belong to the Args layer and are skipped.
    pub fn parse_options(
        descriptors: &[StrOption],
        argv: &[String],
    ) -> Result<ParsedOptions, super::CliError> {
        let mut values = Vec::new();
        let mut errors = Vec::new();
        let mut index = 0;
        while index < argv.len() {
            let arg = &argv[index];
            index += 1;
            let Some(flag) = arg.strip_prefix("--") else {
                continue;
            };
            let (name, inline) = match flag.split_once('=') {
                Some((name, value)) => (name, Some(value.to_owned())),
                None => (flag, None),
            };
            let known = descriptors.iter().any(|d| d.name == name);
            if !known {
                errors.push(super::CliError::UnrecognizedOption {
                    option: name.to_owned(),
                    command: None,
                    suggestions: Vec::new(),
                });
                continue;
            }
            let value = match inline {
                Some(value) => value,
                None => match argv.get(index).filter(|next| !next.starts_with("--")) {
                    Some(value) => {
                        index += 1;
                        value.clone()
                    }
                    None => {
                        return Err(super::CliError::InvalidValue {
                            option: name.to_owned(),
                            value: String::new(),
                            expected: "a value".to_owned(),
                            kind: "option".to_owned(),
                        });
                    }
                },
            };
            values.push((name.to_owned(), value));
        }
        match descriptors
            .iter()
            .find(|d| d.required && !values.iter().any(|(name, _)| name == &d.name))
        {
            Some(missing) => Err(super::CliError::MissingOption {
                option: missing.name.clone(),
            }),
            None => Ok(ParsedOptions { values, errors }),
        }
    }

    /// A positional spec: one fixed value or a bounded run.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum PositionalSpec {
        /// Exactly one value.
        Fixed {
            /// Positional name.
            name: String,
        },
        /// Up to `max` values (may be empty).
        Variadic {
            /// Positional name.
            name: String,
            /// Upper bound.
            max: usize,
            /// Used when no inputs remain (`withDefault`).
            default: Option<Vec<String>>,
        },
    }

    /// Parse positionals in order: bounded runs leave room for
    /// following fixed specs, leftovers fail (`UnexpectedArgument`).
    pub fn parse_positionals(
        specs: &[PositionalSpec],
        inputs: &[String],
    ) -> Result<Vec<(String, Vec<String>)>, super::CliError> {
        let mut bound = Vec::new();
        let mut rest = inputs;
        for (index, spec) in specs.iter().enumerate() {
            match spec {
                PositionalSpec::Fixed { name } => match rest.split_first() {
                    Some((first, tail)) => {
                        bound.push((name.clone(), vec![first.clone()]));
                        rest = tail;
                    }
                    None => {
                        return Err(super::CliError::MissingArgument {
                            argument: name.clone(),
                        });
                    }
                },
                PositionalSpec::Variadic { name, max, default } => {
                    if rest.is_empty() {
                        bound.push((name.clone(), default.clone().unwrap_or_default()));
                        continue;
                    }
                    let need = specs[index + 1..]
                        .iter()
                        .filter(|spec| matches!(spec, PositionalSpec::Fixed { .. }))
                        .count();
                    let take = rest.len().saturating_sub(need).min(*max);
                    bound.push((name.clone(), rest[..take].to_vec()));
                    rest = &rest[take..];
                }
            }
        }
        if rest.is_empty() {
            Ok(bound)
        } else {
            Err(super::CliError::UnexpectedArgument {
                arguments: rest.to_vec(),
            })
        }
    }
}

/// Parameter descriptors: typed `flags`/`args` readers (first brick:
/// never-sentinels only; value descriptors extend `CliParam::parse`).
pub mod param {
    /// Descriptor kind: flags read `flags`, arguments read `args`.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ParamKind {
        /// A `--flag` reader.
        Flag,
        /// A positional reader.
        Argument,
    }

    /// A parameter descriptor (mirrors `Flag.*`, `Argument.*`, `Param.*`).
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct CliParam {
        /// Which map `parse` reads.
        pub kind: ParamKind,
        /// Bound name (`"__never__"` for sentinels).
        pub name: String,
        /// Value parser for supplied inputs.
        pub primitive: ParamPrimitive,
        /// Fallback when absent (mirrors `withDefault`).
        pub default: Option<ParamValue>,
        /// Absence yields `None` instead of failing (mirrors `optional`).
        pub optional: bool,
        /// Config key consulted between supplied values and defaults
        /// (mirrors `withFallbackConfig`).
        pub fallback_config: Option<String>,
    }

    /// Bound inputs a descriptor reads: supplied maps plus config.
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct CliOptions {
        /// Bound `--flag` values by name.
        pub flags: std::collections::BTreeMap<String, Vec<String>>,
        /// Bound positional values by name.
        pub args: std::collections::BTreeMap<String, Vec<String>>,
        /// Config fallback values by key.
        pub config: std::collections::BTreeMap<String, String>,
    }

    /// Value shapes a descriptor can produce.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum ParamValue {
        /// Plain text.
        Text(String),
        /// A switch state.
        Switch(bool),
        /// An integer.
        Integer(i64),
    }

    /// Primitive parsers a descriptor can carry (first brick: never +
    /// boolean; text/choice/integer reuse `super::primitive`).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ParamPrimitive {
        /// Always fails.
        Never,
        /// Switches and explicit booleans.
        Boolean,
        /// Plain text.
        Text,
        /// Integers.
        Integer,
    }

    impl CliParam {
        /// An always-failing sentinel of `kind` (`Param.Never`).
        pub fn never(kind: ParamKind) -> Self {
            Self {
                kind,
                name: "__never__".to_owned(),
                primitive: ParamPrimitive::Never,
                default: None,
                optional: false,
                fallback_config: None,
            }
        }

        /// An always-failing flag sentinel (`Flag.Never`).
        pub fn flag_never() -> Self {
            Self::never(ParamKind::Flag)
        }

        /// An always-failing argument sentinel (`Argument.Never`).
        pub fn argument_never() -> Self {
            Self::never(ParamKind::Argument)
        }

        /// A boolean switch (`Flag.Boolean`).
        pub fn boolean(name: &str) -> Self {
            Self {
                kind: ParamKind::Flag,
                name: name.to_owned(),
                primitive: ParamPrimitive::Boolean,
                default: None,
                optional: false,
                fallback_config: None,
            }
        }

        /// A text value of `kind` (`Flag.String`, `Argument.String`).
        pub fn text(kind: ParamKind, name: &str) -> Self {
            Self {
                kind,
                name: name.to_owned(),
                primitive: ParamPrimitive::Text,
                default: None,
                optional: false,
                fallback_config: None,
            }
        }

        /// Read the config key when nothing is supplied (`withFallbackConfig`).
        pub fn with_fallback_config(mut self, key: &str) -> Self {
            self.fallback_config = Some(key.to_owned());
            self
        }

        /// An integer value of `kind` (`Flag.Int`, `Argument.Int`).
        pub fn integer(kind: ParamKind, name: &str) -> Self {
            Self {
                kind,
                name: name.to_owned(),
                primitive: ParamPrimitive::Integer,
                default: None,
                optional: false,
                fallback_config: None,
            }
        }

        /// Fall back to `value` when absent (`withDefault`).
        pub fn with_default(mut self, value: ParamValue) -> Self {
            self.default = Some(value);
            self
        }

        /// Yield `None` instead of failing when absent (`optional`).
        pub fn optional(mut self) -> Self {
            self.optional = true;
            self
        }

        /// Parse from bound maps: absent names take the default or fail
        /// as `MissingOption`; supplied values run the primitive.
        pub fn parse(
            &self,
            options: &CliOptions,
        ) -> Result<Option<(String, ParamValue)>, super::CliError> {
            let map = match self.kind {
                ParamKind::Flag => &options.flags,
                ParamKind::Argument => &options.args,
            };
            if let Some(values) = map.get(&self.name) {
                return self.parse_values(values);
            }
            if let Some(key) = &self.fallback_config {
                if let Some(value) = options.config.get(key) {
                    return self.parse_values(&[value.clone()]);
                }
            }
            match &self.default {
                Some(value) => Ok(Some((self.name.clone(), value.clone()))),
                None if self.optional => Ok(None),
                None => Err(super::CliError::MissingOption {
                    option: self.name.clone(),
                }),
            }
        }

        /// Run the primitive over bound values.
        fn parse_values(
            &self,
            values: &[String],
        ) -> Result<Option<(String, ParamValue)>, super::CliError> {
            let invalid = |expected: String| super::CliError::InvalidValue {
                option: self.name.clone(),
                value: values.join(" "),
                expected,
                kind: match self.kind {
                    ParamKind::Flag => "option".to_owned(),
                    ParamKind::Argument => "argument".to_owned(),
                },
            };
            match self.primitive {
                ParamPrimitive::Never => {
                    Err(invalid("This option does not accept values".to_owned()))
                }
                ParamPrimitive::Boolean => {
                    if values.is_empty() {
                        return Ok(Some((self.name.clone(), ParamValue::Switch(true))));
                    }
                    match super::primitive::parse_bool(&values[values.len() - 1]) {
                        Ok(flag) => Ok(Some((self.name.clone(), ParamValue::Switch(flag)))),
                        Err(expected) => Err(invalid(expected)),
                    }
                }
                ParamPrimitive::Text => match values.last() {
                    Some(value) => Ok(Some((self.name.clone(), ParamValue::Text(value.clone())))),
                    None => Err(invalid("a value".to_owned())),
                },
                ParamPrimitive::Integer => match values.last() {
                    Some(value) => match super::primitive::parse_int(value) {
                        Ok(count) => Ok(Some((self.name.clone(), ParamValue::Integer(count)))),
                        Err(expected) => Err(invalid(expected)),
                    },
                    None => Err(invalid("a value".to_owned())),
                },
            }
        }
    }

    /// Try `first`, falling back to `second` on absence (`orElse`
    /// and `orElseResult` alike; hard failures propagate unmasked).
    pub fn or_else(
        first: &CliParam,
        second: &CliParam,
        options: &CliOptions,
    ) -> Result<Option<(String, ParamValue)>, super::CliError> {
        match first.parse(options) {
            Err(
                super::CliError::MissingOption { .. } | super::CliError::MissingArgument { .. },
            ) => second.parse(options),
            result => result,
        }
    }
}

/// Levenshtein distance over chars (mirrors the TS helper; astral
/// planes count scalars here, UTF-16 units there).
pub fn levenshtein(first: &str, second: &str) -> usize {
    let left: Vec<char> = first.chars().collect();
    let right: Vec<char> = second.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    for (i, &a) in left.iter().enumerate() {
        let mut current = vec![i + 1];
        for (j, &b) in right.iter().enumerate() {
            let cost = usize::from(a != b);
            current.push(
                (previous[j] + 1)
                    .min(current[j] + 1)
                    .min(previous[j] + cost),
            );
        }
        previous = current;
    }
    previous[right.len()]
}

/// Candidates within distance 2, nearest first (mirrors `suggest`).
pub fn suggest(input: &str, candidates: &[&str]) -> Vec<String> {
    let mut scored: Vec<(usize, &str)> = candidates
        .iter()
        .map(|candidate| (levenshtein(input, candidate), *candidate))
        .filter(|(distance, _)| *distance <= 2)
        .collect();
    scored.sort_by_key(|(distance, _)| *distance);
    scored
        .into_iter()
        .map(|(_, candidate)| candidate.to_owned())
        .collect()
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
