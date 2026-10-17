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
            Self::InvalidValue {
                option,
                value,
                kind,
                ..
            } => {
                write!(f, "invalid value for {kind} {option}: {value}")
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
        // A value that begins with a dash is still a value. Without this,
        // `--lat -40.41` reads the number as an unknown option rather than as
        // this flag's value, which is the whole of the negative-number row.
        arg = arg.allow_hyphen_values(true);
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

    /// Read a file and decode its text through a caller's decoder
    /// (`Primitive.FileSchema` core): the decoder's own failure message is what
    /// the caller sees, and a missing file fails before the decoder runs. The
    /// pin carries the schema itself; the decoder is the port's seam for it.
    pub fn parse_file_schema<A>(
        path: &str,
        decode: impl Fn(&str) -> Result<A, String>,
    ) -> Result<A, String> {
        let text = parse_file_text(path)?;
        decode(&text)
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

/// Built-in `--log-level` setting (mirrors `GlobalFlag.LogLevel`):
/// parse the setting from argv, then filter emitted logs by severity.
pub mod log_level {
    /// Canonical log levels, most to least severe (`All` shows
    /// everything, `None` shows nothing).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum LogLevel {
        /// Show every emission.
        All,
        /// Finest severity.
        Trace,
        /// Debug severity.
        Debug,
        /// Default minimum.
        Info,
        /// Warning severity.
        Warn,
        /// Error severity.
        Error,
        /// Most severe.
        Fatal,
        /// Show nothing.
        None,
    }

    /// Severity rank: lower is more severe (`All`/`None` are not ranked).
    fn rank(level: LogLevel) -> u8 {
        match level {
            LogLevel::Fatal => 0,
            LogLevel::Error => 1,
            LogLevel::Warn => 2,
            LogLevel::Info => 3,
            LogLevel::Debug => 4,
            LogLevel::Trace => 5,
            LogLevel::All | LogLevel::None => unreachable!("sentinels never rank"),
        }
    }

    /// Parse one `--log-level` value (`warning` aliases `warn`).
    fn parse_value(value: &str) -> Result<LogLevel, super::CliError> {
        match value {
            "all" => Ok(LogLevel::All),
            "trace" => Ok(LogLevel::Trace),
            "debug" => Ok(LogLevel::Debug),
            "info" => Ok(LogLevel::Info),
            "warn" | "warning" => Ok(LogLevel::Warn),
            "error" => Ok(LogLevel::Error),
            "fatal" => Ok(LogLevel::Fatal),
            "none" => Ok(LogLevel::None),
            _ => Err(super::CliError::InvalidValue {
                option: "log-level".to_owned(),
                value: value.to_owned(),
                expected: "all|trace|debug|info|warn|warning|error|fatal|none".to_owned(),
                kind: "option".to_owned(),
            }),
        }
    }

    /// Resolve the built-in setting from argv (`--log-level v` or
    /// `--log-level=v`): absent is `None` (default minimum `Info`
    /// applies); an unknown value fails as `InvalidValue`.
    pub fn resolve_setting(argv: &[String]) -> Result<Option<LogLevel>, super::CliError> {
        let mut index = 0;
        while index < argv.len() {
            let arg = &argv[index];
            if let Some(inline) = arg.strip_prefix("--log-level=") {
                return parse_value(inline).map(Some);
            }
            if arg == "--log-level" {
                match argv.get(index + 1) {
                    Some(next) => return parse_value(next).map(Some),
                    None => {
                        return Err(super::CliError::InvalidValue {
                            option: "log-level".to_owned(),
                            value: String::new(),
                            expected: "all|trace|debug|info|warn|warning|error|fatal|none"
                                .to_owned(),
                            kind: "option".to_owned(),
                        });
                    }
                }
            }
            index += 1;
        }
        Ok(None)
    }

    /// `true` when an emission at `emit` passes minimum `min`
    /// (`None` is the omitted-setting default, `Info`).
    pub fn visible(emit: LogLevel, min: Option<LogLevel>) -> bool {
        match min.unwrap_or(LogLevel::Info) {
            LogLevel::All => true,
            LogLevel::None => false,
            min => rank(emit) <= rank(min),
        }
    }

    /// Keep the emissions passing `min`, preserving emission order.
    pub fn filter_emitted<A: Clone>(
        emitted: &[(LogLevel, A)],
        min: Option<LogLevel>,
    ) -> Vec<(LogLevel, A)> {
        emitted
            .iter()
            .filter(|(level, _)| visible(*level, min))
            .cloned()
            .collect()
    }
}

/// Structured help (`HelpDoc`) and its plain-text formatter (`cli/CliOutput`
/// + `cli/HelpDoc` at the pin, colors off).
///
/// The existing [`CliCommand`] tree drives clap parsing; this module is the
/// parallel help surface the `Help.test.ts` rows pin: a `HelpDoc` value per
/// command (flags with required markers, hidden/unlisted filtering, aliases,
/// groups, examples) rendered by [`format_help`] with the same column
/// alignment (CJK width 2, zero-width marks 0).
pub mod help {
    /// One flag as declared on a command (input to [`HelpNode`)).
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct HelpFlagDecl {
        /// Long name, no dashes.
        pub name: String,
        /// Raw aliases (`withAlias`); formatted with `-`/`--` on render.
        pub aliases: Vec<String>,
        /// Value shape (`string`, `boolean`, `integer`, `file`, or a metavar
        /// like `<bash|zsh|fish|sh>`).
        pub type_name: String,
        /// Help text.
        pub description: Option<String>,
        /// Choice keys appended as `(choices: a, b)` (mirrors `Flag.Literals`).
        pub choice_keys: Vec<String>,
        /// Omission fails (mirrors `required`: non-boolean without
        /// `optional`/`withDefault`).
        pub required: bool,
        /// Hidden flags still parse but never render (`withHidden`).
        pub hidden: bool,
    }

    impl HelpFlagDecl {
        /// A required text flag (`Flag.String`).
        pub fn string(name: &str) -> Self {
            Self {
                name: name.to_owned(),
                aliases: Vec::new(),
                type_name: "string".to_owned(),
                description: None,
                choice_keys: Vec::new(),
                required: true,
                hidden: false,
            }
        }

        /// A switch (`Flag.Boolean`): never required.
        pub fn boolean(name: &str) -> Self {
            Self {
                required: false,
                type_name: "boolean".to_owned(),
                ..Self::string(name)
            }
        }

        /// An integer flag (`Flag.Int`).
        pub fn integer(name: &str) -> Self {
            Self {
                type_name: "integer".to_owned(),
                ..Self::string(name)
            }
        }

        /// A path flag (`Flag.File`, `Flag.Path`, file readers).
        pub fn file(name: &str) -> Self {
            Self {
                type_name: "file".to_owned(),
                ..Self::string(name)
            }
        }

        /// One of a fixed set (`Flag.Literals` without metavar).
        pub fn choice(name: &str, keys: &[&str]) -> Self {
            Self {
                choice_keys: keys.iter().map(|key| key.to_string()).collect(),
                ..Self::string(name)
            }
        }

        /// Swap the displayed type (`withMetavar`).
        pub fn with_metavar(mut self, metavar: &str) -> Self {
            self.type_name = metavar.to_owned();
            self
        }

        /// Add a raw alias (`withAlias`).
        pub fn with_alias(mut self, alias: &str) -> Self {
            self.aliases.push(alias.to_owned());
            self
        }

        /// Attach help text (`withDescription`).
        pub fn with_description(mut self, text: &str) -> Self {
            self.description = Some(text.to_owned());
            self
        }

        /// Allow absence (`optional`).
        pub fn optional(mut self) -> Self {
            self.required = false;
            self
        }

        /// Fall back when absent (`withDefault` implies optional metadata).
        pub fn with_default(mut self) -> Self {
            self.required = false;
            self
        }

        /// Parse but never render (`withHidden`).
        pub fn hidden(mut self) -> Self {
            self.hidden = true;
            self
        }
    }

    /// One positional argument as declared on a command.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct HelpArgDecl {
        /// Argument name.
        pub name: String,
        /// Value shape (`string`, `file`, ...).
        pub type_name: String,
        /// Help text.
        pub description: Option<String>,
        /// Omission fails (mirrors `required`: not optional, and a variadic
        /// needs `min >= 1`).
        pub required: bool,
        /// Consumes the rest (`variadic`).
        pub variadic: bool,
    }

    impl HelpArgDecl {
        /// A required text argument (`Argument.String`).
        pub fn string(name: &str) -> Self {
            Self {
                name: name.to_owned(),
                type_name: "string".to_owned(),
                description: None,
                required: true,
                variadic: false,
            }
        }

        /// A file argument (`Argument.File`).
        pub fn file(name: &str) -> Self {
            Self {
                type_name: "file".to_owned(),
                ..Self::string(name)
            }
        }

        /// Attach help text (`withDescription`).
        pub fn with_description(mut self, text: &str) -> Self {
            self.description = Some(text.to_owned());
            self
        }

        /// Allow absence (`optional`).
        pub fn optional(mut self) -> Self {
            self.required = false;
            self
        }

        /// Fall back when absent (`withDefault` implies optional metadata).
        pub fn with_default(mut self) -> Self {
            self.required = false;
            self
        }

        /// Consume the rest with no minimum (`variadic()`): not required.
        pub fn variadic(mut self) -> Self {
            self.variadic = true;
            self.required = false;
            self
        }

        /// Consume the rest with a minimum (`variadic({ min })`).
        pub fn variadic_min(mut self, min: usize) -> Self {
            self.variadic = true;
            self.required = min > 0;
            self
        }
    }

    /// One command node (`Command.make` plus its combinators).
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct HelpNode {
        /// Command name.
        pub name: String,
        /// Long description.
        pub description: String,
        /// One-line listing override (`shortDescription` wins in listings).
        pub short_description: Option<String>,
        /// Second name (`withAlias`).
        pub alias: Option<String>,
        /// Absent from help but still parses (`unlisted`).
        pub unlisted: bool,
        /// Flags declared here (own shared flags merged in, like the pin).
        pub local_flags: Vec<HelpFlagDecl>,
        /// Flags shared with children (`withSharedFlags` context).
        pub shared_flags: Vec<HelpFlagDecl>,
        /// Positionals in declaration order.
        pub args: Vec<HelpArgDecl>,
        /// Child commands, grouped.
        pub groups: Vec<HelpGroup>,
        /// Usage examples.
        pub examples: Vec<HelpExample>,
    }

    /// One subcommand group (`{ group, commands }` or a bare command).
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct HelpGroup {
        /// Group heading; `None` renders under `SUBCOMMANDS`.
        pub group: Option<String>,
        /// Commands in this group.
        pub commands: Vec<HelpNode>,
    }

    /// One usage example.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct HelpExample {
        /// Invocation line.
        pub command: String,
        /// Explanation rendered as a `#` comment above the line.
        pub description: Option<String>,
    }

    impl HelpNode {
        /// A command with no flags or arguments (`Command.make`).
        pub fn new(name: &str) -> Self {
            Self {
                name: name.to_owned(),
                ..Self::default()
            }
        }

        /// Set the long description (`withDescription`).
        pub fn with_description(mut self, text: &str) -> Self {
            self.description = text.to_owned();
            self
        }

        /// Add a second name (`withAlias`).
        pub fn with_alias(mut self, alias: &str) -> Self {
            self.alias = Some(alias.to_owned());
            self
        }

        /// Keep out of help while still parsing (`unlisted`).
        pub fn unlisted(mut self) -> Self {
            self.unlisted = true;
            self
        }

        /// Add flags (`Command.make`'s second argument).
        pub fn with_flags(mut self, flags: Vec<HelpFlagDecl>) -> Self {
            self.local_flags.extend(flags);
            self
        }

        /// Share flags with children: merged into the local set too, exactly
        /// like the pin's `withSharedFlags`.
        pub fn with_shared_flags(mut self, flags: Vec<HelpFlagDecl>) -> Self {
            self.local_flags.extend(flags.clone());
            self.shared_flags.extend(flags);
            self
        }

        /// Add positionals.
        pub fn with_args(mut self, args: Vec<HelpArgDecl>) -> Self {
            self.args.extend(args);
            self
        }

        /// Add children under the default heading (`withSubcommands`).
        pub fn with_subcommands(mut self, commands: Vec<HelpNode>) -> Self {
            self.groups.push(HelpGroup {
                group: None,
                commands,
            });
            self
        }

        /// Add children in explicit groups (ungrouped entries allowed).
        pub fn with_groups(mut self, groups: Vec<HelpGroup>) -> Self {
            self.groups.extend(groups);
            self
        }

        /// Add usage examples (`withExamples`).
        pub fn with_examples(mut self, examples: Vec<HelpExample>) -> Self {
            self.examples.extend(examples);
            self
        }

        /// Find a flag by name or alias, hidden ones included: hidden flags
        /// still parse on the command line.
        pub fn find_flag(&self, name: &str) -> Option<&HelpFlagDecl> {
            self.local_flags
                .iter()
                .chain(self.shared_flags.iter())
                .find(|flag| flag.name == name || flag.aliases.iter().any(|a| a == name))
        }

        /// Find a child by name or alias, unlisted ones included: unlisted
        /// subcommands still parse on the command line.
        pub fn find_subcommand(&self, name: &str) -> Option<&HelpNode> {
            self.groups
                .iter()
                .flat_map(|group| &group.commands)
                .find(|child| child.name == name || child.alias.as_deref() == Some(name))
        }

        /// Visible child names (names plus aliases, unlisted excluded): the
        /// only candidates unknown-subcommand suggestions may reveal.
        pub fn visible_subcommand_names(&self) -> Vec<&str> {
            let mut names = Vec::new();
            for child in self.groups.iter().flat_map(|group| &group.commands) {
                if child.unlisted {
                    continue;
                }
                names.push(child.name.as_str());
                if let Some(alias) = &child.alias {
                    names.push(alias.as_str());
                }
            }
            names
        }

        /// Visible flag names (hidden excluded): the only candidates
        /// unrecognized-flag suggestions may reveal.
        pub fn visible_flag_names(&self) -> Vec<&str> {
            self.local_flags
                .iter()
                .filter(|flag| !flag.hidden)
                .map(|flag| flag.name.as_str())
                .collect()
        }

        /// Resolve a child by exact name, suggesting only visible names on a
        /// miss (unlisted names never leak through suggestions).
        pub fn resolve_subcommand(&self, name: &str) -> Result<&HelpNode, UnknownSubcommand> {
            match self.find_subcommand(name) {
                Some(child) => Ok(child),
                None => Err(UnknownSubcommand {
                    subcommand: name.to_owned(),
                    suggestions: super::suggest(name, &self.visible_subcommand_names()),
                }),
            }
        }

        /// Structured help for this node at `command_path`
        /// (`buildHelpDoc`): hidden flags and unlisted subcommands are
        /// omitted, and an all-unlisted tree reads as a leaf command.
        pub fn doc(&self, command_path: &[String]) -> HelpDoc {
            let args: Vec<ArgDoc> = self
                .args
                .iter()
                .map(|arg| ArgDoc {
                    name: arg.name.clone(),
                    type_name: arg.type_name.clone(),
                    description: arg.description.clone(),
                    required: arg.required,
                    variadic: arg.variadic,
                })
                .collect();

            let mut usage = if command_path.is_empty() {
                self.name.clone()
            } else {
                command_path.join(" ")
            };
            let has_visible = self
                .groups
                .iter()
                .any(|group| group.commands.iter().any(|child| !child.unlisted));
            if has_visible {
                usage += " <subcommand>";
            }
            usage += " [flags]";
            for arg in &args {
                let shape = if arg.variadic {
                    format!("<{}...>", arg.name)
                } else {
                    format!("<{}>", arg.name)
                };
                usage += " ";
                usage += &if arg.required {
                    shape
                } else {
                    format!("[{shape}]")
                };
            }

            let flags: Vec<FlagDoc> = self
                .local_flags
                .iter()
                .filter(|flag| !flag.hidden)
                .map(to_flag_doc)
                .collect();

            let mut subcommands = Vec::new();
            for group in &self.groups {
                let visible: Vec<SubcommandDoc> = group
                    .commands
                    .iter()
                    .filter(|child| !child.unlisted)
                    .map(|child| SubcommandDoc {
                        name: child.name.clone(),
                        alias: child.alias.clone(),
                        short_description: child.short_description.clone(),
                        description: child.description.clone(),
                    })
                    .collect();
                if visible.is_empty() {
                    continue;
                }
                subcommands.push(SubcommandGroupDoc {
                    group: group.group.clone(),
                    commands: visible,
                });
            }

            HelpDoc {
                description: self.description.clone(),
                usage,
                flags,
                global_flags: Vec::new(),
                args,
                subcommands,
                examples: self
                    .examples
                    .iter()
                    .map(|example| ExampleDoc {
                        command: example.command.clone(),
                        description: example.description.clone(),
                    })
                    .collect(),
            }
        }
    }

    /// A rendered flag (`HelpDoc.FlagDoc`).
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct FlagDoc {
        /// Long name, no dashes.
        pub name: String,
        /// Formatted aliases (`-v`, `--format`).
        pub aliases: Vec<String>,
        /// Value shape or metavar.
        pub type_name: String,
        /// Help text with any `(choices: ...)` suffix.
        pub description: Option<String>,
        /// Whether omission fails.
        pub required: bool,
    }

    /// A rendered positional (`HelpDoc.ArgDoc`).
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct ArgDoc {
        /// Argument name.
        pub name: String,
        /// Value shape.
        pub type_name: String,
        /// Help text.
        pub description: Option<String>,
        /// Whether omission fails.
        pub required: bool,
        /// Whether it consumes the rest.
        pub variadic: bool,
    }

    /// A rendered subcommand (`HelpDoc.SubcommandDoc`).
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct SubcommandDoc {
        /// Command name.
        pub name: String,
        /// Second name, when set.
        pub alias: Option<String>,
        /// One-line override, when set.
        pub short_description: Option<String>,
        /// Brief description.
        pub description: String,
    }

    /// A rendered subcommand group (`HelpDoc.SubcommandGroupDoc`).
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct SubcommandGroupDoc {
        /// Heading; `None` renders under `SUBCOMMANDS`.
        pub group: Option<String>,
        /// Commands in this group.
        pub commands: Vec<SubcommandDoc>,
    }

    /// A rendered example (`HelpDoc.ExampleDoc`).
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct ExampleDoc {
        /// Invocation line.
        pub command: String,
        /// Explanation, when set.
        pub description: Option<String>,
    }

    /// Structured help for one command path (`HelpDoc`).
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct HelpDoc {
        /// Brief description of the command.
        pub description: String,
        /// Usage line (`mycli copy [flags] <source>`).
        pub usage: String,
        /// Shared ancestor flags first, then local ones.
        pub flags: Vec<FlagDoc>,
        /// Built-in global flags for the path.
        pub global_flags: Vec<FlagDoc>,
        /// Positionals in declaration order.
        pub args: Vec<ArgDoc>,
        /// Visible subcommands, grouped.
        pub subcommands: Vec<SubcommandGroupDoc>,
        /// Usage examples.
        pub examples: Vec<ExampleDoc>,
    }

    /// A miss resolving a subcommand (suggestions carry visible names only).
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct UnknownSubcommand {
        /// Subcommand as typed.
        pub subcommand: String,
        /// Visible names with a similar spelling.
        pub suggestions: Vec<String>,
    }

    /// Format one declared flag for display (`toFlagDoc`): aliases gain
    /// `-`/`--`, choice keys append `(choices: ...)`, booleans are never
    /// required.
    fn to_flag_doc(flag: &HelpFlagDecl) -> FlagDoc {
        let aliases = flag
            .aliases
            .iter()
            .map(|alias| {
                if alias.chars().count() == 1 {
                    format!("-{alias}")
                } else {
                    format!("--{alias}")
                }
            })
            .collect();
        let description = match (flag.description.clone(), flag.choice_keys.as_slice()) {
            (_, []) => flag.description.clone(),
            (None, keys) => Some(format!("(choices: {})", keys.join(", "))),
            (Some(text), keys) => Some(format!("{text} (choices: {})", keys.join(", "))),
        };
        FlagDoc {
            name: flag.name.clone(),
            aliases,
            type_name: flag.type_name.clone(),
            description,
            required: flag.required,
        }
    }

    /// Built-in global flags for the path (`--help`, `--version`,
    /// `--wizard`, `--completions`, `--log-level`).
    pub fn builtin_global_flags() -> Vec<FlagDoc> {
        vec![
            FlagDoc {
                name: "help".to_owned(),
                aliases: vec!["-h".to_owned()],
                type_name: "boolean".to_owned(),
                description: Some("Show help information".to_owned()),
                required: false,
            },
            FlagDoc {
                name: "version".to_owned(),
                aliases: vec!["-v".to_owned()],
                type_name: "boolean".to_owned(),
                description: Some("Show version information".to_owned()),
                required: false,
            },
            FlagDoc {
                name: "wizard".to_owned(),
                aliases: Vec::new(),
                type_name: "boolean".to_owned(),
                description: Some("Start wizard mode for a command".to_owned()),
                required: false,
            },
            FlagDoc {
                name: "completions".to_owned(),
                aliases: Vec::new(),
                type_name: "<bash|zsh|fish|sh>".to_owned(),
                description: Some(
                    "Print shell completion script (choices: bash, zsh, fish, sh)".to_owned(),
                ),
                required: false,
            },
            FlagDoc {
                name: "log-level".to_owned(),
                aliases: Vec::new(),
                type_name: "<all|trace|debug|info|warn|warning|error|fatal|none>".to_owned(),
                description: Some(
                    "Sets the minimum log level (choices: all, trace, debug, info, warn, warning, error, fatal, none)"
                        .to_owned(),
                ),
                required: false,
            },
        ]
    }

    /// Structured help for `path` under `root` (`getHelpForCommandPath`):
    /// the current node's doc plus inherited shared ancestor flags (deduped
    /// by name, hidden excluded) prepended to the local flags, with the
    /// built-in global flags attached. `None` when the path misses.
    pub fn help_for_path(root: &HelpNode, path: &[&str]) -> Option<HelpDoc> {
        let mut lineage: Vec<&HelpNode> = vec![root];
        for part in &path[1..] {
            let current = lineage[lineage.len() - 1];
            match current.find_subcommand(part) {
                Some(child) => lineage.push(child),
                None => return None,
            }
        }
        let owned: Vec<String> = path.iter().map(|part| part.to_string()).collect();
        let current = lineage[lineage.len() - 1];
        let base = current.doc(&owned);

        let mut seen: Vec<&str> = base.flags.iter().map(|flag| flag.name.as_str()).collect();
        let mut shared = Vec::new();
        for ancestor in &lineage[..lineage.len() - 1] {
            for flag in &ancestor.shared_flags {
                if flag.hidden || seen.contains(&flag.name.as_str()) {
                    continue;
                }
                seen.push(&flag.name);
                shared.push(to_flag_doc(flag));
            }
        }

        Some(HelpDoc {
            flags: shared.into_iter().chain(base.flags).collect(),
            global_flags: builtin_global_flags(),
            ..base
        })
    }

    /// Terminal display width of a string, mirroring the pin's
    /// `visualLength`: grapheme clusters count 2 when full-width or emoji
    /// presentation, 0 when only zero-width marks, else 1.
    ///
    /// Segmentation is a narrow UAX-29 subset (base + combining marks +
    /// ZWJ joins); Indic spacing marks and regional-indicator pairs keep
    /// their plain width instead of the pin's exact shaping.
    pub fn visual_width(text: &str) -> usize {
        let mut width = 0;
        let mut cluster: Vec<char> = Vec::new();
        let mut join_next = false;
        let flush = |cluster: &[char], width: &mut usize| {
            *width += grapheme_width(cluster);
        };
        for ch in text.chars() {
            if cluster.is_empty() {
                cluster.push(ch);
                join_next = ch == '\u{200d}';
            } else if is_extend(ch) || ch == '\u{200d}' || join_next {
                cluster.push(ch);
                join_next = ch == '\u{200d}';
            } else {
                flush(&cluster, &mut width);
                cluster.clear();
                cluster.push(ch);
                join_next = ch == '\u{200d}';
            }
        }
        if !cluster.is_empty() {
            flush(&cluster, &mut width);
        }
        width
    }

    /// Width of one grapheme cluster: strip zero-width marks, then 2 for
    /// emoji presentation or full-width, else 1.
    fn grapheme_width(cluster: &[char]) -> usize {
        let visible: Vec<char> = cluster
            .iter()
            .copied()
            .filter(|ch| !is_zero_width(*ch))
            .collect();
        if visible.is_empty() {
            return 0;
        }
        if visible.contains(&'\u{fe0f}') || visible.iter().any(|ch| is_emoji_presentation(*ch)) {
            return 2;
        }
        if is_full_width(visible[0] as u32) {
            return 2;
        }
        1
    }

    /// Combining and enclosing marks that join the base grapheme.
    fn is_extend(ch: char) -> bool {
        let cp = ch as u32;
        matches!(
            cp,
            0x0300..=0x036f
                | 0x1ab0..=0x1aff
                | 0x1dc0..=0x1dff
                | 0x20d0..=0x20ff
                | 0xfe00..=0xfe0f
                | 0xfe20..=0xfe2f
                | 0xe0100..=0xe01ef
        )
    }

    /// Zero-width code points (the pin's `\p{Mark}\p{Control}` plus the
    /// default-ignorable format controls, narrowed to the ranges the help
    /// rows exercise).
    fn is_zero_width(ch: char) -> bool {
        let cp = ch as u32;
        if matches!(cp, 0x0000..=0x001f | 0x007f..=0x009f) {
            return true;
        }
        if is_extend(ch) {
            return true;
        }
        matches!(
            cp,
            0x00ad
                | 0x034f
                | 0x061c
                | 0x115f..=0x1160
                | 0x17b4..=0x17b5
                | 0x180b..=0x180f
                | 0x200b..=0x200f
                | 0x202a..=0x202e
                | 0x2060..=0x206f
                | 0x3164
                | 0xfeff
                | 0xffa0
                | 0x1bca0..=0x1bca3
                | 0x1d173..=0x1d17a
                | 0xe0000..=0xe0fff
        )
    }

    /// Full-width code points, copied from the pin's ranges.
    fn is_full_width(cp: u32) -> bool {
        if cp < 0x1100 {
            return false;
        }
        matches!(
            cp,
            0x1100..=0x115f
                | 0x2329..=0x232a
                | 0x2e80..=0x303e
                | 0x3040..=0xa4cf
                | 0xac00..=0xd7a3
                | 0xf900..=0xfaff
                | 0xfe10..=0xfe19
                | 0xfe30..=0xfe6f
                | 0xff00..=0xff60
                | 0xffe0..=0xffe6
                | 0x1b000..=0x1b2ff
                | 0x1f200..=0x1f251
                | 0x20000..=0x3fffd
        )
    }

    /// Code points with emoji presentation (astral pictographs wholesale,
    /// plus the text-default symbols that present as emoji).
    fn is_emoji_presentation(ch: char) -> bool {
        let cp = ch as u32;
        if matches!(cp, 0x1f000..=0x1faff) {
            return true;
        }
        matches!(
            cp,
            0x231a..=0x231b
                | 0x23e9..=0x23ec
                | 0x23f0
                | 0x23f3
                | 0x25fd..=0x25fe
                | 0x2614..=0x2615
                | 0x2648..=0x2653
                | 0x267f
                | 0x2693
                | 0x26a1
                | 0x26aa..=0x26ab
                | 0x26bd..=0x26be
                | 0x26c4..=0x26c5
                | 0x26ce
                | 0x26d4
                | 0x26ea
                | 0x26f2..=0x26f3
                | 0x26f5
                | 0x26fa
                | 0x26fd
                | 0x2705
                | 0x270a..=0x270b
                | 0x2728
                | 0x274c
                | 0x274e
                | 0x2753..=0x2755
                | 0x2757
                | 0x2795..=0x2797
                | 0x27b0
                | 0x27bf
                | 0x2b1b..=0x2b1c
                | 0x2b50
                | 0x2b55
        )
    }

    /// Pad `text` with spaces to terminal `width` (mirrors `pad`).
    fn pad_to(text: &str, width: usize) -> String {
        let padding = width.saturating_sub(visual_width(text));
        format!("{text}{}", " ".repeat(padding))
    }

    /// Aligned two-column table (mirrors `renderTable`): the description
    /// column sits at the widest left cell plus four, capped at `width_cap`.
    fn render_table(rows: &[(String, String)], width_cap: Option<usize>) -> String {
        let widest = rows
            .iter()
            .map(|(left, _)| visual_width(left))
            .max()
            .unwrap_or(0);
        let column = match width_cap {
            Some(cap) => widest.saturating_add(4).min(cap),
            None => widest.saturating_add(4),
        };
        rows.iter()
            .map(|(left, right)| {
                let target = column.max(visual_width(left).saturating_add(1));
                format!("  {}{right}", pad_to(left, target))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn flag_left(flag: &FlagDoc) -> String {
        let mut names = vec![format!("--{}", flag.name)];
        names.extend(flag.aliases.iter().cloned());
        let names = names.join(", ");
        if flag.type_name == "boolean" {
            names
        } else {
            format!("{names} {}", flag.type_name)
        }
    }

    /// Render a `HelpDoc` as plain text (the pin's `formatHelpDocImpl` with
    /// colors off).
    pub fn format_help(doc: &HelpDoc) -> String {
        let mut sections: Vec<String> = Vec::new();
        if !doc.description.is_empty() {
            sections.push("DESCRIPTION".to_owned());
            sections.push(format!("  {}", doc.description));
            sections.push(String::new());
        }
        sections.push("USAGE".to_owned());
        sections.push(format!("  {}", doc.usage));
        sections.push(String::new());
        if !doc.args.is_empty() {
            sections.push("ARGUMENTS".to_owned());
            let rows: Vec<(String, String)> = doc
                .args
                .iter()
                .map(|arg| {
                    let name = if arg.variadic {
                        format!("{}...", arg.name)
                    } else {
                        arg.name.clone()
                    };
                    let left = format!("{name} {}", arg.type_name);
                    let text = arg.description.clone().unwrap_or_default();
                    let right = if arg.required {
                        text
                    } else {
                        format!("{text} (optional)")
                    };
                    (left, right)
                })
                .collect();
            sections.push(render_table(&rows, Some(25)));
            sections.push(String::new());
        }
        if !doc.flags.is_empty() {
            sections.push("FLAGS".to_owned());
            let rows: Vec<(String, String)> = doc
                .flags
                .iter()
                .map(|flag| {
                    (
                        flag_left(flag),
                        flag.description.clone().unwrap_or_default(),
                    )
                })
                .collect();
            sections.push(render_table(&rows, None));
            sections.push(String::new());
        }
        if !doc.global_flags.is_empty() {
            sections.push("GLOBAL FLAGS".to_owned());
            let rows: Vec<(String, String)> = doc
                .global_flags
                .iter()
                .map(|flag| {
                    (
                        flag_left(flag),
                        flag.description.clone().unwrap_or_default(),
                    )
                })
                .collect();
            sections.push(render_table(&rows, None));
            sections.push(String::new());
        }
        if !doc.subcommands.is_empty() {
            let ungrouped = doc.subcommands.iter().find(|group| group.group.is_none());
            if let Some(group) = ungrouped {
                sections.push("SUBCOMMANDS".to_owned());
                sections.push(render_subcommands(&group.commands));
                if doc.subcommands.len() > 1 {
                    sections.push(String::new());
                }
            }
            for group in doc.subcommands.iter().filter(|group| group.group.is_some()) {
                sections.push(format!("{}:", group.group.clone().unwrap_or_default()));
                sections.push(render_subcommands(&group.commands));
                sections.push(String::new());
            }
        }
        if !doc.examples.is_empty() {
            sections.push("EXAMPLES".to_owned());
            let mut first = true;
            let mut previous_had_description = false;
            for example in &doc.examples {
                if example.description.is_some() {
                    if !first {
                        sections.push(String::new());
                    }
                    sections.push(format!(
                        "  # {}",
                        example.description.clone().unwrap_or_default()
                    ));
                } else if previous_had_description {
                    sections.push(String::new());
                }
                sections.push(format!("  {}", example.command));
                first = false;
                previous_had_description = example.description.is_some();
            }
            sections.push(String::new());
        }
        if sections.last().is_some_and(|last| last.is_empty()) {
            sections.pop();
        }
        sections.join("\n")
    }

    /// Render one subcommand listing (mirrors the pin's `SUBCOMMANDS` table,
    /// width cap 20, alias as `name, alias`, short description first).
    fn render_subcommands(commands: &[SubcommandDoc]) -> String {
        let rows: Vec<(String, String)> = commands
            .iter()
            .map(|sub| {
                let left = match &sub.alias {
                    Some(alias) => format!("{}, {alias}", sub.name),
                    None => sub.name.clone(),
                };
                let right = sub
                    .short_description
                    .clone()
                    .unwrap_or_else(|| sub.description.clone());
                (left, right)
            })
            .collect();
        render_table(&rows, Some(20))
    }
}

/// Interactive prompts (`cli/Prompt` at the pin): the pure core behind the
/// `MockTerminal` rows. Keystrokes in, ANSI frames out, and the `Effect`
/// plumbing stay in TypeScript (no TTY harness exists here); what ports is
/// the logic the rows pin: date-mask rendering, per-field typed buffers,
/// text-buffer editing, int/float keystroke parsing, case-insensitive
/// choice filtering, wrapping cursor steps, password masking, theme
/// symbols/separators/colors, and multi-select set operations with min/max
/// validation.
pub mod prompt {
    /// Theme symbols and colors (mirrors `defaultTheme`; overrides arrive
    /// via struct-update syntax, matching `makeTheme` partial overrides).
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct PromptTheme {
        /// Leading symbol (`?`).
        pub prefix: String,
        /// Active-choice pointer (`❯`).
        pub pointer: String,
        /// Small pointer after the message (`›`).
        pub pointer_small: String,
        /// Submitted tick (`✔`).
        pub tick: String,
        /// Submitted ellipsis (`…`).
        pub ellipsis: String,
        /// Paging arrow up (`↑`).
        pub arrow_up: String,
        /// Paging arrow down (`↓`).
        pub arrow_down: String,
        /// Selected checkbox (`☒`).
        pub checkbox_on: String,
        /// Unselected checkbox (`☐`).
        pub checkbox_off: String,
        /// Title/description separator (`- `).
        pub description_separator: String,
        /// Mask per password character (`*`).
        pub password_mask: String,
        /// Toggle label separator (`/`).
        pub toggle_separator: String,
        /// Highlight color.
        pub primary_color: String,
        /// Dim color.
        pub muted_color: String,
        /// Success color.
        pub success_color: String,
        /// Error color.
        pub error_color: String,
        /// Submitted-value color.
        pub submitted_color: String,
    }

    impl Default for PromptTheme {
        fn default() -> Self {
            Self {
                prefix: "?".to_owned(),
                pointer: "❯".to_owned(),
                pointer_small: "›".to_owned(),
                tick: "✔".to_owned(),
                ellipsis: "…".to_owned(),
                arrow_up: "↑".to_owned(),
                arrow_down: "↓".to_owned(),
                checkbox_on: "☒".to_owned(),
                checkbox_off: "☐".to_owned(),
                description_separator: "- ".to_owned(),
                password_mask: "*".to_owned(),
                toggle_separator: "/".to_owned(),
                // The pin's ANSI colors; rows pin the codes, not the hues.
                primary_color: "\x1b[36m".to_owned(),
                muted_color: "\x1b[90m".to_owned(),
                success_color: "\x1b[32m".to_owned(),
                error_color: "\x1b[31m".to_owned(),
                submitted_color: "\x1b[37m".to_owned(),
            }
        }
    }

    /// Wrap `text` in `color` (mirrors `Ansi.annotate`: color plus reset).
    pub fn annotate(color: &str, text: &str) -> String {
        format!("{color}{text}\x1b[0m")
    }

    /// `symbol` plus one space before `text`; an empty symbol leaves `text`
    /// bare (mirrors `separateSymbol`).
    pub fn separate_symbol(symbol: &str, text: &str) -> String {
        if symbol.is_empty() {
            text.to_owned()
        } else {
            format!("{symbol} {text}")
        }
    }

    /// Prompt title: `prefix` plus one space before the message; empty
    /// symbols render the bare message (the `:291` row pins `"Name"`).
    pub fn title_line(prefix: &str, message: &str) -> String {
        separate_symbol(prefix, message)
    }

    /// Optional prompt header: no message means no header (the `:405` row
    /// pins the choice-list-only frame); an empty message still renders one
    /// (the `:428` row pins `?  › `).
    pub fn title_line_opt(
        message: Option<&str>,
        prefix: &str,
        pointer_small: &str,
    ) -> Option<String> {
        message.map(|message| format!("{} {pointer_small}", title_line(prefix, message)))
    }

    /// `left` plus one space plus the separated `right` (the `:484` row
    /// pins `"First one"` for an empty separator, never `"Firstone"`).
    pub fn join_pair(left: &str, separator: &str, right: &str) -> String {
        format!("{left} {}", separate_symbol(separator, right))
    }

    /// Toggle labels (mirrors the pin's
    /// `active + " " + separateSymbol(separator, inactive)`): the default
    /// `/` reads `"on / off"`, an empty separator still keeps one space.
    pub fn toggle_text(active: &str, inactive: &str, separator: &str) -> String {
        format!("{active} {}", separate_symbol(separator, inactive))
    }

    /// Two-digit year (mirrors `Year.toString` for `YY`).
    pub fn two_digit_year(year: i64) -> String {
        let padded = format!("{year:04}");
        padded[padded.len() - 2..].to_owned()
    }

    /// Day ordinal suffix (mirrors `Day.ordinalIndicator`: 11-13 are `th`).
    pub fn ordinal_indicator(day: u32) -> &'static str {
        if (11..=13).contains(&day) {
            return "th";
        }
        match day % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        }
    }

    /// Meridiem label (mirrors `Meridiem.toString`: `A` uppercases).
    pub fn meridiem(hour: u32, uppercase: bool) -> &'static str {
        let lower = hour >= 12;
        match (lower, uppercase) {
            (true, true) => "PM",
            (true, false) => "pm",
            (false, true) => "AM",
            (false, false) => "am",
        }
    }

    /// Outcome of a `ctrl-<key>` press on a text buffer: only clearing and
    /// cursor moves are bound; anything else (e.g. `ctrl-l`) is ignored
    /// (the `:367` row pins `"Jane"` surviving a `ctrl-l`).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum CtrlOutcome {
        /// `ctrl-u`: the buffer was cleared.
        Cleared,
        /// `ctrl-a`: the cursor moved to the start.
        MovedToStart,
        /// `ctrl-e`: the cursor moved to the end.
        MovedToEnd,
        /// Unbound combination: the buffer is untouched.
        Ignored,
    }

    impl CtrlOutcome {
        /// `true` for unbound combinations.
        pub fn is_ignored(self) -> bool {
            matches!(self, Self::Ignored)
        }
    }

    /// Editable line buffer (the pure core behind `TextState`: value plus
    /// cursor, in characters; the pin counts UTF-16 units, so astral-plane
    /// cursor columns differ by one per astral character).
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct TextBuffer {
        value: Vec<char>,
        cursor: usize,
    }

    impl TextBuffer {
        /// An empty buffer with the cursor at the start.
        pub fn new() -> Self {
            Self::default()
        }

        /// A buffer prefilled with the default, cursor at the end (typing
        /// appends to the default: the `:144`/`:304` rows pin `"Jane Doe"`).
        pub fn with_default(default: &str) -> Self {
            let value: Vec<char> = default.chars().collect();
            let cursor = value.len();
            Self { value, cursor }
        }

        /// Insert text at the cursor.
        pub fn type_text(&mut self, text: &str) {
            let chars: Vec<char> = text.chars().collect();
            self.value
                .splice(self.cursor..self.cursor, chars.iter().cloned());
            self.cursor += chars.len();
        }

        /// Delete the character before the cursor (`false` is a beep: the
        /// cursor is already at the start).
        pub fn backspace(&mut self) -> bool {
            if self.cursor == 0 {
                return false;
            }
            self.cursor -= 1;
            self.value.remove(self.cursor);
            true
        }

        /// Clear the whole buffer (`ctrl-u`).
        pub fn clear(&mut self) {
            self.value.clear();
            self.cursor = 0;
        }

        /// Move the cursor to the start (`ctrl-a`).
        pub fn move_start(&mut self) {
            self.cursor = 0;
        }

        /// Move the cursor to the end (`ctrl-e`).
        pub fn move_end(&mut self) {
            self.cursor = self.value.len();
        }

        /// Move the cursor one character left (clamped at the start).
        pub fn move_left(&mut self) {
            self.cursor = self.cursor.saturating_sub(1);
        }

        /// Drop the per-field typed input (`Tab` advances to the next date
        /// field: the `:67`/`:80`/`:105` rows pin a fresh buffer).
        pub fn tab(&mut self) {
            self.clear();
        }

        /// Apply a `ctrl-<key>` press: `u` clears, `a`/`e` move, the rest
        /// (e.g. `l`) are ignored.
        pub fn apply_ctrl(&mut self, key: char) -> CtrlOutcome {
            match key {
                'u' => {
                    self.clear();
                    CtrlOutcome::Cleared
                }
                'a' => {
                    self.move_start();
                    CtrlOutcome::MovedToStart
                }
                'e' => {
                    self.move_end();
                    CtrlOutcome::MovedToEnd
                }
                _ => CtrlOutcome::Ignored,
            }
        }

        /// The submitted value (an emptied default submits `""`: `:382`).
        pub fn submit(&self) -> String {
            self.value.iter().collect()
        }
    }

    /// One integer keystroke (mirrors `defaultIntProcessor`): a lone `-`
    /// starts a negative; otherwise the concatenation must be an integer or
    /// the key is a beep (`None`).
    pub fn int_append(value: &str, input: &str) -> Option<String> {
        let combined = format!("{value}{input}");
        if combined == "-" {
            return Some(combined);
        }
        let digits = combined.strip_prefix('-').unwrap_or(&combined);
        if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
            return combined
                .parse::<i64>()
                .ok()
                .map(|parsed| parsed.to_string());
        }
        None
    }

    /// One float keystroke (mirrors `defaultFloatProcessor`): a second `.`
    /// is a beep; a leading `-` starts a negative; a trailing `.` or a digit
    /// after a `.` keeps the typed shape (the `:169` row pins `"0.05"`).
    /// Seam: the pin's `parseFloat` accepts a trailing `e` (`"12e"` parses
    /// as `12`); this port beeps on it instead.
    pub fn float_append(value: &str, input: &str) -> Option<String> {
        if input == "." && value.contains('.') {
            return None;
        }
        if value.is_empty() && input == "-" {
            return Some("-".to_owned());
        }
        let combined = format!("{value}{input}");
        if combined.parse::<f64>().is_err() {
            return None;
        }
        if input == "." || (value.contains('.') && input.bytes().all(|b| b.is_ascii_digit())) {
            return Some(combined);
        }
        // `Display` prints integral floats without a fraction (`125.0`
        // renders as `"125"`), matching the pin's `${parsed}`.
        combined
            .parse::<f64>()
            .ok()
            .map(|parsed| format!("{parsed}"))
    }

    /// Case-insensitive substring filter over titles/filenames, returning
    /// matching indices (mirrors `filterAutoCompleteChoices` on titles and
    /// `filterFiles` on names; an empty query matches everything).
    pub fn filter_matches(items: &[&str], query: &str) -> Vec<usize> {
        if query.is_empty() {
            return (0..items.len()).collect();
        }
        let query = query.to_lowercase();
        items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.to_lowercase().contains(&*query))
            .map(|(index, _)| index)
            .collect()
    }

    /// Move a choice cursor by `delta`, wrapping at the ends (mirrors the
    /// pin's modulo down-step and wrap-to-last up-step; `ctrl-n`/`ctrl-p`/
    /// `ctrl-k` all route here). An empty list stays at 0.
    pub fn step_index(index: usize, len: usize, delta: i64) -> usize {
        if len == 0 {
            return 0;
        }
        ((index as i64 + delta).rem_euclid(len as i64)) as usize
    }

    /// Submit the highlighted choice: an empty filtered list or a disabled
    /// highlight is a beep (`None`, the `:702`/`:729`/`:750` rows); the
    /// empty-list case also covers the interrupt-on-no-choices row at the
    /// data level (no submit is possible either way).
    pub fn submit_choice(disabled: &[bool], filtered: &[usize], index: usize) -> Option<usize> {
        let current = *filtered.get(index).or_else(|| filtered.first())?;
        if disabled.get(current).copied().unwrap_or(false) {
            return None;
        }
        Some(current)
    }

    /// Validate a multi-select submission count (mirrors the pin's
    /// `At least {min} are required` / `At most {max} choices are allowed`).
    pub fn multiselect_validate(
        selected: usize,
        min: Option<usize>,
        max: Option<usize>,
    ) -> Option<String> {
        if let Some(min) = min {
            if selected < min {
                return Some(format!("At least {min} are required"));
            }
        }
        if let Some(max) = max {
            if selected > max {
                return Some(format!("At most {max} choices are allowed"));
            }
        }
        None
    }

    /// Indices the Select-All meta option would hold: every enabled choice
    /// (the `:1155` row pins the disabled choice staying out).
    pub fn select_all(count: usize, disabled: &[bool]) -> Vec<usize> {
        (0..count)
            .filter(|index| !disabled.get(*index).copied().unwrap_or(false))
            .collect()
    }

    /// Indices the Invert-Selection meta option would hold: enabled choices
    /// flip, disabled choices stay out (the `:1174` row pins this).
    pub fn invert_selection(selected: &[usize], count: usize, disabled: &[bool]) -> Vec<usize> {
        (0..count)
            .filter(|index| {
                let is_disabled = disabled.get(*index).copied().unwrap_or(false);
                if is_disabled {
                    return false;
                }
                let is_selected = selected.contains(index);
                !is_selected
            })
            .collect()
    }

    /// Label of the Select-All meta option for the current state.
    pub fn select_all_label(all_selected: bool) -> &'static str {
        if all_selected {
            "Select None"
        } else {
            "Select All"
        }
    }

    /// Mask a value for password display, one mask per character.
    pub fn mask_text(value: &str, mask: &str) -> String {
        mask.repeat(value.chars().count())
    }

    /// Cursor offset for a masked password line (mirrors the pin's
    /// `(cursor - value.length) * mask.length`): one left step moves the
    /// cursor back by one whole mask (the `:543` row pins `-2` for a
    /// two-character mask).
    pub fn masked_cursor_offset(cursor: usize, value_len: usize, mask_len: usize) -> i64 {
        (cursor as i64 - value_len as i64) * mask_len as i64
    }

    /// Paging-arrow prefix padded to the widest arrow (mirrors
    /// `renderPagingPrefix`): the `:505` row pins `"   "` aligning with
    /// `"vvv"`. Widths count characters here, UTF-16 units there.
    pub fn paging_prefix(
        arrow_up: &str,
        arrow_down: &str,
        show_up: bool,
        show_down: bool,
    ) -> String {
        let width = arrow_up.chars().count().max(arrow_down.chars().count());
        let pad = |arrow: &str| {
            let missing = width.saturating_sub(arrow.chars().count());
            format!("{arrow}{}", " ".repeat(missing))
        };
        if show_up {
            pad(arrow_up)
        } else if show_down {
            pad(arrow_down)
        } else {
            " ".repeat(width)
        }
    }

    /// Checkbox symbol for a choice's selected state.
    pub fn checkbox_for(selected: bool, theme: &PromptTheme) -> &str {
        if selected {
            &theme.checkbox_on
        } else {
            &theme.checkbox_off
        }
    }

    /// Checkbox cell padded so titles align (mirrors the pin's
    /// `checkbox + " ".repeat(width - len + 1)`): the `:1114` row pins
    /// `"[x] "` aligning with `"    "`. Widths count characters here,
    /// UTF-16 units there.
    pub fn checkbox_cell(checkbox: &str, width: usize) -> String {
        let missing = width.saturating_sub(checkbox.chars().count()) + 1;
        format!("{checkbox}{}", " ".repeat(missing))
    }
}
