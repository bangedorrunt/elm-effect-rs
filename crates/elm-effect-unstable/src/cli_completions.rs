//! `cli/Completions` port, descriptor half.
//!
//! The pin splits this work three ways: the param/command model, the internal
//! descriptor that flattens a command tree into plain data
//! (`cli/internal/completions/descriptor.ts`), and the shell script generators
//! (`cli/internal/completions/{bash,zsh,fish}.ts`). This module is the
//! descriptor: a [`DescriptorCommand`] tree goes in, a [`CommandDescriptor`]
//! comes out. The generators are the separate `Completions.generate` surface
//! and are not built yet.
//!
//! Only the types that change a completion shape survive the trip: `String` is
//! the fallback for everything the descriptor does not distinguish, and the
//! three file readers all read a file.

use crate::cli::primitive::PathKind;

/// The value shape a flag advertises (`Completions.FlagType`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlagType {
    /// A switch, completed as `--flag` and `--no-flag`.
    Boolean,
    /// Free text.
    String,
    /// An integer.
    Int,
    /// A finite number.
    Finite,
    /// A date.
    Date,
    /// One of a fixed set.
    Choice(Vec<String>),
    /// A filesystem path.
    Path(PathKind),
}

/// The value shape an argument advertises (`Completions.ArgumentType`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArgumentType {
    /// Free text.
    String,
    /// An integer.
    Int,
    /// A finite number.
    Finite,
    /// A date.
    Date,
    /// One of a fixed set.
    Choice(Vec<String>),
    /// A filesystem path.
    Path(PathKind),
}

/// One flag, ready for completion generation (`Completions.FlagDescriptor`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlagDescriptor {
    /// Long name, no dashes.
    pub name: String,
    /// Aliases, no dashes.
    pub aliases: Vec<String>,
    /// Help text, when the flag has one.
    pub description: Option<String>,
    /// The value shape.
    pub flag_type: FlagType,
}

/// One positional, ready for completion generation
/// (`Completions.ArgumentDescriptor`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArgumentDescriptor {
    /// Name.
    pub name: String,
    /// Help text, when the argument has one.
    pub description: Option<String>,
    /// The argument must be supplied.
    pub required: bool,
    /// The argument consumes the remaining positionals.
    pub variadic: bool,
    /// The value shape.
    pub argument_type: ArgumentType,
}

/// A command tree, flattened (`Completions.CommandDescriptor`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandDescriptor {
    /// Command name.
    pub name: String,
    /// Short description when set, else the description.
    pub description: Option<String>,
    /// Flags, local first then inherited.
    pub flags: Vec<FlagDescriptor>,
    /// Positionals in declaration order.
    pub arguments: Vec<ArgumentDescriptor>,
    /// Reachable subcommands, alias entries included.
    pub subcommands: Vec<CommandDescriptor>,
}

/// The primitive a flag or argument carries. The descriptor reads only the
/// tag, so `FileText`, `FileParse` and `FileSchema` stay distinct here and
/// collapse to a file path on the way out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParamType {
    /// Free text.
    String,
    /// A switch.
    Boolean,
    /// An integer.
    Int,
    /// A finite number.
    Finite,
    /// A date.
    Date,
    /// One of a fixed set.
    Choice(Vec<String>),
    /// A filesystem path.
    Path(PathKind),
    /// Read a file as text.
    FileText,
    /// Read a file and parse it.
    FileParse,
    /// Read a file and decode it.
    FileSchema,
}

/// A flag or argument as the descriptor sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParamDef {
    /// Bound name.
    pub name: String,
    /// Aliases (`withAlias`).
    pub aliases: Vec<String>,
    /// Help text (`withDescription`).
    pub description: Option<String>,
    /// The value shape.
    pub param_type: ParamType,
    /// Absence is allowed (`optional`); arguments only, flags are never
    /// required in the descriptor.
    pub optional: bool,
    /// Consumes the rest (`variadic`); arguments only.
    pub variadic: bool,
    /// Omitted from completions (`withHidden`); flags only.
    pub hidden: bool,
    /// A display name swap (`withMetavar`). It changes help, never the shape.
    pub metavar: Option<String>,
}

impl ParamDef {
    /// A flag of `param_type` (`Flag.Int`, `Flag.Path`, ...).
    pub fn flag(name: &str, param_type: ParamType) -> Self {
        Self {
            name: name.to_owned(),
            aliases: Vec::new(),
            description: None,
            param_type,
            optional: false,
            variadic: false,
            hidden: false,
            metavar: None,
        }
    }

    /// A positional of `param_type` (`Argument.String`, `Argument.File`, ...).
    pub fn argument(name: &str, param_type: ParamType) -> Self {
        Self::flag(name, param_type)
    }

    /// Add an alias (`withAlias`).
    pub fn with_alias(mut self, alias: &str) -> Self {
        self.aliases.push(alias.to_owned());
        self
    }

    /// Attach help text (`withDescription`).
    pub fn with_description(mut self, text: &str) -> Self {
        self.description = Some(text.to_owned());
        self
    }

    /// Swap the displayed name (`withMetavar`).
    pub fn with_metavar(mut self, text: &str) -> Self {
        self.metavar = Some(text.to_owned());
        self
    }

    /// Hide from completions (`withHidden`).
    pub fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    /// Allow absence (`optional`).
    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }

    /// Consume the rest (`variadic`).
    pub fn variadic(mut self) -> Self {
        self.variadic = true;
        self
    }
}

/// A command node (`Command.make` plus its combinators).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DescriptorCommand {
    /// Command name.
    pub name: String,
    /// Long description.
    pub description: Option<String>,
    /// One-line description; wins over `description` in parent listings.
    pub short_description: Option<String>,
    /// Flags declared here.
    pub flags: Vec<ParamDef>,
    /// Positionals declared here.
    pub arguments: Vec<ParamDef>,
    /// Child commands.
    pub subcommands: Vec<DescriptorCommand>,
    /// Absent from help and completions (`unlisted`).
    pub unlisted: bool,
    /// Second name the command answers to (`withAlias`).
    pub alias: Option<String>,
    /// Flags a parent shares with this command (`context flags`).
    pub context_flags: Vec<ParamDef>,
}

impl DescriptorCommand {
    /// A command with no flags or arguments (`Command.make`).
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            ..Self::default()
        }
    }

    /// Set the long description (`withDescription`).
    pub fn with_description(mut self, text: &str) -> Self {
        self.description = Some(text.to_owned());
        self
    }

    /// Set the one-line description (`withShortDescription`).
    pub fn with_short_description(mut self, text: &str) -> Self {
        self.short_description = Some(text.to_owned());
        self
    }

    /// Add flags (`Command.make`'s second argument).
    pub fn with_flags(mut self, flags: Vec<ParamDef>) -> Self {
        self.flags.extend(flags);
        self
    }

    /// Add positionals.
    pub fn with_arguments(mut self, arguments: Vec<ParamDef>) -> Self {
        self.arguments.extend(arguments);
        self
    }

    /// Add children (`withSubcommands`).
    pub fn with_subcommands(mut self, subcommands: Vec<DescriptorCommand>) -> Self {
        self.subcommands.extend(subcommands);
        self
    }

    /// Keep the command out of help and completions (`withUnlisted`).
    pub fn unlisted(mut self) -> Self {
        self.unlisted = true;
        self
    }

    /// Add a second name (`withAlias`).
    pub fn with_alias(mut self, alias: &str) -> Self {
        self.alias = Some(alias.to_owned());
        self
    }

    /// Flags this command shares with its children (`withSharedFlags`).
    pub fn with_context_flags(mut self, flags: Vec<ParamDef>) -> Self {
        self.context_flags.extend(flags);
        self
    }
}

fn to_flag_type(param_type: &ParamType) -> FlagType {
    match param_type {
        ParamType::Boolean => FlagType::Boolean,
        ParamType::Int => FlagType::Int,
        ParamType::Finite => FlagType::Finite,
        ParamType::Date => FlagType::Date,
        ParamType::Choice(values) => FlagType::Choice(values.clone()),
        ParamType::Path(kind) => FlagType::Path(*kind),
        ParamType::FileText | ParamType::FileParse | ParamType::FileSchema => {
            FlagType::Path(PathKind::File)
        }
        ParamType::String => FlagType::String,
    }
}

fn to_argument_type(param_type: &ParamType) -> ArgumentType {
    match param_type {
        ParamType::Int => ArgumentType::Int,
        ParamType::Finite => ArgumentType::Finite,
        ParamType::Date => ArgumentType::Date,
        ParamType::Choice(values) => ArgumentType::Choice(values.clone()),
        ParamType::Path(kind) => ArgumentType::Path(*kind),
        ParamType::FileText | ParamType::FileParse | ParamType::FileSchema => {
            ArgumentType::Path(PathKind::File)
        }
        ParamType::String | ParamType::Boolean => ArgumentType::String,
    }
}

/// Flatten a command tree for completion generation (`fromCommand`).
///
/// Shape follows help: local flags first, then inherited ones root to leaf,
/// with an inherited name dropped when a local flag already claimed it.
/// Hidden flags and unlisted subcommands never reach the descriptor, so the
/// shell does not advertise what `--help` omits.
pub fn from_command(
    command: &DescriptorCommand,
    inherited_flags: &[ParamDef],
) -> CommandDescriptor {
    let mut flags = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    let local = command.flags.len();
    for (index, flag) in command.flags.iter().chain(inherited_flags).enumerate() {
        if flag.hidden {
            continue;
        }
        let inherited = index >= local;
        if inherited && seen.contains(&flag.name.as_str()) {
            continue;
        }
        seen.push(&flag.name);
        flags.push(FlagDescriptor {
            name: flag.name.clone(),
            aliases: flag.aliases.clone(),
            description: flag.description.clone(),
            flag_type: to_flag_type(&flag.param_type),
        });
    }

    let arguments = command
        .arguments
        .iter()
        .map(|argument| ArgumentDescriptor {
            name: argument.name.clone(),
            description: argument.description.clone(),
            required: !argument.optional,
            variadic: argument.variadic,
            argument_type: to_argument_type(&argument.param_type),
        })
        .collect();

    let shared: Vec<ParamDef> = inherited_flags
        .iter()
        .chain(command.context_flags.iter())
        .cloned()
        .collect();
    let mut subcommands = Vec::new();
    for child in &command.subcommands {
        if child.unlisted {
            continue;
        }
        let descriptor = from_command(child, &shared);
        subcommands.push(descriptor.clone());
        if let Some(alias) = &child.alias {
            if alias != &child.name {
                subcommands.push(CommandDescriptor {
                    name: alias.clone(),
                    ..descriptor
                });
            }
        }
    }

    CommandDescriptor {
        name: command.name.clone(),
        description: command
            .short_description
            .clone()
            .or_else(|| command.description.clone()),
        flags,
        arguments,
        subcommands,
    }
}
