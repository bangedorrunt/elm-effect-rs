//! CLI + AI LanguageModel port (ADR-0002 Phase 12).
//!
//! [`Cli`] parses `argv` into [`Command`] selections with typed
//! [`Arg`] values (`--flag`, `--opt value`, positionals, `--` rest),
//! renders `--help` text, and reports [`CliError`] without I/O, so tests
//! never spawn processes. [`LanguageModel`] is the provider trait
//! (`generate_text`, `generate_object`, `stream_text` over
//! [`ChatMessage`] parts); [`FakeModel`] replays scripted responses for
//! tests. Real providers (network, clap derive) live outside this crate
//! on purpose: the trait is the port, transports are not.

use std::collections::BTreeMap;

/// CLI parse failure.
#[derive(Clone, Debug, PartialEq)]
pub enum CliError {
    /// Unknown flag or command.
    Unknown(String),
    /// Missing required value.
    Missing(String),
    /// Value failed to parse.
    Invalid {
        /// Argument name.
        arg: String,
        /// Raw text.
        got: String,
    },
    /// Help was requested (carries the text).
    Help(String),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(a) => write!(f, "unknown argument: {a}"),
            Self::Missing(a) => write!(f, "missing required argument: {a}"),
            Self::Invalid { arg, got } => write!(f, "invalid value for {arg}: {got:?}"),
            Self::Help(t) => write!(f, "{t}"),
        }
    }
}

impl std::error::Error for CliError {}

/// One argument descriptor.
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    /// Boolean switch (`--verbose`, optional `--no-` negation).
    Flag {
        /// Long name.
        name: String,
        /// Help text.
        help: String,
    },
    /// Valued option (`--port 8080`).
    Opt {
        /// Long name.
        name: String,
        /// Help text.
        help: String,
        /// Required when true.
        required: bool,
    },
    /// Positional operand.
    Positional {
        /// Display name.
        name: String,
        /// Help text.
        help: String,
    },
}

/// Parsed argument values.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Parsed {
    /// Flags present.
    pub flags: BTreeMap<String, bool>,
    /// Option values, last one per name.
    pub opts: BTreeMap<String, String>,
    /// Every value an option was given, in the order they appeared. A
    /// repeated flag has several; an ordinary one has the single value.
    pub opts_runs: BTreeMap<String, Vec<String>>,
    /// Positional values.
    pub positionals: Vec<String>,
    /// Everything after `--`.
    pub rest: Vec<String>,
}

impl Parsed {
    /// Flag value (absent means false).
    pub fn flag(&self, name: &str) -> bool {
        self.flags.get(name).copied().unwrap_or(false)
    }

    /// Option value.
    pub fn opt(&self, name: &str) -> Option<&str> {
        self.opts.get(name).map(String::as_str)
    }

    /// The `index`-th value of an option that was given more than once. The
    /// single-value map keeps the last one for a caller that wants one value;
    /// this reads the whole run, which is what a repeated flag needs.
    pub fn opt_at(&self, name: &str, index: usize) -> Option<&str> {
        self.opts_runs
            .get(name)
            .and_then(|values| values.get(index))
            .map(String::as_str)
    }
}

/// One command node: name, args, subcommands.
#[derive(Clone, Debug)]
pub struct Command {
    /// Command name.
    pub name: String,
    /// Help text.
    pub help: String,
    /// Arguments.
    pub args: Vec<Arg>,
    /// Subcommands.
    pub subcommands: Vec<Command>,
    /// Hidden from help listings (`Command.unlisted`). It belongs to the
    /// command, so adding subcommands or shared flags does not clear it.
    pub unlisted: bool,
}

impl Command {
    /// Leaf command.
    pub fn new(name: &str, help: &str, args: Vec<Arg>) -> Self {
        Self {
            name: name.to_string(),
            help: help.to_string(),
            args,
            subcommands: Vec::new(),
            unlisted: false,
        }
    }

    /// Command with subcommands.
    pub fn group(name: &str, help: &str, subcommands: Vec<Command>) -> Self {
        Self {
            name: name.to_string(),
            help: help.to_string(),
            args: Vec::new(),
            subcommands,
            unlisted: false,
        }
    }

    /// Parse argv (without the binary name): subcommand dispatch first,
    /// then flags/options/positionals. `--help` anywhere returns the
    /// help text as [`CliError::Help`].
    pub fn parse(&self, argv: &[String]) -> Result<(Vec<String>, Parsed), CliError> {
        if argv.iter().any(|a| a == "--help" || a == "-h") {
            return Err(CliError::Help(self.help_text()));
        }
        if let Some((first, rest)) = argv.split_first() {
            if let Some(sub) = self.subcommands.iter().find(|c| &c.name == first) {
                let (mut path, parsed) = sub.parse(rest)?;
                path.insert(0, self.name.clone());
                return Ok((path, parsed));
            }
            if !self.subcommands.is_empty() && !first.starts_with('-') {
                return Err(CliError::Unknown(first.clone()));
            }
        }
        Ok((vec![self.name.clone()], self.parse_args(argv)?))
    }

    /// Render help text.
    pub fn help_text(&self) -> String {
        let mut out = format!("{} - {}\n", self.name, self.help);
        if !self.args.is_empty() {
            out.push_str("Arguments:\n");
            for arg in &self.args {
                match arg {
                    Arg::Flag { name, help } => out.push_str(&format!("  --{name}\n      {help}\n")),
                    Arg::Opt { name, help, required } => out.push_str(&format!(
                        "  --{name} <value>{}\n      {help}\n",
                        if *required { " (required)" } else { "" }
                    )),
                    Arg::Positional { name, help } => {
                        out.push_str(&format!("  <{name}>\n      {help}\n"))
                    }
                }
            }
        }
        if !self.subcommands.is_empty() {
            out.push_str("Commands:\n");
            for sub in &self.subcommands {
                out.push_str(&format!("  {}\n      {}\n", sub.name, sub.help));
            }
        }
        out
    }

    fn parse_args(&self, argv: &[String]) -> Result<Parsed, CliError> {
        let mut parsed = Parsed::default();
        let mut positionals = self.args.iter().filter(|a| matches!(a, Arg::Positional { .. }));
        let mut i = 0;
        while i < argv.len() {
            let arg = &argv[i];
            if arg == "--" {
                parsed.rest.extend(argv[i + 1..].iter().cloned());
                break;
            }
            if arg == "--help" || arg == "-h" {
                return Err(CliError::Help(self.help_text()));
            }
            if let Some(name) = arg.strip_prefix("--no-") {
                if self.has_flag(name) {
                    parsed.flags.insert(name.to_string(), false);
                    i += 1;
                    continue;
                }
                return Err(CliError::Unknown(arg.clone()));
            }
            if let Some(rest) = arg.strip_prefix("--") {
                let (name, inline) = match rest.split_once('=') {
                    Some((n, v)) => (n, Some(v)),
                    None => (rest, None),
                };
                if self.has_flag(name) {
                    if inline.is_some() {
                        return Err(CliError::Invalid {
                            arg: name.to_string(),
                            got: inline.unwrap_or_default().to_string(),
                        });
                    }
                    parsed.flags.insert(name.to_string(), true);
                    i += 1;
                    continue;
                }
                if self.has_opt(name) {
                    let value = match inline {
                        Some(v) => v.to_string(),
                        None => {
                            i += 1;
                            argv.get(i).cloned().ok_or_else(|| CliError::Missing(name.to_string()))?
                        }
                    };
                    parsed
                        .opts_runs
                        .entry(name.to_string())
                        .or_default()
                        .push(value.clone());
                    parsed.opts.insert(name.to_string(), value);
                    i += 1;
                    continue;
                }
                return Err(CliError::Unknown(arg.clone()));
            }
            match positionals.next() {
                Some(Arg::Positional { .. }) => parsed.positionals.push(arg.clone()),
                _ => return Err(CliError::Unknown(arg.clone())),
            }
            i += 1;
        }
        for a in &self.args {
            if let Arg::Opt { name, required: true, .. } = a {
                if !parsed.opts.contains_key(name) {
                    return Err(CliError::Missing(name.clone()));
                }
            }
        }
        Ok(parsed)
    }

    fn has_flag(&self, name: &str) -> bool {
        self.args.iter().any(|a| matches!(a, Arg::Flag { name: n, .. } if n == name))
    }

    fn has_opt(&self, name: &str) -> bool {
        self.args.iter().any(|a| matches!(a, Arg::Opt { name: n, .. } if n == name))
    }
}

/// Chat message part.
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    /// Plain text.
    Text(String),
    /// Tool call request.
    ToolCall {
        /// Tool name.
        name: String,
        /// JSON arguments.
        arguments: String,
    },
    /// Tool result.
    ToolResult {
        /// Tool name.
        name: String,
        /// JSON result.
        result: String,
    },
}

/// One chat message.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatMessage {
    /// Role (`system`, `user`, `assistant`, `tool`).
    pub role: String,
    /// Content parts.
    pub parts: Vec<Part>,
}

impl ChatMessage {
    /// User message with one text part.
    pub fn user(text: &str) -> Self {
        Self { role: "user".to_string(), parts: vec![Part::Text(text.to_string())] }
    }

    /// Assistant message with one text part.
    pub fn assistant(text: &str) -> Self {
        Self { role: "assistant".to_string(), parts: vec![Part::Text(text.to_string())] }
    }

    /// Concatenated text parts.
    pub fn text(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| match p {
                Part::Text(t) => Some(t.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }
}

/// Model failure.
#[derive(Clone, Debug, PartialEq)]
pub enum ModelError {
    /// Provider refused or failed.
    Provider(String),
    /// Output did not parse.
    Parse(String),
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Provider(m) => write!(f, "provider error: {m}"),
            Self::Parse(m) => write!(f, "parse error: {m}"),
        }
    }
}

impl std::error::Error for ModelError {}

/// Language model provider trait.
pub trait LanguageModel: Send + Sync {
    /// Complete to text.
    fn generate_text(&self, messages: &[ChatMessage]) -> Result<String, ModelError>;
    /// Complete to an object (JSON text).
    fn generate_object(&self, messages: &[ChatMessage], schema: &str) -> Result<String, ModelError>;
    /// Stream text chunks in order.
    fn stream_text(&self, messages: &[ChatMessage]) -> Result<Vec<String>, ModelError> {
        Ok(vec![self.generate_text(messages)?])
    }
}

/// Scripted fake model for tests.
#[derive(Clone, Debug, Default)]
pub struct FakeModel {
    /// Queued text responses, in order.
    pub texts: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    /// Observed prompts.
    pub seen: std::sync::Arc<std::sync::Mutex<Vec<Vec<ChatMessage>>>>,
}

impl FakeModel {
    /// Fake answering `texts` in order (repeats the last).
    pub fn new(texts: Vec<String>) -> Self {
        Self {
            texts: std::sync::Arc::new(std::sync::Mutex::new(texts)),
            seen: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }
}

impl LanguageModel for FakeModel {
    fn generate_text(&self, messages: &[ChatMessage]) -> Result<String, ModelError> {
        self.seen.lock().unwrap().push(messages.to_vec());
        let mut texts = self.texts.lock().unwrap();
        if texts.len() > 1 {
            Ok(texts.remove(0))
        } else {
            texts.first().cloned().ok_or(ModelError::Provider("no scripted response".to_string()))
        }
    }

    fn generate_object(&self, messages: &[ChatMessage], _schema: &str) -> Result<String, ModelError> {
        self.generate_text(messages)
    }

    fn stream_text(&self, messages: &[ChatMessage]) -> Result<Vec<String>, ModelError> {
        Ok(vec![self.generate_text(messages)?])
    }
}

/// One flag's contribution to an argv list.
///
/// A value that begins with a dash is emitted INLINE (`--value=-v`) because a
/// separate one would be read as a flag or an option-looking operand rather
/// than as this flag's value. A bare `-` is not one of those, so it travels on
/// its own, as do an empty value and a plain word.
pub fn flag_arg(name: &str, value: &str) -> Vec<String> {
    if value.starts_with('-') && value.len() > 1 {
        vec![format!("--{name}={value}")]
    } else {
        vec!["--".to_string() + name, value.to_string()]
    }
}

/// The argv a wizard produces for a command's named choices.
///
/// `steps` walks the command's arguments in the order they were declared:
/// `Flag(name)` asks for one value of that flag, and `Positional(name)` asks
/// for one value of that positional. Each answer becomes the argv the flag-arg
/// rule spells for it, and the command's own name leads the list.
pub fn wizard_argv(command: &Command, steps: &[WizardStep]) -> Vec<String> {
    let mut argv = vec![command.name.clone()];
    for step in steps {
        match step {
            WizardStep::Flag { name, value } => argv.extend(flag_arg(name, value)),
            WizardStep::Positional { value, .. } => argv.push(value.clone()),
        }
    }
    argv
}

/// One question the wizard asks, and the answer it was given.
#[derive(Clone, Debug, PartialEq)]
pub enum WizardStep {
    /// A flag value, named by the flag it answers.
    Flag {
        /// Flag name.
        name: String,
        /// The value that was entered.
        value: String,
    },
    /// A positional value.
    Positional {
        /// Display name.
        name: String,
        /// The value that was entered.
        value: String,
    },
}

impl WizardStep {
    /// Record the answer this step was given.
    pub fn answer(&mut self, text: &str) {
        match self {
            Self::Flag { value, .. } | Self::Positional { value, .. } => {
                value.clear();
                value.push_str(text);
            }
        }
    }
}

/// The questions a command's wizard asks, in declaration order: one per valued
/// flag, then one per positional. A boolean switch is not a question, because
/// the wizard renders it rather than asking for it.
pub fn wizard_plan(command: &Command) -> Vec<WizardStep> {
    let mut steps = Vec::new();
    for arg in &command.args {
        match arg {
            Arg::Opt { name, .. } => steps.push(WizardStep::Flag {
                name: name.clone(),
                value: String::new(),
            }),
            Arg::Positional { name, .. } => steps.push(WizardStep::Positional {
                name: name.clone(),
                value: String::new(),
            }),
            Arg::Flag { .. } => {}
        }
    }
    steps
}

/// Merge the repeated values of a key=value flag into one record.
///
/// Each value is `key=value` on its first `=`; a value with no `=` (or with an
/// empty side) is refused naming it, which is what makes a malformed pair a
/// failure rather than a silently skipped entry. Later values win on a
/// repeated key, and the record comes back in key order.
pub fn merge_key_values(values: &[String]) -> Result<BTreeMap<String, String>, String> {
    let mut record = BTreeMap::new();
    for value in values {
        let (key, item) = crate::cli::primitive::parse_key_value(value)?;
        record.insert(key, item);
    }
    Ok(record)
}

/// The values of a repeated option, in the order they were given.
pub fn option_runs(parsed: &Parsed, name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut index = 0;
    while let Some(value) = parsed.opt_at(name, index) {
        out.push(value.to_string());
        index += 1;
    }
    out
}

impl Command {
    /// Hide this command from help listings (`Command.unlisted`).
    pub fn unlisted(mut self) -> Self {
        self.unlisted = true;
        self
    }

    /// Attach subcommands, keeping the command's own metadata.
    pub fn with_subcommands(mut self, subcommands: Vec<Command>) -> Self {
        self.subcommands.extend(subcommands);
        self
    }

    /// Add shared flags, keeping the command's own metadata.
    pub fn with_shared_flags(mut self, args: Vec<Arg>) -> Self {
        self.args.extend(args);
        self
    }
}
