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
        /// Takes every value it is given, in order, rather than exactly one.
        variadic: bool,
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
    /// Fold a selected node's own values over the globals collected ahead of
    /// it. The node's entry wins on a shared name (`local overrides
    /// global`); a global the node never re-states survives untouched.
    fn absorb(&mut self, node: Parsed) {
        for (name, value) in node.flags {
            self.flags.insert(name, value);
        }
        for (name, value) in node.opts {
            self.opts.insert(name, value);
        }
        for (name, value) in node.opts_runs {
            self.opts_runs.insert(name, value);
        }
        if !node.positionals.is_empty() {
            self.positionals = node.positionals;
        }
        if !node.rest.is_empty() {
            self.rest = node.rest;
        }
    }

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
    /// Arguments this node declares for itself.
    pub args: Vec<Arg>,
    /// Args shared with every subcommand (`withSharedFlags`). Kept apart
    /// from `args` because a shared arg is in scope on both sides of a
    /// subcommand while a local one is not.
    shared_args: Vec<Arg>,
    /// Subcommands.
    pub subcommands: Vec<Command>,
    /// Hidden from help listings (`Command.unlisted`). It belongs to the
    /// command, so adding subcommands or shared flags does not clear it.
    pub unlisted: bool,
    /// Other names that select this command (`Command.withAlias`). The first
    /// name stays the one a path records.
    pub aliases: Vec<String>,
    /// The labels the subcommand groups were declared under, in declaration
    /// order. A label is help metadata: it is never a selector, so a command in
    /// a group is reached by its own name exactly as an ungrouped one is.
    pub subcommand_groups: Vec<String>,
    /// The code that runs once this command's values are parsed. A command
    /// without one is a group: selecting it hands the line to its children.
    handler: Option<Handler>,
}

impl Command {
    /// Leaf command.
    pub fn new(name: &str, help: &str, args: Vec<Arg>) -> Self {
        Self {
            name: name.to_string(),
            help: help.to_string(),
            args,
            shared_args: Vec::new(),
            subcommands: Vec::new(),
            unlisted: false,
            aliases: Vec::new(),
            subcommand_groups: Vec::new(),
            handler: None,
        }
    }

    /// Command with subcommands.
    pub fn group(name: &str, help: &str, subcommands: Vec<Command>) -> Self {
        Self {
            name: name.to_string(),
            help: help.to_string(),
            args: Vec::new(),
            shared_args: Vec::new(),
            subcommands,
            unlisted: false,
            aliases: Vec::new(),
            subcommand_groups: Vec::new(),
            handler: None,
        }
    }

    /// Parse argv (without the binary name): subcommand dispatch first,
    /// then flags/options/positionals. `--help` anywhere returns the
    /// help text as [`CliError::Help`].
    pub fn parse(&self, argv: &[String]) -> Result<(Vec<String>, Parsed), CliError> {
        self.parse_inherited(Vec::new(), argv)
    }

    /// Parse with the shared args this node inherits from its ancestors.
    ///
    /// A parent's shared flag is a flag on both sides of the subcommand it was
    /// written beside, so the chain carries down the path that was selected and
    /// the selected node accepts an ancestor's shared arg as declared.
    /// Positionals are never inherited: an operand belongs to the node that
    /// names it.
    fn parse_inherited(
        &self,
        inherited: Vec<&Arg>,
        argv: &[String],
    ) -> Result<(Vec<String>, Parsed), CliError> {
        if argv.iter().any(|a| a == "--help" || a == "-h") {
            return Err(CliError::Help(self.help_text()));
        }
        if let Some((first, rest)) = argv.split_first() {
            // A subcommand is selected by any of its names, so an alias
            // routes the same way the canonical name does.
            if let Some(sub) = self.subcommands.iter().find(|c| c.matches_name(first)) {
                let mut chain: Vec<&Arg> = inherited;
                chain.extend(self.shared_args.iter());
                let (mut path, parsed) = sub.parse_inherited(chain, rest)?;
                path.insert(0, self.name.clone());
                return Ok((path, parsed));
            }
            if !self.subcommands.is_empty() && !first.starts_with('-') {
                return Err(CliError::Unknown(first.clone()));
            }
        }
        if self.subcommands.is_empty() {
            return Ok((vec![self.name.clone()], self.parse_args(argv, &inherited, true)?));
        }
        // A node that carries subcommands and declares its own flags reads a
        // global flag ahead of the subcommand, and the value it collected
        // rides into the selected node's values.
        self.parse_globals_then_subcommand(argv, &inherited)
    }

    /// Whether this node declares a valued option under `name`. The leading
    /// scan needs it to know the next token is that option's value rather
    /// than the subcommand.
    fn has_valued_opt(&self, name: &str, declared: &[&Arg]) -> bool {
        declared
            .iter()
            .any(|a| matches!(a, Arg::Opt { name: n, .. } if n == name))
    }

    /// Read this node's own flags off the front of `argv`, then hand what is
    /// left to the subcommand they preceded.
    ///
    /// A global flag is read where it appears rather than only after the
    /// subcommand, so `--region us-east-1 deploy` selects `deploy` with the
    /// region already collected. The selected node's own values win on a
    /// shared name, which is the `local overrides global` rule.
    fn parse_globals_then_subcommand(
        &self,
        argv: &[String],
        inherited: &[&Arg],
    ) -> Result<(Vec<String>, Parsed), CliError> {
        // A parent's local flag is out of scope on the subcommand path: if a
        // line both names a subcommand and a local flag ahead of it, refuse the
        // flag rather than letting it fall through into `unknown argument`.
        // Only consider subcommands that occur before an explicit `--` separator,
        // since anything after the dash is operands and cannot select.
        let dash_idx = argv.iter().position(|t| t == "--").unwrap_or(argv.len());
        let sub_idx = (0..dash_idx).find(|i| self.subcommands.iter().any(|c| c.matches_name(&argv[*i])));
        if let Some(sub_idx) = sub_idx {
            for token in &argv[..sub_idx] {
                if !token.starts_with('-') {
                    continue;
                }
                let name = token.trim_start_matches("--");
                let name = name.split('=').next().unwrap_or(name);
                let is_local = self.args.iter().any(|a| {
                    matches!(a, Arg::Flag { name: n, .. } | Arg::Opt { name: n, .. } if *n == name)
                });
                let is_shared_or_inherited = self
                    .shared_args
                    .iter()
                    .chain(inherited.iter().map(|a| *a))
                    .any(|a| {
                        matches!(a, Arg::Flag { name: n, .. } | Arg::Opt { name: n, .. } if *n == name)
                    });
                if is_local && !is_shared_or_inherited {
                    return Err(CliError::Unknown(token.clone()));
                }
            }
        }
        let mut leading: Vec<String> = Vec::new();
        let mut i = 0;
        while i < argv.len() {
            let token = &argv[i];
            // Everything after the dash is operands, so no subcommand is read
            // out of them. `parse_args` owns that rule already.
            if token == "--" {
                break;
            }
            if let Some(sub) = self.subcommands.iter().find(|c| c.matches_name(token)) {
                let mut chain: Vec<&Arg> = inherited.to_vec();
                chain.extend(self.shared_args.iter());
                // A parent's local flag cannot ride on the subcommand path:
                // if one was given before the subcommand, refuse it by name.
                for token in &leading {
                    let name = token
                        .trim_start_matches("--")
                        .split('=')
                        .next()
                        .unwrap_or("")
                        .to_string();
                    let is_local = self
                        .args
                        .iter()
                        .any(|a| matches!(a, Arg::Flag { name: n, .. } | Arg::Opt { name: n, .. } if *n == name));
                    let shared_or_inherited = self
                        .shared_args
                        .iter()
                        .chain(inherited.iter().map(|a| *a))
                        .any(|a| matches!(a, Arg::Flag { name: n, .. } | Arg::Opt { name: n, .. } if *n == name));
                    if is_local && !shared_or_inherited {
                        return Err(CliError::Unknown(token.clone()));
                    }
                }
                let (mut path, parsed) = sub.parse_inherited(chain, &argv[i + 1..])?;
                path.insert(0, self.name.clone());
                let mut head_scope: Vec<&Arg> = self.shared_args.iter().collect();
                head_scope.extend(inherited.iter().copied());
                let mut head = self.parse_args(&leading, &head_scope, false)?;
                head.absorb(parsed);
                return Ok((path, head));
            }
            // A token that is neither a flag nor this node's subcommand ends
            // the scan, and `parse_args` reports it: an unknown flag is named
            // as itself rather than blamed on the word that follows it.
            if !token.starts_with('-') {
                break;
            }
            leading.push(token.clone());
            let name = token.strip_prefix("--").unwrap_or(token);
            let (_, inline) = match name.split_once('=') {
                Some((n, v)) => (n, Some(v)),
                None => (name, None),
            };
            let mut scan_scope: Vec<&Arg> = self.shared_args.iter().collect();
            scan_scope.extend(inherited.iter().copied());
            if inline.is_none() && self.has_valued_opt(name, &scan_scope) && i + 1 < argv.len() {
                i += 1;
                leading.push(argv[i].clone());
            }
            i += 1;
        }
        // No subcommand was selected, so the whole line is this node's own.
        if self.subcommands.is_empty() {
            // own + shared declared at this node, plus anything inherited
            let mut declared = self.args.iter().chain(self.shared_args.iter())
                .collect::<Vec<&Arg>>();
            declared.extend(inherited.iter().copied());
            return Ok((vec![self.name.clone()], self.parse_args(argv, &declared, true)?));
        }
        let mut declared: Vec<&Arg> = self.args.iter().chain(self.shared_args.iter())
            .collect();
        declared.extend(inherited.iter().copied());
        Ok((vec![self.name.clone()], self.parse_args(argv, &declared, true)?))
    }

    /// Render help text.
    pub fn help_text(&self) -> String {
        let mut out = format!("{} - {}\n", self.name, self.help);
        if !self.args.is_empty() || !self.shared_args.is_empty() {
            out.push_str("Arguments:\n");
            for arg in self.args.iter().chain(self.shared_args.iter()) {
                match arg {
                    Arg::Flag { name, help } => out.push_str(&format!("  --{name}\n      {help}\n")),
                    Arg::Opt { name, help, required } => out.push_str(&format!(
                        "  --{name} <value>{}\n      {help}\n",
                        if *required { " (required)" } else { "" }
                    )),
                    Arg::Positional { name, help, variadic } => {
                        // A variadic operand names itself once in help, because
                        // it is one argument that takes many values.
                        let suffix = if *variadic { "..." } else { "" };
                        out.push_str(&format!("  <{name}>{suffix}\n      {help}\n"))
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

    fn parse_args(&self, argv: &[String], extra: &[&Arg], include_local: bool) -> Result<Parsed, CliError> {
        let mut declared: Vec<&Arg> = Vec::new();
        if include_local { declared.extend(self.args.iter()); }
        declared.extend(self.shared_args.iter());
        declared.extend(extra.iter().copied());
        let mut parsed = Parsed::default();
        let mut positionals = self.args.iter().filter(|a| matches!(a, Arg::Positional { .. }));
        let mut variadic = false;
        let mut i = 0;
        while i < argv.len() {
            let arg = &argv[i];
            if arg == "--" {
                let operands = &argv[i + 1..];
                // Tokens after the dash are operands: no subcommand and no
                // flag is read out of them. A variadic positional is the
                // argument that takes many values, so it takes these; a node
                // without one keeps them in `rest` for its caller.
                let variadic = self
                    .args
                    .iter()
                    .any(|a| matches!(a, Arg::Positional { variadic: true, .. }));
                if variadic {
                    parsed.positionals.extend(operands.iter().cloned());
                } else {
                    parsed.rest.extend(operands.iter().cloned());
                }
                break;
            }
            if arg == "--help" || arg == "-h" {
                return Err(CliError::Help(self.help_text()));
            }
            if let Some(negated) = arg.strip_prefix("--no-") {
                // The negated form is a switch, not a valued flag: a value
                // beside it reads as two things at once, so it is refused
                // rather than silently ignored.
                if let Some((_, value)) = negated.split_once('=') {
                    return Err(CliError::Invalid {
                        arg: arg.clone(),
                        got: value.to_string(),
                    });
                }
                if self.has_flag(negated, &declared) {
                    parsed.flags.insert(negated.to_string(), false);
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
                if self.has_flag(name, &declared) {
                    // An inline or following literal is the flag's value, so
                    // `--verbose false` is false rather than true-with-a-
                    // stray-word. A bare switch stays true.
                    let literal = inline.or_else(|| {
                        argv.get(i + 1).and_then(|next| {
                            crate::cli::primitive::parse_bool(next).ok().map(|_| next.as_str())
                        })
                    });
                    match literal {
                        Some(text) => match crate::cli::primitive::parse_bool(text) {
                            Ok(value) => {
                                parsed.flags.insert(name.to_string(), value);
                                // A following literal is consumed with the flag;
                                // an inline one already rode on it.
                                i += if inline.is_some() { 1 } else { 2 };
                                continue;
                            }
                            Err(_) => {
                                return Err(CliError::Invalid {
                                    arg: name.to_string(),
                                    got: text.to_string(),
                                })
                            }
                        },
                        None => {
                            parsed.flags.insert(name.to_string(), true);
                            i += 1;
                            continue;
                        }
                    }
                }
                if self.has_opt(name, &declared) {
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
            if variadic {
                parsed.positionals.push(arg.clone());
                i += 1;
                continue;
            }
            match positionals.next() {
                // A variadic positional keeps taking values, so the iterator
                // does not advance past it and the order across an interleaved
                // option is the order the line gave.
                Some(Arg::Positional { variadic: true, .. }) => {
                    parsed.positionals.push(arg.clone());
                    variadic = true;
                }
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

    /// Whether `name` is a flag declared in `declared` (already merged with
    /// this node's own locals/shared plus extras by the caller).
    fn has_flag(&self, name: &str, declared: &[&Arg]) -> bool {
        declared
            .iter()
            .any(|a| matches!(a, Arg::Flag { name: n, .. } if n == name))
    }

    /// Whether a valued option under `name` is declared in `declared`.
    fn has_opt(&self, name: &str, declared: &[&Arg]) -> bool {
        declared
            .iter()
            .any(|a| matches!(a, Arg::Opt { name: n, .. } if n == name))
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
        self.shared_args.extend(args);
        self
    }
}

/// A command handler: the code that runs once the command's values are parsed.
/// It is a boxed closure so a command tree stays clonable, and its `Debug`
/// names the type rather than trying to print a function.
#[derive(Clone)]
pub struct Handler(std::sync::Arc<dyn Fn(&Parsed) + Send + Sync>);

impl Handler {
    /// Run this handler over the parsed values.
    pub fn call(&self, parsed: &Parsed) {
        (self.0)(parsed)
    }
}

impl std::fmt::Debug for Handler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Handler")
    }
}

/// What a run selected and did: the path from the root, the values parsed at
/// the node it landed on, and whether that node's handler ran.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    /// Command path from the root, in the commands' own names.
    pub path: Vec<String>,
    /// The values parsed at the selected node.
    pub parsed: Parsed,
    /// True when the selected node has a handler and it ran.
    pub invoked: bool,
}

impl Command {
    /// Attach a handler (`Command.make`'s third argument).
    pub fn handler(mut self, handler: impl Fn(&Parsed) + Send + Sync + 'static) -> Self {
        self.handler = Some(Handler(std::sync::Arc::new(handler)));
        self
    }

    /// Another name that selects this command (`Command.withAlias`).
    pub fn alias(mut self, alias: &str) -> Self {
        self.aliases.push(alias.to_string());
        self
    }

    /// Run this command tree over argv and invoke the selected node's handler.
    ///
    /// The dispatch is the existing `parse`: subcommand first, then this
    /// node's own flags and positionals. A node without a handler is a group,
    /// so selecting it with no subcommand present is a refusal rather than a
    /// silently empty run.
    pub fn run(&self, argv: &[String]) -> Result<Run, CliError> {
        let (path, parsed) = self.parse(argv)?;
        let node = path
            .iter()
            .skip(1)
            .try_fold(self, |node, name| {
                node.subcommands
                    .iter()
                    .find(|sub| &sub.name == name || sub.aliases.iter().any(|a| a == name))
                    .ok_or_else(|| CliError::Unknown(name.clone()))
            })?;
        match &node.handler {
            Some(handler) => {
                handler.call(&parsed);
                Ok(Run { path, parsed, invoked: true })
            }
            // A group reached without a subcommand has nothing to run, which
            // is the same refusal an unknown subcommand gets.
            None => Err(CliError::Unknown(self.name.clone())),
        }
    }

    /// Whether this command is selected by `name`, its own or an alias.
    pub fn matches_name(&self, name: &str) -> bool {
        self.name == name || self.aliases.iter().any(|alias| alias == name)
    }
}

impl Command {
    /// Add subcommands that arrive in labelled groups (`withSubcommands`'s
    /// group form). Every command in a group is registered as a subcommand, so
    /// it is selected by its own name; the label rides along for help text and
    /// is never a selector.
    pub fn with_subcommand_groups(mut self, groups: Vec<(String, Vec<Command>)>) -> Self {
        for (label, commands) in groups {
            if !label.is_empty() {
                self.subcommand_groups.push(label);
            }
            self.subcommands.extend(commands);
        }
        self
    }
}
